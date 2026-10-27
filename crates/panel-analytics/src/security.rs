//! Security analytics, probing path detection, brute force pattern recognition, and IP risk scoring.

use crate::models::ParsedLogEntry;
use serde::{Deserialize, Serialize};

/// Type of security event detected from traffic logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityEventType {
    Probing,
    BruteForce,
    HighErrorRate,
    General,
}

impl SecurityEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Probing => "probing",
            Self::BruteForce => "brute_force",
            Self::HighErrorRate => "high_error_rate",
            Self::General => "general",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "probing" => Some(Self::Probing),
            "brute_force" => Some(Self::BruteForce),
            "high_error_rate" => Some(Self::HighErrorRate),
            "general" => Some(Self::General),
            _ => None,
        }
    }
}

/// Threat risk classification of a client IP address.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum SecurityRiskLevel {
    #[default]
    Low,
    Suspicious,
    HighRisk,
    Critical,
}

impl SecurityRiskLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Suspicious => "suspicious",
            Self::HighRisk => "high_risk",
            Self::Critical => "critical",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "low" => Some(Self::Low),
            "suspicious" => Some(Self::Suspicious),
            "high_risk" => Some(Self::HighRisk),
            "critical" => Some(Self::Critical),
            _ => None,
        }
    }
}

/// A recorded individual security event or suspicious anomaly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SecurityEvent {
    pub id: Option<i64>,
    pub site_id: i64,
    pub timestamp: i64,
    pub client_ip: String,
    pub method: String,
    pub uri: String,
    pub status: u16,
    pub user_agent: Option<String>,
    pub country: Option<String>,
    pub event_type: SecurityEventType,
    pub threat_score: u32,
}

/// Statistics and risk breakdown for a top client IP address.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TopSecurityIp {
    pub ip: String,
    pub total_requests: u64,
    pub error_4xx: u64,
    pub error_5xx: u64,
    pub probing_count: u64,
    pub auth_failures: u64,
    pub country: Option<String>,
    pub top_path: Option<String>,
    pub risk_level: SecurityRiskLevel,
    pub last_seen: i64,
}

/// Probing frequency statistics for sensitive paths.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProbingPathStat {
    pub path: String,
    pub category: String,
    pub count: u64,
    pub unique_ips: u64,
}

/// An identified brute force attack incident against an authentication endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BruteForceIncident {
    pub ip: String,
    pub target_path: String,
    pub failed_attempts: u64,
    pub first_attempt_ts: i64,
    pub last_attempt_ts: i64,
    pub country: Option<String>,
    pub risk_score: u32,
}

/// Complete aggregated security overview summary for a site over a time window.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SecurityOverviewSummary {
    pub total_probing_requests: u64,
    pub total_brute_force_attempts: u64,
    pub unique_suspicious_ips: u64,
    pub top_ips: Vec<TopSecurityIp>,
    pub probing_paths: Vec<ProbingPathStat>,
    pub brute_force_incidents: Vec<BruteForceIncident>,
}

/// Checks whether an HTTP request URI matches known sensitive probing signatures.
///
/// Returns `Some((category_name, threat_score))` if a probing pattern is detected.
pub fn classify_probing_path(raw_uri: &str) -> Option<(&'static str, u32)> {
    let uri = raw_uri
        .split('?')
        .next()
        .unwrap_or(raw_uri)
        .to_ascii_lowercase();

    // 1. Environment & Credential files
    if uri.ends_with("/.env")
        || uri.contains(".env.")
        || uri == "/.env"
        || uri.contains("/.aws/")
        || uri.ends_with("/config.json")
    {
        return Some(("env_credentials", 85));
    }

    // 2. Version control leaks (.git, .svn)
    if uri.contains("/.git") || uri.contains("/.svn") || uri.ends_with("/.git/config") {
        return Some(("vcs_leak", 80));
    }

    // 3. WordPress attack vectors
    if uri.ends_with("/wp-login.php")
        || uri.ends_with("/xmlrpc.php")
        || uri.ends_with("/wp-config.php")
    {
        return Some(("wordpress_probing", 75));
    }

    // 4. Database management tools
    if uri.contains("/phpmyadmin") || uri.contains("/pma") || uri.contains("/adminer") {
        return Some(("db_admin", 75));
    }

    // 5. Sensitive server / debug dumps
    if uri.ends_with("/phpinfo.php")
        || uri.ends_with("/info.php")
        || uri.ends_with("/backup.sql")
        || uri.ends_with("/dump.sql")
        || uri.ends_with("/.ds_store")
        || uri.ends_with("/.htaccess")
    {
        return Some(("debug_dump", 70));
    }

    // 6. Generic administrative panel probing
    if uri.ends_with("/actuator")
        || uri.contains("/actuator/")
        || uri.ends_with("/setup.php")
        || uri.ends_with("/install.php")
    {
        return Some(("admin_probing", 65));
    }

    None
}

/// Checks if an endpoint represents an authentication / login path.
pub fn is_auth_endpoint(raw_uri: &str) -> bool {
    let uri = raw_uri
        .split('?')
        .next()
        .unwrap_or(raw_uri)
        .to_ascii_lowercase();
    uri.ends_with("/wp-login.php")
        || uri.ends_with("/xmlrpc.php")
        || uri.ends_with("/login")
        || uri.ends_with("/signin")
        || uri.contains("/api/login")
        || uri.contains("/api/auth")
        || uri.ends_with("/admin/login")
}

/// Evaluates a parsed log entry and returns a `SecurityEvent` if suspicious behavior is identified.
pub fn analyze_log_entry(entry: &ParsedLogEntry, country: Option<&str>) -> Option<SecurityEvent> {
    // 1. Probing check
    if let Some((_category, score)) = classify_probing_path(&entry.uri) {
        return Some(SecurityEvent {
            id: None,
            site_id: entry.site_id,
            timestamp: entry.timestamp,
            client_ip: entry.client_ip.clone(),
            method: entry.method.clone(),
            uri: entry.uri.clone(),
            status: entry.status,
            user_agent: entry.user_agent.clone(),
            country: country.map(|s| s.to_string()),
            event_type: SecurityEventType::Probing,
            threat_score: score,
        });
    }

    // 2. Auth failure / brute force attempt (HTTP 401, 403, 429 on auth endpoints)
    if is_auth_endpoint(&entry.uri)
        && (entry.status == 401 || entry.status == 403 || entry.status == 429)
    {
        return Some(SecurityEvent {
            id: None,
            site_id: entry.site_id,
            timestamp: entry.timestamp,
            client_ip: entry.client_ip.clone(),
            method: entry.method.clone(),
            uri: entry.uri.clone(),
            status: entry.status,
            user_agent: entry.user_agent.clone(),
            country: country.map(|s| s.to_string()),
            event_type: SecurityEventType::BruteForce,
            threat_score: 70,
        });
    }

    // 3. Repeated client errors (HTTP 403 / 404 scanning)
    if entry.status == 403 || entry.status == 404 {
        return Some(SecurityEvent {
            id: None,
            site_id: entry.site_id,
            timestamp: entry.timestamp,
            client_ip: entry.client_ip.clone(),
            method: entry.method.clone(),
            uri: entry.uri.clone(),
            status: entry.status,
            user_agent: entry.user_agent.clone(),
            country: country.map(|s| s.to_string()),
            event_type: SecurityEventType::HighErrorRate,
            threat_score: 25,
        });
    }

    None
}

/// Calculates the overall risk level for a given client IP based on its behavior metrics.
pub fn calculate_ip_risk_level(
    total_requests: u64,
    error_4xx: u64,
    probing_count: u64,
    auth_failures: u64,
) -> SecurityRiskLevel {
    if probing_count >= 10 || auth_failures >= 15 {
        SecurityRiskLevel::Critical
    } else if probing_count >= 3
        || auth_failures >= 5
        || (total_requests >= 20 && error_4xx * 2 >= total_requests)
    {
        SecurityRiskLevel::HighRisk
    } else if probing_count > 0
        || auth_failures > 0
        || (total_requests >= 10 && error_4xx > total_requests / 3)
    {
        SecurityRiskLevel::Suspicious
    } else {
        SecurityRiskLevel::Low
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_probing_path() {
        assert_eq!(
            classify_probing_path("/.env").map(|(c, _)| c),
            Some("env_credentials")
        );
        assert_eq!(
            classify_probing_path("/subfolder/.env.production").map(|(c, _)| c),
            Some("env_credentials")
        );
        assert_eq!(
            classify_probing_path("/.git/config").map(|(c, _)| c),
            Some("vcs_leak")
        );
        assert_eq!(
            classify_probing_path("/wp-login.php?redirect_to=foo").map(|(c, _)| c),
            Some("wordpress_probing")
        );
        assert_eq!(
            classify_probing_path("/xmlrpc.php").map(|(c, _)| c),
            Some("wordpress_probing")
        );
        assert_eq!(
            classify_probing_path("/phpmyadmin/index.php").map(|(c, _)| c),
            Some("db_admin")
        );
        assert_eq!(
            classify_probing_path("/backup.sql").map(|(c, _)| c),
            Some("debug_dump")
        );

        // Normal legitimate URLs
        assert_eq!(classify_probing_path("/index.html"), None);
        assert_eq!(classify_probing_path("/assets/logo.png"), None);
        assert_eq!(classify_probing_path("/api/v1/posts"), None);
    }

    #[test]
    fn test_is_auth_endpoint() {
        assert!(is_auth_endpoint("/wp-login.php"));
        assert!(is_auth_endpoint("/api/login"));
        assert!(is_auth_endpoint("/admin/login"));
        assert!(is_auth_endpoint("/signin"));
        assert!(!is_auth_endpoint("/products/view"));
    }

    #[test]
    fn test_analyze_log_entry_probing_and_brute_force() {
        let probe_entry = ParsedLogEntry {
            site_id: 1,
            timestamp: 1700000000,
            client_ip: "192.168.1.100".to_string(),
            host: "example.com".to_string(),
            method: "GET".to_string(),
            uri: "/.env".to_string(),
            status: 404,
            body_bytes_sent: 200,
            request_time_secs: 0.01,
            upstream_response_time_secs: None,
            upstream_cache_status: None,
            referer: None,
            user_agent: Some("curl/7.68".to_string()),
            protocol: Some("HTTP/1.1".to_string()),
            tls_version: None,
        };

        let ev = analyze_log_entry(&probe_entry, Some("ID")).expect("expected probe event");
        assert_eq!(ev.event_type, SecurityEventType::Probing);
        assert!(ev.threat_score >= 80);
        assert_eq!(ev.country.as_deref(), Some("ID"));

        let brute_entry = ParsedLogEntry {
            site_id: 1,
            timestamp: 1700000000,
            client_ip: "10.0.0.5".to_string(),
            host: "example.com".to_string(),
            method: "POST".to_string(),
            uri: "/wp-login.php".to_string(),
            status: 401,
            body_bytes_sent: 500,
            request_time_secs: 0.05,
            upstream_response_time_secs: None,
            upstream_cache_status: None,
            referer: None,
            user_agent: Some("python-requests/2.25".to_string()),
            protocol: Some("HTTP/1.1".to_string()),
            tls_version: None,
        };

        let ev_bf =
            analyze_log_entry(&brute_entry, Some("US")).expect("expected brute force event");
        // /wp-login.php matches both probing category and auth, probing takes precedence or score >= 70
        assert!(ev_bf.threat_score >= 70);
    }

    #[test]
    fn test_calculate_ip_risk_level() {
        assert_eq!(
            calculate_ip_risk_level(100, 2, 0, 0),
            SecurityRiskLevel::Low
        );
        assert_eq!(
            calculate_ip_risk_level(20, 5, 1, 0),
            SecurityRiskLevel::Suspicious
        );
        assert_eq!(
            calculate_ip_risk_level(30, 25, 4, 0),
            SecurityRiskLevel::HighRisk
        );
        assert_eq!(
            calculate_ip_risk_level(50, 40, 12, 5),
            SecurityRiskLevel::Critical
        );
    }
}
