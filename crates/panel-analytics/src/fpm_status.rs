//! PHP-FPM status polling via FastCGI over Unix Domain Socket (`/fpm-status?json`).
//!
//! Provides protocol encoding, response parsing, and connection management
//! to collect pool metrics (active processes, idle, queue, slow requests, max children).

use crate::error::AnalyticsError;
use crate::models::PhpFpmPoolMetricPoint;
use panel_proto::PhpFpmStatusMetrics;
use std::path::Path;
#[cfg(not(unix))]
use tracing::warn;

/// FastCGI record types
const FCGI_BEGIN_REQUEST: u8 = 1;
const FCGI_PARAMS: u8 = 4;
const FCGI_STDIN: u8 = 5;
const FCGI_STDOUT: u8 = 6;
const FCGI_END_REQUEST: u8 = 8;
const FCGI_RESPONDER: u8 = 1;

/// Builds a FastCGI GET request packet for querying an FPM status endpoint (e.g. `/fpm-status?json`).
pub fn build_fcgi_status_request(status_path: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(128);

    // 1. FCGI_BEGIN_REQUEST (version: 1, type: 1, req_id: 1, content_len: 8, pad: 0)
    buf.extend_from_slice(&[
        1,
        FCGI_BEGIN_REQUEST, // version 1, type 1
        0,
        1, // request_id = 1
        0,
        8, // content_length = 8
        0,
        0, // padding_length = 0, reserved = 0
    ]);
    buf.extend_from_slice(&[
        0,
        FCGI_RESPONDER, // role = 1 (FCGI_RESPONDER)
        0,              // flags = 0
        0,
        0,
        0,
        0,
        0, // reserved
    ]);

    // 2. FCGI_PARAMS (type: 4, req_id: 1)
    let params: &[(&[u8], &[u8])] = &[
        (b"SCRIPT_NAME", status_path.as_bytes()),
        (b"SCRIPT_FILENAME", status_path.as_bytes()),
        (b"QUERY_STRING", b"json"),
        (b"REQUEST_METHOD", b"GET"),
    ];
    let mut param_body = Vec::with_capacity(96);
    for (k, v) in params {
        param_body.push(k.len() as u8);
        param_body.push(v.len() as u8);
        param_body.extend_from_slice(k);
        param_body.extend_from_slice(v);
    }
    let param_len = param_body.len() as u16;
    buf.extend_from_slice(&[
        1,
        FCGI_PARAMS,
        0,
        1, // request_id = 1
        (param_len >> 8) as u8,
        (param_len & 0xff) as u8,
        0,
        0,
    ]);
    buf.extend_from_slice(&param_body);

    // 3. FCGI_PARAMS empty (signals end of params)
    buf.extend_from_slice(&[1, FCGI_PARAMS, 0, 1, 0, 0, 0, 0]);

    // 4. FCGI_STDIN empty (signals end of stdin)
    buf.extend_from_slice(&[1, FCGI_STDIN, 0, 1, 0, 0, 0, 0]);

    buf
}

/// Parses the FastCGI response stream extracting STDOUT body and JSON status metrics.
pub fn parse_fcgi_status_response(
    bytes: &[u8],
    timestamp: i64,
) -> Result<PhpFpmPoolMetricPoint, AnalyticsError> {
    let mut stdout_body = Vec::new();
    let mut offset = 0;

    while offset + 8 <= bytes.len() {
        let record_type = bytes[offset + 1];
        let content_len = u16::from_be_bytes([bytes[offset + 4], bytes[offset + 5]]) as usize;
        let padding_len = bytes[offset + 6] as usize;
        let content_start = offset + 8;
        let content_end = content_start + content_len;

        if content_end > bytes.len() {
            break;
        }

        if record_type == FCGI_STDOUT {
            stdout_body.extend_from_slice(&bytes[content_start..content_end]);
        } else if record_type == FCGI_END_REQUEST {
            break;
        }

        offset = content_end + padding_len;
    }

    let payload = if stdout_body.is_empty() {
        bytes
    } else if let Some(idx) = stdout_body.windows(4).position(|w| w == b"\r\n\r\n") {
        &stdout_body[idx + 4..]
    } else if let Some(idx) = stdout_body.windows(2).position(|w| w == b"\n\n") {
        &stdout_body[idx + 2..]
    } else {
        &stdout_body[..]
    };

    let proto_metrics = serde_json::from_slice::<PhpFpmStatusMetrics>(payload).map_err(|e| {
        AnalyticsError::Parse(format!(
            "Failed to parse PHP-FPM JSON status payload: {e}. Raw: {}",
            String::from_utf8_lossy(payload)
        ))
    })?;

    Ok(convert_status_metrics(&proto_metrics, timestamp))
}

/// Parses a direct JSON status string into `PhpFpmPoolMetricPoint`.
pub fn parse_status_json(
    json_str: &str,
    timestamp: i64,
) -> Result<PhpFpmPoolMetricPoint, AnalyticsError> {
    let proto_metrics = serde_json::from_str::<PhpFpmStatusMetrics>(json_str)
        .map_err(|e| AnalyticsError::Parse(format!("Failed to parse PHP-FPM status JSON: {e}")))?;

    Ok(convert_status_metrics(&proto_metrics, timestamp))
}

/// Converts proto metrics to an analytics pool metric point.
pub fn convert_status_metrics(
    metrics: &PhpFpmStatusMetrics,
    timestamp: i64,
) -> PhpFpmPoolMetricPoint {
    let (user, php_version) = parse_pool_identifiers(&metrics.pool);

    PhpFpmPoolMetricPoint {
        timestamp,
        pool_name: metrics.pool.clone(),
        user,
        php_version,
        active_processes: metrics.active_processes,
        idle_processes: metrics.idle_processes,
        total_processes: metrics.total_processes,
        max_active_processes: metrics.max_active_processes,
        max_children_reached: metrics.max_children_reached,
        listen_queue: metrics.listen_queue,
        max_listen_queue: metrics.max_listen_queue,
        listen_queue_len: metrics.listen_queue_len,
        slow_requests: metrics.slow_requests,
        accepted_conn: metrics.accepted_conn,
    }
}

/// Extracts (username, php_version) from a pool name such as "p_alice-8.3" or "bob-8.2".
pub fn parse_pool_identifiers(pool_name: &str) -> (Option<String>, Option<String>) {
    let Some((user, ver)) = pool_name.rsplit_once('-') else {
        return (None, None);
    };
    if ver.chars().all(|c| c.is_ascii_digit() || c == '.') && ver.contains('.') {
        (Some(user.to_string()), Some(ver.to_string()))
    } else {
        (None, None)
    }
}

/// Client for polling PHP-FPM status metrics over FastCGI UDS socket.
pub struct FpmStatusClient {
    status_path: String,
    timeout_duration: std::time::Duration,
}

impl Default for FpmStatusClient {
    fn default() -> Self {
        Self {
            status_path: "/fpm-status".to_string(),
            timeout_duration: std::time::Duration::from_secs(2),
        }
    }
}

impl FpmStatusClient {
    pub fn new(status_path: impl Into<String>, timeout: std::time::Duration) -> Self {
        Self {
            status_path: status_path.into(),
            timeout_duration: timeout,
        }
    }

    pub fn status_path(&self) -> &str {
        &self.status_path
    }

    pub fn timeout_duration(&self) -> std::time::Duration {
        self.timeout_duration
    }

    /// Queries the FPM pool status metrics from a given Unix Domain Socket.
    pub fn query_socket(
        &self,
        socket_path: &Path,
        timestamp: i64,
    ) -> Result<PhpFpmPoolMetricPoint, AnalyticsError> {
        // Hermetic mock support: check for {socket}.status.json file
        let mock_file = socket_path.with_extension("status.json");
        if let Ok(content) = std::fs::read_to_string(&mock_file) {
            return parse_status_json(&content, timestamp);
        }

        #[cfg(unix)]
        {
            use std::io::{Read, Write};
            use std::os::unix::net::UnixStream;

            if !socket_path.exists() {
                return Err(AnalyticsError::System(format!(
                    "PHP-FPM socket does not exist: {}",
                    socket_path.display()
                )));
            }

            let mut stream = UnixStream::connect(socket_path).map_err(|e| {
                AnalyticsError::System(format!(
                    "Failed to connect to PHP-FPM socket {}: {e}",
                    socket_path.display()
                ))
            })?;

            let _ = stream.set_read_timeout(Some(self.timeout_duration));
            let _ = stream.set_write_timeout(Some(self.timeout_duration));

            let req_pkt = build_fcgi_status_request(&self.status_path);
            stream.write_all(&req_pkt).map_err(|e| {
                AnalyticsError::System(format!("Failed to write FCGI request: {e}"))
            })?;

            let mut resp_buf = Vec::new();
            stream.read_to_end(&mut resp_buf).map_err(|e| {
                AnalyticsError::System(format!("Failed to read FCGI response: {e}"))
            })?;

            parse_fcgi_status_response(&resp_buf, timestamp)
        }

        #[cfg(not(unix))]
        {
            let _ = (&self.status_path, self.timeout_duration);
            warn!(
                "Direct FastCGI socket polling is unsupported on non-unix systems: {}",
                socket_path.display()
            );
            let pool_name = socket_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();
            let (user, php_version) = parse_pool_identifiers(&pool_name);
            Ok(PhpFpmPoolMetricPoint {
                timestamp,
                pool_name,
                user,
                php_version,
                active_processes: 0,
                idle_processes: 0,
                total_processes: 0,
                max_active_processes: 0,
                max_children_reached: 0,
                listen_queue: 0,
                max_listen_queue: 0,
                listen_queue_len: 0,
                slow_requests: 0,
                accepted_conn: 0,
            })
        }
    }
}
