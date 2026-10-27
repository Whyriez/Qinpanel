use crate::error::AnalyticsError;
use crate::models::ParsedLogEntry;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};

/// Raw JSON object as emitted by Nginx with `panel_json` log format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NginxJsonLog {
    #[serde(alias = "t", alias = "time_iso8601")]
    pub time_local: Option<String>,

    #[serde(alias = "ip")]
    pub remote_addr: Option<String>,

    #[serde(alias = "m")]
    pub request_method: Option<String>,

    #[serde(alias = "u")]
    pub request_uri: Option<String>,

    #[serde(alias = "s", default, deserialize_with = "deserialize_optional_u16")]
    pub status: Option<u16>,

    #[serde(alias = "b", default, deserialize_with = "deserialize_optional_u64")]
    pub body_bytes_sent: Option<u64>,

    #[serde(alias = "rt", default, deserialize_with = "deserialize_optional_f64")]
    pub request_time: Option<f64>,

    #[serde(alias = "urt", default, deserialize_with = "deserialize_optional_f64")]
    pub upstream_response_time: Option<f64>,

    #[serde(alias = "cs")]
    pub upstream_cache_status: Option<String>,

    #[serde(alias = "ref", alias = "referer", alias = "http_referer")]
    pub http_referrer: Option<String>,

    #[serde(alias = "ua")]
    pub http_user_agent: Option<String>,

    #[serde(alias = "h")]
    pub host: Option<String>,

    #[serde(alias = "proto")]
    pub server_protocol: Option<String>,

    #[serde(alias = "tls")]
    pub ssl_protocol: Option<String>,
}

fn deserialize_optional_u16<'de, D>(deserializer: D) -> Result<Option<u16>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum NumOrStr {
        Num(u16),
        Str(String),
    }

    match Option::<NumOrStr>::deserialize(deserializer)? {
        Some(NumOrStr::Num(n)) => Ok(Some(n)),
        Some(NumOrStr::Str(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() || trimmed == "-" {
                Ok(None)
            } else {
                Ok(trimmed.parse::<u16>().ok())
            }
        }
        None => Ok(None),
    }
}

fn deserialize_optional_u64<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum NumOrStr {
        Num(u64),
        Str(String),
    }

    match Option::<NumOrStr>::deserialize(deserializer)? {
        Some(NumOrStr::Num(n)) => Ok(Some(n)),
        Some(NumOrStr::Str(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() || trimmed == "-" {
                Ok(None)
            } else {
                Ok(trimmed.parse::<u64>().ok())
            }
        }
        None => Ok(None),
    }
}

fn deserialize_optional_f64<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum NumOrStr {
        Num(f64),
        Str(String),
    }

    match Option::<NumOrStr>::deserialize(deserializer)? {
        Some(NumOrStr::Num(f)) => Ok(Some(f)),
        Some(NumOrStr::Str(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() || trimmed == "-" {
                Ok(None)
            } else if let Ok(f) = trimmed.parse::<f64>() {
                Ok(Some(f))
            } else {
                let last_val = trimmed
                    .split([',', ':', ' '])
                    .map(str::trim)
                    .rfind(|part| !part.is_empty() && *part != "-");
                Ok(last_val.and_then(|part| part.parse::<f64>().ok()))
            }
        }
        None => Ok(None),
    }
}

impl NginxJsonLog {
    /// Parse a single JSON log line.
    pub fn parse_line(line: &str) -> Result<Self, AnalyticsError> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Err(AnalyticsError::Parse("Empty log line".into()));
        }
        serde_json::from_str::<Self>(trimmed)
            .map_err(|e| AnalyticsError::Parse(format!("Failed to parse JSON log line: {e}")))
    }

    /// Convert into structured `ParsedLogEntry` attributing to a specific `site_id`.
    pub fn into_entry(self, site_id: i64) -> ParsedLogEntry {
        let timestamp = self
            .time_local
            .as_deref()
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .map(|dt| dt.timestamp())
            .unwrap_or_else(|| Utc::now().timestamp());

        let client_ip = self
            .remote_addr
            .filter(|s| s != "-")
            .unwrap_or_else(|| "127.0.0.1".into());
        let host = self
            .host
            .filter(|s| s != "-")
            .unwrap_or_else(|| "localhost".into());
        let method = self
            .request_method
            .filter(|s| s != "-")
            .unwrap_or_else(|| "GET".into());
        let uri = self
            .request_uri
            .filter(|s| s != "-")
            .unwrap_or_else(|| "/".into());
        let status = self.status.unwrap_or(200);
        let body_bytes_sent = self.body_bytes_sent.unwrap_or(0);
        let request_time_secs = self.request_time.unwrap_or(0.0);
        let upstream_response_time_secs = self.upstream_response_time;
        let upstream_cache_status = self.upstream_cache_status.filter(|s| s != "-");
        let referer = self.http_referrer.filter(|s| s != "-");
        let user_agent = self.http_user_agent.filter(|s| s != "-");
        let protocol = self.server_protocol.filter(|s| s != "-");
        let tls_version = self.ssl_protocol.filter(|s| s != "-");

        ParsedLogEntry {
            site_id,
            timestamp,
            client_ip,
            host,
            method,
            uri,
            status,
            body_bytes_sent,
            request_time_secs,
            upstream_response_time_secs,
            upstream_cache_status,
            referer,
            user_agent,
            protocol,
            tls_version,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_full_nginx_json_line() {
        let line = r#"{"time_local":"2026-10-10T14:30:00+00:00","remote_addr":"203.0.113.195","request_method":"GET","request_uri":"/blog/post-1","status":200,"body_bytes_sent":2048,"request_time":0.045,"upstream_response_time":"0.038","upstream_cache_status":"HIT","http_referrer":"https://google.com","http_user_agent":"Mozilla/5.0","host":"example.com","server_protocol":"HTTP/2.0","ssl_protocol":"TLSv1.3"}"#;

        let parsed = NginxJsonLog::parse_line(line).expect("parse line");
        let entry = parsed.into_entry(42);

        assert_eq!(entry.site_id, 42);
        assert_eq!(entry.timestamp, 1791642600);
        assert_eq!(entry.client_ip, "203.0.113.195");
        assert_eq!(entry.host, "example.com");
        assert_eq!(entry.method, "GET");
        assert_eq!(entry.uri, "/blog/post-1");
        assert_eq!(entry.status, 200);
        assert_eq!(entry.body_bytes_sent, 2048);
        assert!((entry.request_time_secs - 0.045).abs() < f64::EPSILON);
        assert_eq!(entry.upstream_response_time_secs, Some(0.038));
        assert_eq!(entry.upstream_cache_status.as_deref(), Some("HIT"));
        assert_eq!(entry.referer.as_deref(), Some("https://google.com"));
        assert_eq!(entry.user_agent.as_deref(), Some("Mozilla/5.0"));
        assert_eq!(entry.protocol.as_deref(), Some("HTTP/2.0"));
        assert_eq!(entry.tls_version.as_deref(), Some("TLSv1.3"));
    }

    #[test]
    fn test_parse_short_alias_nginx_json_line() {
        let line = r#"{"t":"2026-10-10T14:30:00Z","ip":"198.51.100.5","m":"POST","u":"/api/login","s":"401","b":"128","rt":"0.120","urt":"-","cs":"-","ref":"-","ua":"curl/7.88.1","h":"api.example.com","proto":"HTTP/1.1","tls":"TLSv1.2"}"#;

        let parsed = NginxJsonLog::parse_line(line).expect("parse line");
        let entry = parsed.into_entry(10);

        assert_eq!(entry.site_id, 10);
        assert_eq!(entry.client_ip, "198.51.100.5");
        assert_eq!(entry.method, "POST");
        assert_eq!(entry.uri, "/api/login");
        assert_eq!(entry.status, 401);
        assert_eq!(entry.body_bytes_sent, 128);
        assert!((entry.request_time_secs - 0.120).abs() < f64::EPSILON);
        assert_eq!(entry.upstream_response_time_secs, None);
        assert_eq!(entry.upstream_cache_status, None);
        assert_eq!(entry.referer, None);
        assert_eq!(entry.user_agent.as_deref(), Some("curl/7.88.1"));
        assert_eq!(entry.protocol.as_deref(), Some("HTTP/1.1"));
        assert_eq!(entry.tls_version.as_deref(), Some("TLSv1.2"));
    }

    #[test]
    fn test_parse_empty_or_invalid_line() {
        assert!(NginxJsonLog::parse_line("").is_err());
        assert!(NginxJsonLog::parse_line("   ").is_err());
        assert!(NginxJsonLog::parse_line("NOT_JSON").is_err());
    }
}
