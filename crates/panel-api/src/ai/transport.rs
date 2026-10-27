//! Fixed-origin HTTPS, system CA verification, bounded HTTP/1, no redirects/proxy.
use super::domain::models;
use axum::body::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::{Request, StatusCode};
use hyper_util::rt::TokioIo;
use panel_proto::{AiProvider, AiSecret};
use serde_json::Value;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::{TlsConnector, rustls::pki_types::ServerName};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderError {
    Credentials,
    Limit,
    Unavailable,
    Invalid,
    Timeout,
}

struct ConnectionTask(tokio::task::JoinHandle<()>);
impl Drop for ConnectionTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub async fn post(
    provider: AiProvider,
    model: &str,
    key: &AiSecret,
    body: Value,
) -> Result<Value, ProviderError> {
    if !models(provider).contains(&model) || !panel_proto::valid_ai_key(&key.0) {
        return Err(ProviderError::Invalid);
    }
    tokio::time::timeout(Duration::from_secs(40), async {
        let (host, path, header, auth) = match provider {
            AiProvider::Gemini => (
                "generativelanguage.googleapis.com",
                format!("/v1beta/models/{model}:generateContent"),
                "x-goog-api-key",
                key.0.clone(),
            ),
            AiProvider::Openai => (
                "api.openai.com",
                "/v1/responses".into(),
                "authorization",
                format!("Bearer {}", key.0),
            ),
        };
        let socket = tokio::net::TcpStream::connect((host, 443))
            .await
            .map_err(|_| ProviderError::Unavailable)?;
        let name = ServerName::try_from(host).map_err(|_| ProviderError::Invalid)?;
        let tls = TlsConnector::from(
            crate::mail::system_trust().map_err(|_| ProviderError::Unavailable)?,
        )
        .connect(name, socket)
        .await
        .map_err(|_| ProviderError::Unavailable)?;
        let bytes = serde_json::to_vec(&body).map_err(|_| ProviderError::Invalid)?;
        if bytes.len() > 24_576 {
            return Err(ProviderError::Invalid);
        }
        let mut authorization = hyper::header::HeaderValue::from_bytes(auth.as_bytes())
            .map_err(|_| ProviderError::Invalid)?;
        authorization.set_sensitive(true);
        let request = Request::builder()
            .method("POST")
            .uri(path)
            .header("host", host)
            .header(header, authorization)
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .body(Full::new(Bytes::from(bytes)))
            .map_err(|_| ProviderError::Invalid)?;
        exchange(tls, request).await
    })
    .await
    .map_err(|_| ProviderError::Timeout)?
}

async fn exchange<T>(io: T, request: Request<Full<Bytes>>) -> Result<Value, ProviderError>
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(io))
        .await
        .map_err(|_| ProviderError::Unavailable)?;
    let _connection = ConnectionTask(tokio::spawn(async move {
        let _ = connection.await;
    }));
    let response = sender
        .send_request(request)
        .await
        .map_err(|_| ProviderError::Unavailable)?;
    match response.status() {
        StatusCode::OK => {}
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => return Err(ProviderError::Credentials),
        StatusCode::TOO_MANY_REQUESTS => return Err(ProviderError::Limit),
        _ => return Err(ProviderError::Unavailable),
    }
    if !response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(';')
                .next()
                .is_some_and(|v| v.trim() == "application/json")
        })
    {
        return Err(ProviderError::Invalid);
    }
    let bytes = Limited::new(response.into_body(), 65_536)
        .collect()
        .await
        .map_err(|_| ProviderError::Invalid)?
        .to_bytes();
    serde_json::from_slice(&bytes).map_err(|_| ProviderError::Invalid)
}

#[cfg(test)]
mod tests;
