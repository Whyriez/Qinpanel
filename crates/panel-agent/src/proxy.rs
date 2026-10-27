//! Upstream probe for reverse proxy configurations.
//!
//! Checks health and latency of HTTP/TCP or Unix domain socket upstreams.

use panel_proto::{ProxyUpstreamProbeRequest, ProxyUpstreamProbeResponse};
use std::time::{Duration, Instant};

/// Probes an upstream target URL (http://, https://, or unix:) for availability and latency.
pub async fn probe_upstream(req: &ProxyUpstreamProbeRequest) -> ProxyUpstreamProbeResponse {
    let timeout_ms = req.timeout_ms.unwrap_or(5000).clamp(500, 30000);
    let timeout_dur = Duration::from_millis(timeout_ms);
    let start = Instant::now();

    let raw_url = req.upstream_url.trim();

    if let Some(sock_path) = raw_url.strip_prefix("unix:") {
        return probe_unix_socket(sock_path, timeout_dur, start).await;
    }

    if raw_url.starts_with("http://") || raw_url.starts_with("https://") {
        return probe_network_upstream(raw_url, timeout_dur, start).await;
    }

    ProxyUpstreamProbeResponse {
        healthy: false,
        status_code: None,
        latency_ms: start.elapsed().as_millis() as u64,
        message: format!("Unsupported upstream scheme: {raw_url}"),
    }
}

async fn probe_unix_socket(
    path: &str,
    timeout: Duration,
    start: Instant,
) -> ProxyUpstreamProbeResponse {
    #[cfg(unix)]
    {
        match tokio::time::timeout(timeout, tokio::net::UnixStream::connect(path)).await {
            Ok(Ok(mut stream)) => {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let req_bytes = b"GET / HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\n\r\n";
                let _ = stream.write_all(req_bytes).await;
                let mut buf = [0u8; 256];
                let status_code = if let Ok(n) = stream.read(&mut buf).await {
                    if n > 0 {
                        parse_status_code(&buf[..n])
                    } else {
                        None
                    }
                } else {
                    None
                };

                let latency_ms = start.elapsed().as_millis() as u64;
                ProxyUpstreamProbeResponse {
                    healthy: true,
                    status_code,
                    latency_ms,
                    message: format!("Unix socket connected: {path}"),
                }
            }
            Ok(Err(e)) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                ProxyUpstreamProbeResponse {
                    healthy: false,
                    status_code: None,
                    latency_ms,
                    message: format!("Unix socket connection failed: {e}"),
                }
            }
            Err(_) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                ProxyUpstreamProbeResponse {
                    healthy: false,
                    status_code: None,
                    latency_ms,
                    message: format!("Unix socket connection timed out: {path}"),
                }
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = timeout;
        let latency_ms = start.elapsed().as_millis() as u64;
        let exists = std::path::Path::new(path).exists();
        ProxyUpstreamProbeResponse {
            healthy: exists,
            status_code: None,
            latency_ms,
            message: if exists {
                format!("Unix socket path exists (non-unix host): {path}")
            } else {
                format!("Unix socket path not found: {path}")
            },
        }
    }
}

async fn probe_network_upstream(
    raw_url: &str,
    timeout: Duration,
    start: Instant,
) -> ProxyUpstreamProbeResponse {
    let (is_https, authority, path) = if let Some(rest) = raw_url.strip_prefix("https://") {
        let (auth, p) = split_authority_and_path(rest);
        (true, auth, p)
    } else if let Some(rest) = raw_url.strip_prefix("http://") {
        let (auth, p) = split_authority_and_path(rest);
        (false, auth, p)
    } else {
        return ProxyUpstreamProbeResponse {
            healthy: false,
            status_code: None,
            latency_ms: start.elapsed().as_millis() as u64,
            message: "Invalid URL scheme".to_string(),
        };
    };

    let (host, default_port) = if is_https {
        (authority, 443u16)
    } else {
        (authority, 80u16)
    };

    let (host_str, port) = match parse_host_and_port(host, default_port) {
        Ok(res) => res,
        Err(err) => {
            return ProxyUpstreamProbeResponse {
                healthy: false,
                status_code: None,
                latency_ms: start.elapsed().as_millis() as u64,
                message: err,
            };
        }
    };

    let target_addr = format!("{host_str}:{port}");

    match tokio::time::timeout(timeout, tokio::net::TcpStream::connect(&target_addr)).await {
        Ok(Ok(mut stream)) => {
            if is_https {
                let latency_ms = start.elapsed().as_millis() as u64;
                ProxyUpstreamProbeResponse {
                    healthy: true,
                    status_code: None,
                    latency_ms,
                    message: format!("TCP connected to HTTPS target {target_addr}"),
                }
            } else {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let http_req = format!(
                    "GET {path} HTTP/1.1\r\nHost: {host_str}\r\nUser-Agent: panel-agent-probe\r\nConnection: close\r\n\r\n"
                );
                let _ = stream.write_all(http_req.as_bytes()).await;

                let mut buf = [0u8; 512];
                let status_code =
                    match tokio::time::timeout(Duration::from_millis(1500), stream.read(&mut buf))
                        .await
                    {
                        Ok(Ok(n)) if n > 0 => parse_status_code(&buf[..n]),
                        _ => None,
                    };

                let latency_ms = start.elapsed().as_millis() as u64;
                let healthy = status_code.is_none_or(|sc| sc < 500);
                let msg = match status_code {
                    Some(sc) => format!("HTTP {sc} from {target_addr}"),
                    None => format!("TCP connected to {target_addr}"),
                };

                ProxyUpstreamProbeResponse {
                    healthy,
                    status_code,
                    latency_ms,
                    message: msg,
                }
            }
        }
        Ok(Err(e)) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            ProxyUpstreamProbeResponse {
                healthy: false,
                status_code: None,
                latency_ms,
                message: format!("Connection failed to {target_addr}: {e}"),
            }
        }
        Err(_) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            ProxyUpstreamProbeResponse {
                healthy: false,
                status_code: None,
                latency_ms,
                message: format!("Connection timed out to {target_addr}"),
            }
        }
    }
}

fn split_authority_and_path(s: &str) -> (&str, &str) {
    if let Some(pos) = s.find('/') {
        (&s[..pos], &s[pos..])
    } else {
        (s, "/")
    }
}

fn parse_host_and_port(authority: &str, default_port: u16) -> Result<(&str, u16), String> {
    if authority.starts_with('[') {
        if let Some(close_bracket) = authority.find(']') {
            let host = &authority[1..close_bracket];
            let rest = &authority[close_bracket + 1..];
            if rest.is_empty() {
                Ok((host, default_port))
            } else if let Some(port_str) = rest.strip_prefix(':') {
                let port = port_str
                    .parse::<u16>()
                    .map_err(|_| format!("Invalid port in authority: {rest}"))?;
                Ok((host, port))
            } else {
                Err(format!("Malformed IPv6 authority: {authority}"))
            }
        } else {
            Err(format!("Unclosed bracket in authority: {authority}"))
        }
    } else if let Some((h, p)) = authority.split_once(':') {
        let port = p
            .parse::<u16>()
            .map_err(|_| format!("Invalid port in authority: {p}"))?;
        Ok((h, port))
    } else {
        Ok((authority, default_port))
    }
}

fn parse_status_code(bytes: &[u8]) -> Option<u16> {
    let s = std::str::from_utf8(bytes).ok()?;
    let first_line = s.lines().next()?;
    let mut parts = first_line.split_whitespace();
    let _proto = parts.next()?;
    let code_str = parts.next()?;
    code_str.parse::<u16>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_host_and_port() {
        assert_eq!(
            parse_host_and_port("127.0.0.1:8080", 80),
            Ok(("127.0.0.1", 8080))
        );
        assert_eq!(
            parse_host_and_port("example.com", 443),
            Ok(("example.com", 443))
        );
        assert_eq!(parse_host_and_port("[::1]:9000", 80), Ok(("::1", 9000)));
        assert_eq!(parse_host_and_port("[::1]", 80), Ok(("::1", 80)));
    }

    #[test]
    fn test_parse_status_code() {
        let resp = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
        assert_eq!(parse_status_code(resp), Some(200));

        let resp_404 = b"HTTP/1.1 404 Not Found\r\n\r\n";
        assert_eq!(parse_status_code(resp_404), Some(404));

        let invalid = b"GARBAGE";
        assert_eq!(parse_status_code(invalid), None);
    }

    #[tokio::test]
    async fn test_probe_network_upstream_listener() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut in_buf = [0u8; 256];
                let _ = stream.read(&mut in_buf).await;
                let _ = stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK")
                    .await;
                let _ = stream.shutdown().await;
            }
        });

        let req = ProxyUpstreamProbeRequest {
            upstream_url: format!("http://127.0.0.1:{port}/health"),
            timeout_ms: Some(1000),
        };

        let res = probe_upstream(&req).await;
        assert!(res.healthy);
        assert_eq!(res.status_code, Some(200));
    }
}
