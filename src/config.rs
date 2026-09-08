use serde::Deserialize;
use std::fs;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub database: DatabaseConfig,
    pub server: ServerConfig,
    pub collector: CollectorConfig,
    pub providers: ProvidersConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    pub path: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CollectorConfig {
    #[serde(default = "default_interval")]
    pub interval_seconds: u64,
    #[serde(default = "default_kubectl")]
    pub kubectl_path: String,
    #[serde(default)]
    pub devin_session_db_mode: DevinSessionDbMode,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DevinSessionDbMode {
    #[default]
    Auto,
    Local,
    Disabled,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProvidersConfig {
    #[serde(flatten)]
    pub providers: std::collections::HashMap<String, ProviderConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub pods: Vec<PodConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PodConfig {
    pub name: String,
    pub deployment: String,
    pub account_id: String,
    #[serde(default)]
    pub oauth_token_path: Option<String>,
    /// Optional container name for pod-local collectors such as Devin.
    #[serde(default)]
    pub container: Option<String>,
    /// Defaults to Devin CLI's standard state database location.
    #[serde(default)]
    pub devin_session_db_path: Option<String>,
}

fn default_host() -> String { "0.0.0.0".into() }
fn default_port() -> u16 { 8080 }
fn default_interval() -> u64 { 300 }
fn default_kubectl() -> String { "/usr/local/bin/kubectl".into() }
fn default_true() -> bool { true }

pub fn load(path: &str) -> anyhow::Result<Config> {
    let content = fs::read_to_string(path)?;
    let config: Config = serde_yaml::from_str(&content)?;
    Ok(config)
}
