//! Safe plain text MIME construction shared by SMTP and local submission.
use base64::prelude::*;
#[derive(Debug, Clone)]
pub struct EmailMessage {
    pub to: String,
    pub subject: String,
    pub body: String,
}
fn address(value: &str) -> Result<(), String> {
    crate::validate_email_address(value).map_err(|e| e.to_string())?;
    if value.len() > 254
        || value
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || matches!(c, '<' | '>' | '"'))
    {
        return Err("Alamat email tidak valid".into());
    }
    Ok(())
}
fn encoded_header(value: &str) -> String {
    let mut chunks = Vec::new();
    let mut chunk = String::new();
    for ch in value.chars() {
        if chunk.len() + ch.len_utf8() > 36 {
            chunks.push(BASE64_STANDARD.encode(&chunk));
            chunk.clear();
        }
        chunk.push(ch);
    }
    if !chunk.is_empty() {
        chunks.push(BASE64_STANDARD.encode(chunk));
    }
    chunks
        .into_iter()
        .map(|c| format!("=?UTF-8?B?{c}?="))
        .collect::<Vec<_>>()
        .join("\r\n ")
}
pub fn build_message(from: &str, name: &str, msg: &EmailMessage) -> Result<String, String> {
    address(from)?;
    address(&msg.to)?;
    if name.len() > 128
        || msg.subject.len() > 512
        || name.chars().any(char::is_control)
        || msg.subject.chars().any(char::is_control)
        || msg.body.len() > 1024 * 1024
    {
        return Err("Nama, subjek, atau isi email tidak valid/terlalu panjang".into());
    }
    let body = BASE64_STANDARD.encode(msg.body.as_bytes());
    let body = body
        .as_bytes()
        .chunks(76)
        .map(|c| String::from_utf8_lossy(c))
        .collect::<Vec<_>>()
        .join("\r\n");
    Ok(format!(
        "From: {} <{from}>\r\nTo: <{}>\r\nSubject: {}\r\nDate: {}\r\nMessage-ID: <{}@qpanel.local>\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=UTF-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{body}",
        encoded_header(name),
        msg.to,
        encoded_header(&msg.subject),
        chrono::Utc::now().to_rfc2822(),
        uuid::Uuid::new_v4()
    ))
}
