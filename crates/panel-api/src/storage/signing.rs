//! AWS SigV4: canonical URI/query, signed payload/conditions and 60-second GETs.
use super::domain::{Connection, Failure};
use hmac::{Hmac, Mac};
use panel_proto::S3Credentials;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub fn encode(value: &str, preserve_slash: bool) -> String {
    let mut result = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric()
            || matches!(b, b'_' | b'-' | b'.' | b'~')
            || preserve_slash && b == b'/'
        {
            result.push(char::from(b));
        } else {
            result.push_str(&format!("%{b:02X}"));
        }
    }
    result
}
pub fn decode(value: &str) -> Result<String, Failure> {
    let mut bytes = Vec::new();
    let mut input = value.bytes();
    while let Some(b) = input.next() {
        if b == b'%' {
            let h = input
                .next()
                .and_then(|v| char::from(v).to_digit(16))
                .ok_or(Failure::Invalid)?;
            let l = input
                .next()
                .and_then(|v| char::from(v).to_digit(16))
                .ok_or(Failure::Invalid)?;
            bytes.push((h * 16 + l) as u8);
        } else {
            bytes.push(b);
        }
    }
    String::from_utf8(bytes).map_err(|_| Failure::Invalid)
}
pub fn query(values: &[(String, String)]) -> String {
    let mut parts: Vec<_> = values
        .iter()
        .map(|(k, v)| (encode(k, false), encode(v, false)))
        .collect();
    parts.sort();
    parts
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}
pub fn path(connection: &Connection, key: Option<&str>) -> String {
    let mut path = connection.base_path().to_owned();
    if connection.addressing == "path" {
        path.push('/');
        path.push_str(&connection.bucket);
    }
    if let Some(key) = key {
        path.push('/');
        path.push_str(&encode(key, true));
    }
    if path.is_empty() {
        path.push('/');
    }
    path
}
fn sha(value: &[u8]) -> String {
    panel_core::bytes_to_hex(&Sha256::digest(value))
}
fn hmac(key: &[u8], value: &str) -> Result<Vec<u8>, Failure> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).map_err(|_| Failure::Invalid)?;
    mac.update(value.as_bytes());
    Ok(mac.finalize().into_bytes().to_vec())
}
fn signature(secret: &str, date: &str, region: &str, text: &str) -> Result<String, Failure> {
    let date_key = hmac(format!("AWS4{secret}").as_bytes(), date)?;
    let region_key = hmac(&date_key, region)?;
    let service_key = hmac(&region_key, "s3")?;
    let signing_key = hmac(&service_key, "aws4_request")?;
    Ok(panel_core::bytes_to_hex(&hmac(&signing_key, text)?))
}
pub struct SignRequest<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub query: &'a str,
    pub body: &'a [u8],
    pub overwrite: bool,
}
pub fn headers(
    connection: &Connection,
    credentials: &S3Credentials,
    request: SignRequest<'_>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<BTreeMap<String, String>, Failure> {
    let SignRequest {
        method,
        path,
        query,
        body,
        overwrite,
    } = request;
    let date = now.format("%Y%m%d").to_string();
    let timestamp = now.format("%Y%m%dT%H%M%SZ").to_string();
    let mut headers = BTreeMap::from([
        (
            "host".into(),
            connection.request_host().map_err(|_| Failure::Invalid)?,
        ),
        ("x-amz-date".into(), timestamp.clone()),
        ("x-amz-content-sha256".into(), sha(body)),
    ]);
    if let Some(token) = &credentials.session_token {
        headers.insert("x-amz-security-token".into(), token.clone());
    }
    if method == "PUT" && !overwrite {
        headers.insert("if-none-match".into(), "*".into());
    }
    let canonical_headers = headers
        .iter()
        .map(|(k, v)| format!("{k}:{}\n", v.trim()))
        .collect::<String>();
    let signed = headers.keys().cloned().collect::<Vec<_>>().join(";");
    let canonical = format!(
        "{method}\n{path}\n{query}\n{canonical_headers}\n{signed}\n{}",
        sha(body)
    );
    let scope = format!("{date}/{}/s3/aws4_request", connection.region);
    let text = format!(
        "AWS4-HMAC-SHA256\n{timestamp}\n{scope}\n{}",
        sha(canonical.as_bytes())
    );
    let sig = signature(&credentials.secret_key, &date, &connection.region, &text)?;
    headers.insert(
        "authorization".into(),
        format!(
            "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={sig}",
            credentials.access_key
        ),
    );
    Ok(headers)
}
pub fn download(
    connection: &Connection,
    credentials: &S3Credentials,
    key: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<String, Failure> {
    if credentials.session_token.is_some() {
        return Err(Failure::Invalid);
    }
    let date = now.format("%Y%m%d").to_string();
    let timestamp = now.format("%Y%m%dT%H%M%SZ").to_string();
    let scope = format!("{date}/{}/s3/aws4_request", connection.region);
    let mut values = vec![
        ("X-Amz-Algorithm".into(), "AWS4-HMAC-SHA256".into()),
        (
            "X-Amz-Credential".into(),
            format!("{}/{scope}", credentials.access_key),
        ),
        ("X-Amz-Date".into(), timestamp.clone()),
        ("X-Amz-Expires".into(), "60".into()),
        ("X-Amz-SignedHeaders".into(), "host".into()),
        ("response-content-disposition".into(), "attachment".into()),
    ];
    let path = path(connection, Some(key));
    let host = connection.request_host().map_err(|_| Failure::Invalid)?;
    let canonical = format!(
        "GET\n{path}\n{}\nhost:{host}\n\nhost\nUNSIGNED-PAYLOAD",
        query(&values)
    );
    let text = format!(
        "AWS4-HMAC-SHA256\n{timestamp}\n{scope}\n{}",
        sha(canonical.as_bytes())
    );
    values.push((
        "X-Amz-Signature".into(),
        signature(&credentials.secret_key, &date, &connection.region, &text)?,
    ));
    Ok(format!("https://{host}{path}?{}", query(&values)))
}
