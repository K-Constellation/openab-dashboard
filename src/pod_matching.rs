/// Check whether a live Kubernetes pod name is owned by an exact deployment.
///
/// Kubernetes pod names follow `<deployment>-<rs-hash>-<pod-hash>`.
/// The deployment name must appear as an exact `-` separated prefix, not as a
/// substring. The suffix must contain exactly two non-empty segments.
pub(crate) fn is_deployment_pod_name(pod_name: &str, deployment: &str) -> bool {
    if deployment.is_empty() {
        return false;
    }
    let prefix = format!("{}-", deployment);
    let Some(suffix) = pod_name.strip_prefix(&prefix) else {
        return false;
    };
    let mut parts = suffix.split('-');
    let (Some(first), Some(second), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    !first.is_empty() && !second.is_empty()
}

#[cfg(test)]
mod tests {
    use super::is_deployment_pod_name;

    #[test]
    fn accepts_exact_two_suffix_segments() {
        assert!(is_deployment_pod_name("openab-my-agy-7d4c8f9b5-x2kp9", "openab-my-agy"));
    }

    #[test]
    fn rejects_three_suffix_segments() {
        assert!(!is_deployment_pod_name("openab-my-agy-v2-7d4c8f9b5-x2kp9", "openab-my-agy"));
    }

    #[test]
    fn rejects_one_suffix_segment() {
        assert!(!is_deployment_pod_name("openab-my-agy-x2kp9", "openab-my-agy"));
    }

    #[test]
    fn rejects_empty_suffix_segment() {
        assert!(!is_deployment_pod_name("openab-my-agy-7d4c8f9b5-", "openab-my-agy"));
        assert!(!is_deployment_pod_name("openab-my-agy--x2kp9", "openab-my-agy"));
    }

    #[test]
    fn rejects_empty_deployment() {
        assert!(!is_deployment_pod_name("openab-my-agy-7d4c8f9b5-x2kp9", ""));
    }
}
