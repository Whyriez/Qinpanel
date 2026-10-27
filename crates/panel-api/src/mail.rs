use base64::prelude::*;
use std::{sync::Arc, time::Duration};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::{TlsConnector, rustls};

const SMTP_TIMEOUT: Duration = Duration::from_secs(10);
#[derive(Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub encryption: String,
    pub from_address: String,
    pub from_name: String,
}
impl std::fmt::Debug for SmtpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmtpConfig")
            .field("host", &self.host)
            .field("encryption", &self.encryption)
            .field("password", &"[redacted]")
            .finish_non_exhaustive()
    }
}
pub use panel_core::email::{EmailMessage, build_message};
pub struct SmtpClient;
impl SmtpClient {
    pub async fn test_connection(config: &SmtpConfig) -> Result<(), String> {
        timeout(Duration::from_secs(45), async {
            let mut session = SmtpSession::connect(config).await?;
            session.command("QUIT", &[221]).await.map(|_| ())
        })
        .await
        .map_err(|_| "Uji SMTP melewati batas waktu".to_string())?
    }
    pub async fn send_mail(config: &SmtpConfig, msg: &EmailMessage) -> Result<(), String> {
        let mime = build_message(&config.from_address, &config.from_name, msg)?;
        timeout(Duration::from_secs(60), async {
            let mut session = SmtpSession::connect(config).await?;
            session
                .command(&format!("MAIL FROM:<{}>", config.from_address), &[250])
                .await?;
            session
                .command(&format!("RCPT TO:<{}>", msg.to), &[250, 251])
                .await?;
            session.command("DATA", &[354]).await?;
            session.write(&format!("{mime}\r\n.\r\n")).await?;
            session.reply(&[250]).await?;
            // Acceptance is final; a failed QUIT must never cause duplicate delivery.
            let _ = session.command("QUIT", &[221]).await;
            Ok(())
        })
        .await
        .map_err(|_| {
            "Pengiriman SMTP melewati batas waktu; periksa antrean sebelum mengirim ulang"
                .to_string()
        })?
    }
}
trait SmtpIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> SmtpIo for T {}
type Stream = Box<dyn SmtpIo>;
struct SmtpSession {
    reader: BufReader<Stream>,
}
impl SmtpSession {
    async fn connect(config: &SmtpConfig) -> Result<Self, String> {
        Self::connect_with_trust(config, None).await
    }
    async fn connect_with_trust(
        config: &SmtpConfig,
        trust: Option<Arc<rustls::ClientConfig>>,
    ) -> Result<Self, String> {
        validate_config(config)?;
        let socket = timeout(
            SMTP_TIMEOUT,
            TcpStream::connect((config.host.as_str(), config.port)),
        )
        .await
        .map_err(|_| "Koneksi SMTP timeout")?
        .map_err(|_| "Tidak dapat terhubung ke SMTP")?;
        let mut stream: Stream = Box::new(socket);
        if config.encryption == "tls" {
            stream = tls(stream, &config.host, trust.clone()).await?;
        }
        let mut session = Self {
            reader: BufReader::new(stream),
        };
        session.reply(&[220]).await?;
        let capabilities = session.command("EHLO qpanel.local", &[250]).await?;
        if config.encryption == "starttls" {
            if !capabilities.lines().any(|line| {
                line.get(4..)
                    .is_some_and(|v| v.trim().eq_ignore_ascii_case("STARTTLS"))
            }) {
                return Err("Server tidak menawarkan STARTTLS; autentikasi dibatalkan".into());
            }
            session.command("STARTTLS", &[220]).await?;
            if !session.reader.buffer().is_empty() {
                return Err("Balasan STARTTLS tidak valid".into());
            }
            session.reader =
                BufReader::new(tls(session.reader.into_inner(), &config.host, trust).await?);
            session.command("EHLO qpanel.local", &[250]).await?;
        }
        if let Some(username) = config.username.as_ref().filter(|s| !s.is_empty()) {
            session.command("AUTH LOGIN", &[334]).await?;
            session
                .command(&BASE64_STANDARD.encode(username), &[334])
                .await?;
            session
                .command(
                    &BASE64_STANDARD.encode(config.password.as_deref().unwrap_or_default()),
                    &[235],
                )
                .await?;
        }
        Ok(session)
    }
    async fn write(&mut self, value: &str) -> Result<(), String> {
        timeout(
            SMTP_TIMEOUT,
            self.reader.get_mut().write_all(value.as_bytes()),
        )
        .await
        .map_err(|_| "SMTP timeout".to_string())?
        .map_err(|_| "Gagal menulis SMTP".to_string())
    }
    async fn command(&mut self, value: &str, codes: &[u16]) -> Result<String, String> {
        self.write(&format!("{value}\r\n")).await?;
        self.reply(codes).await
    }
    async fn reply(&mut self, codes: &[u16]) -> Result<String, String> {
        timeout(SMTP_TIMEOUT, async {
            let mut result = String::new();
            let mut first = None;
            for _ in 0..128 {
                let mut line = Vec::new();
                loop {
                    let byte = self
                        .reader
                        .read_u8()
                        .await
                        .map_err(|_| "Koneksi SMTP terputus")?;
                    line.push(byte);
                    if line.len() > 8192 {
                        return Err("Balasan SMTP terlalu panjang".to_string());
                    }
                    if byte == b'\n' {
                        break;
                    }
                }
                if line.len() < 5
                    || !line[..3].iter().all(u8::is_ascii_digit)
                    || !matches!(line[3], b' ' | b'-')
                {
                    return Err("Format balasan SMTP tidak valid".into());
                }
                let code = u16::from(line[0] - b'0') * 100
                    + u16::from(line[1] - b'0') * 10
                    + u16::from(line[2] - b'0');
                if first.is_some_and(|v| v != code) || !codes.contains(&code) {
                    return Err(format!("SMTP menolak operasi (kode {code})"));
                }
                first = Some(code);
                result
                    .push_str(std::str::from_utf8(&line).map_err(|_| "Balasan SMTP bukan UTF-8")?);
                if line[3] == b' ' {
                    return Ok(result);
                }
            }
            Err("Terlalu banyak baris balasan SMTP".into())
        })
        .await
        .map_err(|_| "Timeout membaca SMTP".to_string())?
    }
}
pub(crate) fn system_trust() -> Result<Arc<rustls::ClientConfig>, String> {
    let mut roots = rustls::RootCertStore::empty();
    for cert in rustls_native_certs::load_native_certs().certs {
        roots
            .add(cert)
            .map_err(|_| "Sertifikat CA lokal tidak valid")?;
    }
    if roots.is_empty() {
        return Err("CA tepercaya belum tersedia pada server".into());
    }
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|_| "Konfigurasi TLS gagal")?
    .with_root_certificates(roots)
    .with_no_client_auth();
    Ok(Arc::new(config))
}
async fn tls(
    stream: Stream,
    host: &str,
    trust: Option<Arc<rustls::ClientConfig>>,
) -> Result<Stream, String> {
    let config = match trust {
        Some(config) => config,
        None => system_trust()?,
    };
    let name = rustls::pki_types::ServerName::try_from(host.to_owned())
        .map_err(|_| "Hostname TLS tidak valid")?;
    let stream = timeout(
        SMTP_TIMEOUT,
        TlsConnector::from(config).connect(name, stream),
    )
    .await
    .map_err(|_| "Negosiasi TLS timeout")?
    .map_err(|_| "TLS SMTP gagal; periksa sertifikat dan hostname")?;
    Ok(Box::new(stream))
}
pub fn validate_config(config: &SmtpConfig) -> Result<(), String> {
    if config.host.is_empty()
        || config.host.len() > 253
        || config
            .host
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || matches!(c, '/' | '\\'))
        || config.port == 0
    {
        return Err("Host atau port SMTP tidak valid".into());
    }
    match config.encryption.as_str() {
        "tls" | "starttls" => {}
        "none"
            if config.host == "localhost"
                || config
                    .host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback()) => {}
        _ => {
            return Err("Gunakan TLS/STARTTLS; koneksi tanpa enkripsi hanya untuk loopback".into());
        }
    }
    if config.username.as_ref().is_some_and(|s| s.len() > 1024)
        || config.password.as_ref().is_some_and(|s| s.len() > 4096)
    {
        return Err("Kredensial SMTP terlalu panjang".into());
    }
    Ok(())
}
#[cfg(test)]
#[path = "mail_tests.rs"]
mod tests;
