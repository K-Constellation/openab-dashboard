use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{Connection, OpenFlags};
use tokio::process::Command;

use crate::config::{DevinSessionDbMode, PodConfig};
use crate::models::UsageRecord;

const DEFAULT_SESSION_DB_PATH: &str = "/home/node/.local/share/devin/cli/sessions.db";
const DEFAULT_CONTAINER: &str = "openab";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterEnvironment {
    Local,
    Cloud,
    Unknown,
}

impl ClusterEnvironment {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Cloud => "cloud",
            Self::Unknown => "unknown",
        }
    }
}

pub fn should_collect(mode: &DevinSessionDbMode, environment: ClusterEnvironment) -> bool {
    match mode {
        DevinSessionDbMode::Disabled => false,
        DevinSessionDbMode::Local => environment != ClusterEnvironment::Cloud,
        DevinSessionDbMode::Auto => environment == ClusterEnvironment::Local,
    }
}

pub async fn detect_cluster_environment(kubectl: &str) -> ClusterEnvironment {
    let (context, server, nodes) = tokio::join!(
        kubectl_output(kubectl, &["config", "current-context"]),
        kubectl_output(kubectl, &["config", "view", "--minify", "-o", "jsonpath={.clusters[0].cluster.server}"]),
        kubectl_output(kubectl, &["get", "nodes", "-o", "jsonpath={range .items[*]}{.spec.providerID}{\"|\"}{.metadata.labels}{\"\\n\"}{end}"]),
    );

    classify_cluster_environment(
        context.as_deref().unwrap_or_default(),
        server.as_deref().unwrap_or_default(),
        nodes.as_deref().unwrap_or_default(),
    )
}

fn classify_cluster_environment(context: &str, server: &str, nodes: &str) -> ClusterEnvironment {
    let fingerprint = format!("{context} {server} {nodes}").to_ascii_lowercase();
    let cloud_markers = [
        "eks.amazonaws.com",
        "aws://",
        ".eks.amazonaws.com",
        "gke.io",
        "gke_",
        "gce://",
        "aks",
        "azure://",
        "digitalocean",
        "linode",
    ];
    if cloud_markers
        .iter()
        .any(|marker| fingerprint.contains(marker))
    {
        return ClusterEnvironment::Cloud;
    }

    let local_markers = [
        "orbstack",
        "docker-desktop",
        "minikube",
        "kind-",
        "k3d-",
        "127.0.0.1",
        "localhost",
        "k3s://orbstack",
    ];
    if local_markers
        .iter()
        .any(|marker| fingerprint.contains(marker))
    {
        ClusterEnvironment::Local
    } else {
        ClusterEnvironment::Unknown
    }
}

pub async fn collect_pod(
    kubectl: &str,
    pod: &PodConfig,
    provider: &str,
) -> Result<Vec<UsageRecord>> {
    let source_path = pod
        .devin_session_db_path
        .as_deref()
        .unwrap_or(DEFAULT_SESSION_DB_PATH);
    let database = read_pod_file(kubectl, pod, source_path, true)
        .await?
        .expect("required Devin database must be present");
    let wal = read_pod_file(kubectl, pod, &format!("{source_path}-wal"), false).await?;
    let shm = read_pod_file(kubectl, pod, &format!("{source_path}-shm"), false).await?;

    let snapshot_dir = create_snapshot_dir()?;
    let snapshot_path = snapshot_dir.join("sessions.db");
    fs::write(&snapshot_path, database)?;
    if let Some(wal) = wal {
        fs::write(snapshot_dir.join("sessions.db-wal"), wal)?;
    }
    if let Some(shm) = shm {
        fs::write(snapshot_dir.join("sessions.db-shm"), shm)?;
    }

    let agent = pod.name.clone();
    let provider = provider.to_string();
    let parse_result = tokio::task::spawn_blocking(move || {
        parse_session_database(&snapshot_path, &agent, &provider)
    })
    .await?;
    let _ = fs::remove_dir_all(snapshot_dir);
    parse_result
}

async fn read_pod_file(
    kubectl: &str,
    pod: &PodConfig,
    path: &str,
    required: bool,
) -> Result<Option<Vec<u8>>> {
    let container = pod.container.as_deref().unwrap_or(DEFAULT_CONTAINER);
    let output = Command::new(kubectl)
        .args([
            "exec",
            &format!("deployment/{}", pod.deployment),
            "--container",
            container,
            "--",
            "cat",
            path,
        ])
        .output()
        .await
        .with_context(|| format!("failed to read {path} from Devin pod {}", pod.name))?;

    if output.status.success() {
        return Ok(Some(output.stdout));
    }

    if required {
        let error = String::from_utf8_lossy(&output.stderr);
        bail!(
            "could not read Devin session DB from {}: {}",
            pod.name,
            error.trim()
        );
    }
    Ok(None)
}

async fn kubectl_output(kubectl: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(kubectl).args(args).output().await.ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

fn create_snapshot_dir() -> Result<PathBuf> {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let path = std::env::temp_dir().join(format!(
        "openab-dashboard-devin-{}-{nonce}",
        std::process::id(),
    ));
    fs::create_dir(&path)?;
    Ok(path)
}

fn parse_session_database(path: &Path, agent: &str, provider: &str) -> Result<Vec<UsageRecord>> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| {
        format!(
            "could not open copied Devin session DB at {}",
            path.display()
        )
    })?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;

    let mut statement = conn.prepare(
        "WITH assistant_messages AS (
            SELECT
                mn.session_id,
                json_extract(mn.chat_message, '$.message_id') AS message_id,
                MAX(mn.created_at) AS node_created_at,
                MAX(json_extract(mn.chat_message, '$.metadata.created_at')) AS message_created_at,
                MAX(COALESCE(json_extract(mn.chat_message, '$.metadata.generation_model'), s.model)) AS model,
                MAX(json_extract(mn.chat_message, '$.metadata.request_id')) AS request_id,
                MAX(json_extract(mn.chat_message, '$.metadata.metrics.input_tokens')) AS input_tokens,
                MAX(json_extract(mn.chat_message, '$.metadata.metrics.output_tokens')) AS output_tokens,
                MAX(json_extract(mn.chat_message, '$.metadata.metrics.cache_read_tokens')) AS cache_read_tokens,
                MAX(json_extract(mn.chat_message, '$.metadata.metrics.cache_creation_tokens')) AS cache_creation_tokens,
                MAX(json_extract(mn.chat_message, '$.metadata.metrics.total_time_ms')) AS duration_ms
            FROM message_nodes mn
            LEFT JOIN sessions s ON s.id = mn.session_id
            WHERE json_extract(mn.chat_message, '$.role') = 'assistant'
              AND json_extract(mn.chat_message, '$.message_id') IS NOT NULL
              AND json_extract(mn.chat_message, '$.metadata.metrics.input_tokens') IS NOT NULL
            GROUP BY mn.session_id, json_extract(mn.chat_message, '$.message_id')
        )
        SELECT session_id, message_id, node_created_at, message_created_at, model, request_id,
               input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens, duration_ms
        FROM assistant_messages"
    )?;

    let rows = statement.query_map([], |row| {
        Ok(DevinMessage {
            session_id: row.get(0)?,
            message_id: row.get(1)?,
            node_created_at: row.get(2)?,
            message_created_at: row.get(3)?,
            model: row.get(4)?,
            request_id: row.get(5)?,
            input_tokens: row.get(6)?,
            output_tokens: row.get(7)?,
            cache_read_tokens: row.get(8)?,
            cache_creation_tokens: row.get(9)?,
            duration_ms: row.get(10)?,
        })
    })?;

    rows.map(|row| {
        let message = row?;
        let timestamp = message
            .message_created_at
            .as_deref()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc))
            .unwrap_or_else(|| {
                Utc.timestamp_opt(message.node_created_at, 0)
                    .single()
                    .unwrap_or_else(Utc::now)
            });
        let cached_tokens = message
            .cache_read_tokens
            .unwrap_or(0)
            .saturating_add(message.cache_creation_tokens.unwrap_or(0));
        let total_tokens = message
            .input_tokens
            .zip(message.output_tokens)
            .map(|(input, output)| input.saturating_add(output).saturating_add(cached_tokens));

        Ok(UsageRecord {
            timestamp,
            agent: agent.to_string(),
            provider: provider.to_string(),
            model: message.model,
            input_tokens: message.input_tokens,
            output_tokens: message.output_tokens,
            total_tokens,
            duration_ms: message.duration_ms,
            session_id: Some(message.session_id.clone()),
            provider_event_id: Some(format!("{}:{}", message.session_id, message.message_id)),
            metadata: Some(serde_json::json!({
                "source": "devin_session_db",
                "message_id": message.message_id,
                "request_id": message.request_id,
                "cache_read_tokens": message.cache_read_tokens,
                "cache_creation_tokens": message.cache_creation_tokens,
            })),
            ..Default::default()
        })
    })
    .collect()
}

struct DevinMessage {
    session_id: String,
    message_id: String,
    node_created_at: i64,
    message_created_at: Option<String>,
    model: Option<String>,
    request_id: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_read_tokens: Option<i64>,
    cache_creation_tokens: Option<i64>,
    duration_ms: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    #[test]
    fn classifies_known_local_and_cloud_clusters() {
        assert_eq!(
            classify_cluster_environment("orbstack", "https://127.0.0.1:26443", "k3s://orbstack"),
            ClusterEnvironment::Local,
        );
        assert_eq!(
            classify_cluster_environment(
                "production",
                "https://abc.eks.amazonaws.com",
                "aws://i-123"
            ),
            ClusterEnvironment::Cloud,
        );
        assert_eq!(
            classify_cluster_environment("staging", "https://cluster.internal", ""),
            ClusterEnvironment::Unknown,
        );
    }

    #[test]
    fn auto_mode_is_local_only() {
        assert!(should_collect(
            &DevinSessionDbMode::Auto,
            ClusterEnvironment::Local
        ));
        assert!(!should_collect(
            &DevinSessionDbMode::Auto,
            ClusterEnvironment::Cloud
        ));
        assert!(!should_collect(
            &DevinSessionDbMode::Auto,
            ClusterEnvironment::Unknown
        ));
    }

    #[test]
    fn parses_and_deduplicates_devin_assistant_messages() {
        let directory = create_snapshot_dir().unwrap();
        let database_path = directory.join("sessions.db");
        let conn = Connection::open(&database_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY, model TEXT NOT NULL);
             CREATE TABLE message_nodes (
                row_id INTEGER PRIMARY KEY,
                session_id TEXT NOT NULL,
                node_id INTEGER NOT NULL,
                parent_node_id INTEGER,
                chat_message TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                metadata TEXT
             );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, model) VALUES (?1, ?2)",
            params!["session-1", "swe-1-7-medium"],
        )
        .unwrap();

        let message = serde_json::json!({
            "message_id": "message-1",
            "role": "assistant",
            "metadata": {
                "created_at": "2026-09-07T01:02:03Z",
                "generation_model": "gpt-5-6-terra-high",
                "request_id": "request-1",
                "metrics": {
                    "input_tokens": 120,
                    "output_tokens": 24,
                    "cache_read_tokens": 800,
                    "cache_creation_tokens": 20,
                    "total_time_ms": 3210
                }
            }
        });
        conn.execute(
            "INSERT INTO message_nodes (session_id, node_id, chat_message, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params!["session-1", 1, message.to_string(), 1_700_000_000_i64],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_nodes (session_id, node_id, chat_message, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params!["session-1", 2, message.to_string(), 1_700_000_001_i64],
        )
        .unwrap();
        drop(conn);

        let records = parse_session_database(&database_path, "orion", "devin").unwrap();
        let _ = fs::remove_dir_all(directory);

        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.agent, "orion");
        assert_eq!(record.model.as_deref(), Some("gpt-5-6-terra-high"));
        assert_eq!(record.input_tokens, Some(120));
        assert_eq!(record.output_tokens, Some(24));
        assert_eq!(record.total_tokens, Some(964));
        assert_eq!(
            record.provider_event_id.as_deref(),
            Some("session-1:message-1")
        );
        assert_eq!(record.metadata.as_ref().unwrap()["cache_read_tokens"], 800);
    }
}
