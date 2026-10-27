use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn mock_http(status: &str, content_type: &str, body: &str) -> Result<Value, ProviderError> {
    let (client, mut server) = tokio::io::duplex(4096);
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    tokio::spawn(async move {
        let mut incoming = [0u8; 4096];
        let _ = server.read(&mut incoming).await;
        let _ = server.write_all(response.as_bytes()).await;
    });
    let request = Request::builder()
        .method("POST")
        .uri("/fixture")
        .header("host", "fixture.invalid")
        .body(Full::new(Bytes::new()))
        .expect("request");
    exchange(client, request).await
}
#[tokio::test]
async fn bounded_http_accepts_json_and_rejects_redirects_auth_failures_and_non_json() {
    assert_eq!(
        mock_http("200 OK", "application/json; charset=utf-8", "{\"ok\":true}")
            .await
            .expect("json")["ok"],
        true
    );
    assert_eq!(
        mock_http("302 Found", "application/json", "{}").await,
        Err(ProviderError::Unavailable)
    );
    assert_eq!(
        mock_http(
            "401 Unauthorized",
            "application/json",
            "{\"secret_error\":\"not forwarded\"}"
        )
        .await,
        Err(ProviderError::Credentials)
    );
    assert_eq!(
        mock_http("429 Too Many Requests", "application/json", "{}").await,
        Err(ProviderError::Limit)
    );
    assert_eq!(
        mock_http("200 OK", "text/html", "<html>error</html>").await,
        Err(ProviderError::Invalid)
    );
    assert_eq!(
        mock_http("200 OK", "application/json", "malformed").await,
        Err(ProviderError::Invalid)
    );
}
#[tokio::test]
async fn oversized_http_body_is_rejected_without_collecting_it_all() {
    let huge = format!("\"{}\"", "a".repeat(70_000));
    assert_eq!(
        mock_http("200 OK", "application/json", &huge).await,
        Err(ProviderError::Invalid)
    );
}
