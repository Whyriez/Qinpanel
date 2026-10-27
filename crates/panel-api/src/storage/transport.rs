use super::{domain::*, signing};
use axum::body::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::Request;
use hyper_util::rt::TokioIo;
use panel_core::{
    connectivity::EndpointHost,
    object_storage::{identity::StorageEndpoint, network::validate_dial_targets},
};
use panel_proto::S3Credentials;
use serde::Deserialize;
use std::{net::SocketAddr, time::Duration};
use tokio_rustls::{TlsConnector, rustls::pki_types::ServerName};

struct ConnectionTask(tokio::task::JoinHandle<()>);
impl Drop for ConnectionTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub async fn execute(
    connection: Connection,
    credentials: S3Credentials,
    operation: Operation,
    panel: StorageEndpoint,
) -> Result<Outcome, Failure> {
    tokio::time::timeout(
        Duration::from_secs(90),
        execute_inner(connection, credentials, operation, panel),
    )
    .await
    .map_err(|_| Failure::Timeout)?
}
async fn execute_inner(
    connection: Connection,
    credentials: S3Credentials,
    operation: Operation,
    panel: StorageEndpoint,
) -> Result<Outcome, Failure> {
    connection.validate().map_err(|_| Failure::Invalid)?;
    if !credentials.valid() {
        return Err(Failure::Invalid);
    }
    if connection.managed {
        return execute_managed(&connection, &credentials, operation, &panel).await;
    }
    let (core, targets) = admit(&connection, &panel).await?;
    if let Operation::Download { key } = &operation {
        super::domain::key(key)?;
        return Ok(Outcome::Download {
            url: signing::download(&connection, &credentials, key, chrono::Utc::now())?,
            expires_at: chrono::Utc::now().timestamp() + 60,
        });
    }
    let host = connection.request_host().map_err(|_| Failure::Invalid)?;
    let tls = connect(&host, &targets).await?;
    targets
        .require_current(&core, timestamp()?)
        .map_err(|_| Failure::Endpoint)?;
    let request = signed_request(&connection, &credentials, &operation)?;
    let limit = if matches!(operation, Operation::ReadObject { .. }) {
        UPLOAD_LIMIT
    } else {
        LIST_LIMIT
    };
    let body = exchange(tls, request, limit).await?;
    match operation {
        Operation::List { prefix, .. } => parse_listing(&body, &prefix).map(Outcome::Listing),
        Operation::ReadObject { .. } => Ok(Outcome::Content(body)),
        _ => Ok(Outcome::Done),
    }
}
// Only a trusted DB target can select the fixed authenticated S3 Unix socket.
// External URLs continue through the DNS/TLS guard above, including private IP rejection.
async fn execute_managed(
    connection: &Connection,
    credentials: &S3Credentials,
    operation: Operation,
    panel: &StorageEndpoint,
) -> Result<Outcome, Failure> {
    if connection.managed_state.as_deref() != Some("ready")
        || connection
            .core()
            .map_err(|_| Failure::Invalid)?
            .config
            .endpoint
            .host
            == panel.host
    {
        return Err(Failure::Endpoint);
    }
    if let Operation::Download { key } = &operation {
        super::domain::key(key)?;
        return Ok(Outcome::Download {
            url: signing::download(connection, credentials, key, chrono::Utc::now())?,
            expires_at: chrono::Utc::now().timestamp() + 60,
        });
    }
    #[cfg(unix)]
    {
        let socket = tokio::net::UnixStream::connect("/run/qonpanel-storage/s3.sock")
            .await
            .map_err(|_| Failure::Unavailable)?;
        let request = signed_request(connection, credentials, &operation)?;
        let limit = if matches!(operation, Operation::ReadObject { .. }) {
            UPLOAD_LIMIT
        } else {
            LIST_LIMIT
        };
        let body = exchange(socket, request, limit).await?;
        match operation {
            Operation::List { prefix, .. } => parse_listing(&body, &prefix).map(Outcome::Listing),
            Operation::ReadObject { .. } => Ok(Outcome::Content(body)),
            _ => Ok(Outcome::Done),
        }
    }
    #[cfg(not(unix))]
    {
        Err(Failure::Unavailable)
    }
}

fn timestamp() -> Result<u64, Failure> {
    u64::try_from(chrono::Utc::now().timestamp()).map_err(|_| Failure::Invalid)
}
async fn admit(
    connection: &Connection,
    panel: &StorageEndpoint,
) -> Result<
    (
        panel_core::object_storage::identity::StorageConnection,
        panel_core::object_storage::network::ValidatedDialTargets,
    ),
    Failure,
> {
    let host = connection.request_host().map_err(|_| Failure::Invalid)?;
    let endpoint_host = EndpointHost::parse(&host).map_err(|_| Failure::Invalid)?;
    if endpoint_host == panel.host {
        return Err(Failure::Endpoint);
    }
    let addresses: Vec<_> = tokio::net::lookup_host((host.as_str(), 443))
        .await
        .map_err(|_| Failure::Unavailable)?
        .take(17)
        .map(|v| v.ip())
        .collect();
    let mut core = connection.core().map_err(|_| Failure::Invalid)?;
    core.config.endpoint.host = endpoint_host;
    let targets = validate_dial_targets(&core, panel, &addresses, None, timestamp()?)
        .map_err(|_| Failure::Endpoint)?;
    Ok((core, targets))
}
async fn connect(
    host: &str,
    targets: &panel_core::object_storage::network::ValidatedDialTargets,
) -> Result<tokio_rustls::client::TlsStream<tokio::net::TcpStream>, Failure> {
    let mut socket = None;
    for ip in targets.addresses() {
        if let Ok(Ok(stream)) = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::TcpStream::connect(SocketAddr::new(*ip, 443)),
        )
        .await
        {
            socket = Some(stream);
            break;
        }
    }
    let socket = socket.ok_or(Failure::Unavailable)?;
    let name = ServerName::try_from(host.to_owned()).map_err(|_| Failure::Invalid)?;
    TlsConnector::from(crate::mail::system_trust().map_err(|_| Failure::Unavailable)?)
        .connect(name, socket)
        .await
        .map_err(|_| Failure::Unavailable)
}
fn signed_request(
    connection: &Connection,
    credentials: &S3Credentials,
    operation: &Operation,
) -> Result<Request<Full<Bytes>>, Failure> {
    let (method, path, query, body, overwrite) = prepare(connection, operation)?;
    let headers = signing::headers(
        connection,
        credentials,
        signing::SignRequest {
            method,
            path: &path,
            query: &query,
            body: &body,
            overwrite,
        },
        chrono::Utc::now(),
    )?;
    let uri = if query.is_empty() {
        path
    } else {
        format!("{path}?{query}")
    };
    let content_type = match operation {
        Operation::Upload { key, .. } => mime_guess::from_path(key)
            .first_or_octet_stream()
            .to_string(),
        Operation::CreateBucket => "application/xml".to_owned(),
        _ => "application/octet-stream".to_owned(),
    };
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", content_type);
    for (name, value) in headers {
        let mut value = hyper::header::HeaderValue::from_bytes(value.as_bytes())
            .map_err(|_| Failure::Invalid)?;
        if name == "authorization" || name == "x-amz-security-token" {
            value.set_sensitive(true);
        }
        request = request.header(name, value);
    }
    request.body(Full::new(body)).map_err(|_| Failure::Invalid)
}
async fn exchange<T>(io: T, request: Request<Full<Bytes>>, limit: usize) -> Result<Bytes, Failure>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut sender, task) = hyper::client::conn::http1::handshake(TokioIo::new(io))
        .await
        .map_err(|_| Failure::Unavailable)?;
    let _task = ConnectionTask(tokio::spawn(async move {
        let _ = task.await;
    }));
    let method = request.method().clone();
    let response = sender
        .send_request(request)
        .await
        .map_err(|_| Failure::Unavailable)?;
    if !response.status().is_success() {
        return Err(Failure::Provider(response.status().as_u16()));
    }
    let status = response.status().as_u16();
    let complete = match method {
        hyper::Method::GET => status == 200,
        hyper::Method::PUT => matches!(status, 200 | 201 | 204),
        hyper::Method::DELETE => matches!(status, 200 | 204),
        _ => false,
    };
    if !complete {
        return Err(Failure::Invalid);
    }
    Limited::new(response.into_body(), limit)
        .collect()
        .await
        .map(|body| body.to_bytes())
        .map_err(|_| Failure::Invalid)
}

type Prepared = (&'static str, String, String, Bytes, bool);
fn prepare(connection: &Connection, operation: &Operation) -> Result<Prepared, Failure> {
    match operation {
        Operation::List { prefix, token } => {
            if !list_input(prefix, token.as_deref()) {
                return Err(Failure::Invalid);
            }
            let mut values = vec![
                ("list-type".into(), "2".into()),
                ("max-keys".into(), "100".into()),
                ("delimiter".into(), "/".into()),
                ("encoding-type".into(), "url".into()),
                ("prefix".into(), prefix.clone()),
            ];
            if let Some(token) = token {
                values.push(("continuation-token".into(), token.clone()));
            }
            Ok((
                "GET",
                signing::path(connection, None),
                signing::query(&values),
                Bytes::new(),
                true,
            ))
        }
        Operation::Upload {
            key,
            body,
            overwrite,
        } => {
            if !overwrite && !connection.conditional_put() {
                return Err(Failure::ConditionalUnsupported);
            }
            super::domain::key(key)?;
            if body.len() > UPLOAD_LIMIT {
                return Err(Failure::Invalid);
            }
            Ok((
                "PUT",
                signing::path(connection, Some(key)),
                String::new(),
                body.clone(),
                *overwrite,
            ))
        }
        Operation::ReadObject { key } => {
            super::domain::key(key)?;
            Ok((
                "GET",
                signing::path(connection, Some(key)),
                String::new(),
                Bytes::new(),
                true,
            ))
        }
        Operation::Delete { key } => {
            super::domain::key(key)?;
            Ok((
                "DELETE",
                signing::path(connection, Some(key)),
                String::new(),
                Bytes::new(),
                true,
            ))
        }
        Operation::CreateBucket => {
            let body = if connection.provider == "aws" && connection.region != "us-east-1" {
                Bytes::from(format!(
                    "<CreateBucketConfiguration xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><LocationConstraint>{}</LocationConstraint></CreateBucketConfiguration>",
                    connection.region
                ))
            } else {
                Bytes::new()
            };
            Ok((
                "PUT",
                signing::path(connection, None),
                String::new(),
                body,
                true,
            ))
        }
        Operation::Download { .. } => Err(Failure::Invalid),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct XmlObject {
    key: String,
    size: u64,
    last_modified: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct XmlPrefix {
    prefix: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct XmlListing {
    #[serde(default)]
    contents: Vec<XmlObject>,
    #[serde(default)]
    common_prefixes: Vec<XmlPrefix>,
    is_truncated: bool,
    next_continuation_token: Option<String>,
    encoding_type: Option<String>,
}
fn safe_xml(text: &str) -> bool {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(text);
    let mut depth = 0usize;
    let mut first = true;
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) => {
                if depth == 0 {
                    if !first || element.local_name().as_ref() != b"ListBucketResult" {
                        return false;
                    }
                    first = false;
                }
                depth += 1;
                if depth > 16 {
                    return false;
                }
            }
            Ok(Event::Empty(_)) if first => return false,
            Ok(Event::End(_)) => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            Ok(Event::DocType(_)) | Err(_) => return false,
            Ok(Event::Eof) => return !first && depth == 0,
            _ => {}
        }
    }
}
pub fn parse_listing(body: &[u8], requested_prefix: &str) -> Result<Listing, Failure> {
    if body.len() > LIST_LIMIT {
        return Err(Failure::Invalid);
    }
    let text = std::str::from_utf8(body).map_err(|_| Failure::Invalid)?;
    if !safe_xml(text) {
        return Err(Failure::Invalid);
    }
    let xml: XmlListing = quick_xml::de::from_str(text).map_err(|_| Failure::Invalid)?;
    if xml.contents.len() + xml.common_prefixes.len() > 100
        || xml.is_truncated
            && (xml.next_continuation_token.is_none()
                || !list_input("", xml.next_continuation_token.as_deref()))
        || xml.encoding_type.as_deref().is_some_and(|v| v != "url")
    {
        return Err(Failure::Invalid);
    }
    let decode = |value: &str| -> Result<String, Failure> {
        let value = if xml.encoding_type.as_deref() == Some("url") {
            signing::decode(value)?
        } else {
            value.into()
        };
        super::domain::key(&value)?;
        if !value.starts_with(requested_prefix) {
            return Err(Failure::Invalid);
        }
        Ok(value)
    };
    let objects = xml
        .contents
        .iter()
        .map(|v| {
            Ok(Object {
                key: decode(&v.key)?,
                size: v.size,
                modified_at: v.last_modified.clone(),
            })
        })
        .collect::<Result<Vec<_>, Failure>>()?;
    let prefixes = xml
        .common_prefixes
        .iter()
        .map(|v| decode(&v.prefix))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Listing {
        objects,
        prefixes,
        next_token: if xml.is_truncated {
            xml.next_continuation_token
        } else {
            None
        },
    })
}

#[cfg(test)]
mod wire_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    async fn http(status: &str, body: &[u8], limit: usize) -> Result<Bytes, Failure> {
        let (client, mut server) = tokio::io::duplex(4096);
        let mut response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        response.extend_from_slice(body);
        tokio::spawn(async move {
            let mut request = [0u8; 4096];
            let _ = server.read(&mut request).await;
            let _ = server.write_all(&response).await;
        });
        exchange(
            client,
            Request::builder()
                .uri("/")
                .header("host", "fixture.test")
                .body(Full::new(Bytes::new()))
                .expect("request"),
            limit,
        )
        .await
    }
    #[tokio::test]
    async fn bounded_http_does_not_follow_redirects_or_expose_error_body() {
        assert!(matches!(
            http("302 Found", b"sensitive upstream text", 1024).await,
            Err(Failure::Provider(302))
        ));
        assert!(matches!(
            http("403 Forbidden", b"sensitive upstream text", 1024).await,
            Err(Failure::Provider(403))
        ));
        assert_eq!(
            &http("200 OK", b"fixture", 1024).await.expect("body")[..],
            b"fixture"
        );
    }
    #[tokio::test]
    async fn oversized_or_incomplete_http_is_not_a_success() {
        assert!(http("200 OK", &vec![0; 8192], 1024).await.is_err());
        assert!(http("202 Accepted", b"pending", 1024).await.is_err());
        assert!(http("206 Partial Content", b"part", 1024).await.is_err());
    }
}
