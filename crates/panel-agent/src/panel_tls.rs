//! Root-only, fixed-path panel TLS. Never uses tenant certificate paths.
use crate::{AgentError, system::SystemRunner};
use panel_proto::{PanelTlsRequest, PanelTlsStatus};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
const CONFIG: &str = "/etc/nginx/conf.d/10-qpanel-panel-tls.conf";
const MANIFEST: &str = "/etc/panel/panel-tls/active.json";
const ACME_CONFIG: &str = "/etc/nginx/conf.d/05-qpanel-panel-acme.conf";
const CHALLENGE: &str = "/var/www/acme-challenge/.well-known/acme-challenge";
fn challenge_config(ip: std::net::IpAddr) -> String {
    let name = match ip {
        std::net::IpAddr::V6(ip) => format!("[{ip}]"),
        std::net::IpAddr::V4(ip) => ip.to_string(),
    };
    format!(
        "# Managed by QonPanel: panel IP HTTP-01 only.\nserver {{\n listen 80;\n listen [::]:80;\n server_name {name};\n location ^~ /.well-known/acme-challenge/ {{\n root /var/www/acme-challenge;\n default_type text/plain;\n try_files $uri =404;\n }}\n location / {{ return 444; }}\n}}\n"
    )
}
#[derive(Serialize, Deserialize)]
struct Manifest {
    status: PanelTlsStatus,
    config: String,
    key_fingerprint: String,
}
fn invalid(message: &str) -> AgentError {
    AgentError::InvalidInput(message.into())
}
fn atomic(r: &dyn SystemRunner, path: &str, data: &[u8], mode: u32) -> Result<(), AgentError> {
    let temp = format!("{path}.candidate");
    r.write_file(Path::new(&temp), data, mode)?;
    r.set_ownership(Path::new(&temp), 0, 0)?;
    r.move_path(Path::new(&temp), Path::new(path))?;
    crate::platform::relabel(r, Path::new(path))
}
fn checked(r: &dyn SystemRunner, binary: &str, args: &[&str]) -> Result<(), AgentError> {
    if !r.run_command(binary, args)?.status.success() {
        return Err(invalid(
            "Syntax check atau reload Nginx gagal; periksa diagnostik server.",
        ));
    }
    Ok(())
}
fn restore(r: &dyn SystemRunner, path: &str, old: Option<&[u8]>) -> Result<(), AgentError> {
    if let Some(old) = old {
        atomic(r, path, old, 0o644)?;
    } else if r.path_exists(Path::new(path)) {
        r.remove_file(Path::new(path))?;
    }
    checked(r, "nginx", &["-t"])?;
    checked(r, "systemctl", &["reload", "nginx"])
}
fn switch(r: &dyn SystemRunner, path: &str, new: &[u8]) -> Result<(), AgentError> {
    let old = if r.path_exists(Path::new(path)) {
        Some(r.read_file(Path::new(path))?)
    } else {
        None
    };
    atomic(r, path, new, 0o644)?;
    if checked(r, "nginx", &["-t"])
        .and_then(|()| checked(r, "systemctl", &["reload", "nginx"]))
        .is_err()
    {
        restore(r,path,old.as_deref()).map_err(|_| invalid("Aktivasi gagal dan rollback memerlukan pemeriksaan Nginx. Sertifikat sebelumnya tetap disimpan."))?;
        return Err(invalid(
            "Aktivasi TLS gagal; konfigurasi sebelumnya dipulihkan.",
        ));
    }
    Ok(())
}
pub fn inspect(r: &dyn SystemRunner) -> Result<PanelTlsStatus, AgentError> {
    if !r.path_exists(Path::new(MANIFEST)) {
        if r.path_exists(Path::new(CONFIG)) {
            return Err(invalid(
                "Konfigurasi TLS tanpa manifest; rekonsiliasi diperlukan.",
            ));
        }
        return Ok(PanelTlsStatus::default());
    }
    let m: Manifest = serde_json::from_slice(&r.read_file(Path::new(MANIFEST))?)
        .map_err(|_| invalid("Manifest TLS tidak valid."))?;
    if r.read_file(Path::new(CONFIG))? != m.config.as_bytes() {
        return Err(invalid(
            "Konfigurasi TLS berubah di luar panel; tidak ditimpa.",
        ));
    }
    let id = m
        .status
        .active_id
        .as_deref()
        .filter(|id| uuid::Uuid::parse_str(id).is_ok_and(|value| value.to_string() == *id))
        .ok_or_else(|| invalid("Identitas manifest tidak valid."))?;
    let cert = r.read_file(Path::new(&format!(
        "/etc/panel/panel-tls/{id}/fullchain.pem"
    )))?;
    let key = r.read_file(Path::new(&format!("/etc/panel/panel-tls/{id}/privkey.pem")))?;
    if m.status.fingerprint.as_deref() != Some(&format!("{:x}", Sha256::digest(&cert)))
        || m.key_fingerprint != format!("{:x}", Sha256::digest(&key))
    {
        return Err(invalid(
            "Berkas sertifikat aktif berubah di luar panel; rekonsiliasi diperlukan.",
        ));
    }
    Ok(m.status)
}
fn token_valid(token: &str) -> bool {
    (16..=128).contains(&token.len())
        && token
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
}
fn port_conflicts(dump: &str, port: u16, owned: bool) -> bool {
    let mut text = String::new();
    let mut skip = false;
    for line in dump.lines() {
        if let Some(file) = line
            .strip_prefix("# configuration file ")
            .and_then(|s| s.strip_suffix(':'))
        {
            skip = owned && file == CONFIG;
        }
        if !skip {
            text.push_str(line);
            text.push('\n');
        }
    }
    let mut words = Vec::<String>::new();
    let mut word = String::new();
    let mut quote = None;
    let mut escape = false;
    let mut comment = false;
    for c in text.chars() {
        if comment {
            if c == '\n' {
                comment = false;
            } else {
                continue;
            }
        }
        if escape {
            word.push(c);
            escape = false;
            continue;
        }
        if c == '\\' {
            escape = true;
            continue;
        }
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else {
                word.push(c);
            }
            continue;
        }
        if matches!(c, '\'' | '"') {
            quote = Some(c);
            continue;
        }
        if c == '#' {
            comment = true;
            continue;
        }
        if c.is_whitespace() || matches!(c, ';' | '{' | '}') {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            if matches!(c, ';' | '{' | '}') {
                if c == ';'
                    && words.first().is_some_and(|s| s == "listen")
                    && words.get(1).is_some_and(|s| {
                        s.rsplit_once(':')
                            .map_or(s.as_str(), |(_, p)| p)
                            .parse::<u16>()
                            .ok()
                            == Some(port)
                    })
                {
                    return true;
                }
                words.clear();
            }
        } else {
            word.push(c);
        }
    }
    false
}

/// Fixed loopback socket, exact already-validated certificate pin and a single
/// bounded TLS handshake. Public trust/identity is checked separately by API.
pub(crate) fn verify_listener(port: u16, certificate_pem: &str) -> Result<(), AgentError> {
    verify_listener_name(
        port,
        certificate_pem,
        tokio_rustls::rustls::pki_types::ServerName::IpAddress(
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST).into(),
        ),
    )
}
pub(crate) fn verify_listener_name(
    port: u16,
    certificate_pem: &str,
    name: tokio_rustls::rustls::pki_types::ServerName<'static>,
) -> Result<(), AgentError> {
    use std::{
        io::{Read, Write},
        net::{SocketAddr, TcpStream},
        sync::Arc,
        time::{Duration, Instant},
    };
    use tokio_rustls::rustls::{
        self, DigitallySignedStruct, SignatureScheme,
        client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
        pki_types::{CertificateDer, ServerName, UnixTime, pem::PemObject},
    };
    #[derive(Debug)]
    struct Pin(Vec<u8>);
    impl ServerCertVerifier for Pin {
        fn verify_server_cert(
            &self,
            end: &CertificateDer<'_>,
            _chain: &[CertificateDer<'_>],
            _name: &ServerName<'_>,
            _ocsp: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            if end.as_ref() == self.0.as_slice() {
                return Ok(ServerCertVerified::assertion());
            }
            Err(rustls::Error::General("Certificate pin mismatch".into()))
        }
        fn verify_tls12_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            signature: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            rustls::crypto::verify_tls12_signature(
                message,
                cert,
                signature,
                &rustls::crypto::ring::default_provider().signature_verification_algorithms,
            )
        }
        fn verify_tls13_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            signature: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            rustls::crypto::verify_tls13_signature(
                message,
                cert,
                signature,
                &rustls::crypto::ring::default_provider().signature_verification_algorithms,
            )
        }
        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            rustls::crypto::ring::default_provider()
                .signature_verification_algorithms
                .supported_schemes()
        }
    }
    struct Bounded {
        socket: TcpStream,
        deadline: Instant,
    }
    impl Bounded {
        fn remaining(&self) -> std::io::Result<Duration> {
            self.deadline
                .checked_duration_since(Instant::now())
                .filter(|d| !d.is_zero())
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::TimedOut, "TLS deadline"))
        }
    }
    impl Read for Bounded {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.socket.set_read_timeout(Some(self.remaining()?))?;
            self.socket.read(buf)
        }
    }
    impl Write for Bounded {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.socket.set_write_timeout(Some(self.remaining()?))?;
            self.socket.write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.socket.flush()
        }
    }
    let cert = CertificateDer::from_pem_slice(certificate_pem.as_bytes())
        .map_err(|_| invalid("PEM pin listener tidak valid."))?;
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|_| invalid("Konfigurasi TLS verifier gagal."))?
    .dangerous()
    .with_custom_certificate_verifier(Arc::new(Pin(cert.to_vec())))
    .with_no_client_auth();
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let socket = TcpStream::connect_timeout(&address, Duration::from_secs(2))
        .map_err(|_| invalid("Listener TLS belum menerima koneksi lokal."))?;
    let mut stream = Bounded {
        socket,
        deadline: Instant::now() + Duration::from_secs(3),
    };
    let mut connection = rustls::ClientConnection::new(Arc::new(config), name)
        .map_err(|_| invalid("Handshake TLS gagal disiapkan."))?;
    while connection.is_handshaking() {
        connection.complete_io(&mut stream).map_err(|_| {
            invalid("Listener belum menyajikan sertifikat kandidat atau handshake habis waktu.")
        })?;
    }
    Ok(())
}
pub fn execute(
    r: &dyn SystemRunner,
    request: PanelTlsRequest,
) -> Result<PanelTlsStatus, AgentError> {
    match request {
        PanelTlsRequest::Inspect => inspect(r),
        PanelTlsRequest::Challenge {
            ip,
            token,
            key_authorization,
        } => {
            if !token_valid(&token)
                || !key_authorization.starts_with(&format!("{token}."))
                || key_authorization.len() > 512
                || !key_authorization
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
            {
                return Err(invalid("Challenge ACME tidak valid."));
            }
            let config = challenge_config(ip);
            if r.path_exists(Path::new(ACME_CONFIG)) {
                let old = String::from_utf8(r.read_file(Path::new(ACME_CONFIG))?)
                    .map_err(|_| invalid("Konfigurasi challenge tidak valid."))?;
                let old_ip = old
                    .lines()
                    .find_map(|line| line.strip_prefix(" server_name "))
                    .and_then(|s| s.strip_suffix(';'))
                    .and_then(|s| s.trim_matches(['[', ']']).parse::<std::net::IpAddr>().ok())
                    .ok_or_else(|| {
                        invalid("Konfigurasi challenge bukan managed; tidak ditimpa.")
                    })?;
                if old != challenge_config(old_ip) {
                    return Err(invalid(
                        "Konfigurasi challenge diubah operator; tidak ditimpa.",
                    ));
                }
                if old != config {
                    switch(r, ACME_CONFIG, config.as_bytes())?;
                }
            } else {
                switch(r, ACME_CONFIG, config.as_bytes())?;
            }
            r.create_dir_all(Path::new(CHALLENGE), 0o755, 0, 0)?;
            atomic(
                r,
                &format!("{CHALLENGE}/{token}"),
                key_authorization.as_bytes(),
                0o644,
            )?;
            Ok(PanelTlsStatus::default())
        }
        PanelTlsRequest::Cleanup { token } => {
            if !token_valid(&token) {
                return Err(invalid("Token tidak valid."));
            }
            let path = format!("{CHALLENGE}/{token}");
            if r.path_exists(Path::new(&path)) {
                r.remove_file(Path::new(&path))?;
            }
            Ok(PanelTlsStatus::default())
        }
        PanelTlsRequest::Activate {
            id,
            ip,
            port,
            expected_fingerprint,
            fullchain_pem,
            private_key_pem,
        } => {
            if !uuid::Uuid::parse_str(&id).is_ok_and(|value| value.to_string() == id)
                || port < 1024
                || port == 7700
            {
                return Err(invalid("Identitas atau port TLS panel tidak valid."));
            }
            let public_port = r
                .read_file(Path::new("/etc/panel/public-port"))
                .ok()
                .and_then(|v| String::from_utf8(v).ok())
                .and_then(|v| v.trim().parse::<u16>().ok())
                .unwrap_or(17890);
            if port == public_port {
                return Err(invalid(
                    "Gunakan port TLS terpisah agar akses HTTP existing tetap tersedia.",
                ));
            }
            let now = chrono::Utc::now().timestamp();
            panel_core::panel_tls::validate_ip_candidate(&fullchain_pem, &private_key_pem, ip, now)
                .map_err(invalid)?;
            let fingerprint = format!("{:x}", Sha256::digest(fullchain_pem.as_bytes()));
            let current = inspect(r)?;
            if current.active_id.is_some() && (current.ip != Some(ip) || current.port != Some(port))
            {
                return Err(invalid(
                    "IP atau port listener aktif berbeda; siapkan transisi listener terpisah terlebih dahulu.",
                ));
            }
            if current.active_id.as_deref() == Some(&id)
                && current.fingerprint.as_deref() == Some(&fingerprint)
            {
                return Ok(current);
            }
            if current.fingerprint != expected_fingerprint {
                return Err(invalid(
                    "Sertifikat aktif berubah; muat ulang sebelum mengaktifkan kandidat.",
                ));
            }
            let dump = r.run_command("nginx", &["-T"])?;
            if !dump.status.success()
                || dump.stdout.is_empty()
                || dump.stdout.len() > 2 * 1024 * 1024
                || port_conflicts(
                    &String::from_utf8_lossy(&dump.stdout),
                    port,
                    current.active_id.is_some(),
                )
            {
                return Err(invalid(
                    "Port TLS dipakai konfigurasi Nginx lain atau inventaris listener belum dapat diperiksa.",
                ));
            }
            let dir = format!("/etc/panel/panel-tls/{id}");
            r.create_dir_all(Path::new(&dir), 0o700, 0, 0)?;
            for (name, bytes, mode) in [
                ("fullchain.pem", fullchain_pem.as_bytes(), 0o644),
                ("privkey.pem", private_key_pem.as_bytes(), 0o600),
            ] {
                let path = format!("{dir}/{name}");
                if r.path_exists(Path::new(&path)) && r.read_file(Path::new(&path))? != bytes {
                    return Err(invalid("Kandidat sertifikat immutable berbeda."));
                }
                atomic(r, &path, bytes, mode)?;
            }
            let config = format!(
                "# Managed by QonPanel: dedicated panel TLS, never tenant websites.\nserver {{\n listen {port} ssl;\n listen [::]:{port} ssl;\n server_name _;\n ssl_certificate {dir}/fullchain.pem;\n ssl_certificate_key {dir}/privkey.pem;\n ssl_protocols TLSv1.2 TLSv1.3;\n location / {{\n proxy_pass http://127.0.0.1:7700;\n proxy_http_version 1.1;\n proxy_set_header Host $http_host;\n proxy_set_header X-Forwarded-Proto $scheme;\n proxy_set_header X-Real-IP $remote_addr;\n proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;\n proxy_set_header Forwarded \"\";\n proxy_set_header X-Forwarded-Host \"\";\n proxy_set_header CF-Visitor \"\";\n proxy_set_header X-Qpanel-Cf-Visitor \"\";\n proxy_set_header Upgrade $http_upgrade;\n proxy_set_header Connection \"upgrade\";\n }}\n}}\n"
            );
            let old = if r.path_exists(Path::new(CONFIG)) {
                Some(r.read_file(Path::new(CONFIG))?)
            } else {
                None
            };
            switch(r, CONFIG, config.as_bytes())?;
            if r.verify_panel_tls(port, &fullchain_pem).is_err() {
                restore(r,CONFIG,old.as_deref()).map_err(|_|invalid("Verifikasi listener dan rollback gagal; periksa Nginx. Berkas sertifikat sebelumnya tetap disimpan."))?;
                return Err(invalid(
                    "Listener belum memakai sertifikat kandidat; konfigurasi sebelumnya dipulihkan.",
                ));
            }
            let status = PanelTlsStatus {
                active_id: Some(id),
                fingerprint: Some(fingerprint),
                ip: Some(ip),
                port: Some(port),
            };
            let manifest = serde_json::to_vec(&Manifest {
                status: status.clone(),
                config,
                key_fingerprint: format!("{:x}", Sha256::digest(private_key_pem.as_bytes())),
            })
            .map_err(|_| invalid("Manifest TLS gagal disiapkan."))?;
            if atomic(r, MANIFEST, &manifest, 0o600).is_err() {
                restore(r, CONFIG, old.as_deref())?;
                return Err(invalid(
                    "Manifest gagal disimpan; konfigurasi sebelumnya dipulihkan.",
                ));
            }
            Ok(status)
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[test]
    fn listener_conflicts_include_ipv6_quotes_and_multiline_but_exclude_owned_tls_file() {
        for authority in ["19443", "127.0.0.1:19443", "[::]:19443", "\"[::]:19443\""] {
            assert!(port_conflicts(
                &format!("server {{ listen\n {authority} ssl; }}"),
                19443,
                false
            ));
        }
        assert!(!port_conflicts(
            "# listen 19443;\nserver {listen 443 ssl;}",
            19443,
            false
        ));
        let owned = format!(
            "# configuration file {CONFIG}:\nserver {{listen 19443 ssl;}}\n# configuration file /etc/nginx/conf.d/other.conf:\nserver {{listen 19444;}}"
        );
        assert!(!port_conflicts(&owned, 19443, true));
        assert!(port_conflicts(
            &owned.replace("19444", "19443"),
            19443,
            true
        ));
    }
    #[test]
    fn real_loopback_handshake_requires_the_exact_certificate_and_private_key_proof() {
        use std::{net::TcpListener, sync::Arc};
        use tokio_rustls::rustls::{
            self,
            pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
        };
        for (wrong, dns) in [(false, false), (true, false), (false, true), (true, true)] {
            let request = candidate("1.1.1.1", None);
            let PanelTlsRequest::Activate {
                fullchain_pem,
                private_key_pem,
                ..
            } = request
            else {
                panic!("fixture");
            };
            let expected = if wrong {
                let PanelTlsRequest::Activate { fullchain_pem, .. } = candidate("1.1.1.1", None)
                else {
                    panic!("fixture");
                };
                fullchain_pem
            } else {
                fullchain_pem.clone()
            };
            let cert = CertificateDer::from_pem_slice(fullchain_pem.as_bytes()).unwrap();
            let key = PrivateKeyDer::from_pem_slice(private_key_pem.as_bytes()).unwrap();
            let config = rustls::ServerConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(std::time::Duration::from_secs(3)))
                    .unwrap();
                let mut tls = rustls::ServerConnection::new(Arc::new(config)).unwrap();
                while tls.is_handshaking() {
                    if tls.complete_io(&mut socket).is_err() {
                        break;
                    }
                }
                if dns {
                    assert_eq!(tls.server_name(), Some("app.example.com"));
                }
            });
            let result = if dns {
                verify_listener_name(
                    port,
                    &expected,
                    rustls::pki_types::ServerName::try_from("app.example.com").unwrap(),
                )
            } else {
                verify_listener(port, &expected)
            };
            assert_eq!(result.is_ok(), !wrong);
            server.join().unwrap();
        }
    }
    #[test]
    fn unrelated_port_is_rejected_before_writing_candidate_or_changing_proxy() {
        let r = fixture_runner();
        r.set_response("nginx -T", 0, b"server {listen 19443;}\n", b"");
        assert!(execute(&r, candidate("1.1.1.1", None)).is_err());
        assert!(!r.path_exists(Path::new(CONFIG)));
        assert!(!r.path_exists(Path::new(MANIFEST)));
    }
    #[test]
    fn loopback_probe_has_a_total_handshake_deadline() {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(6)))
                .unwrap();
            let _ = socket.read_to_end(&mut Vec::new());
        });
        let PanelTlsRequest::Activate { fullchain_pem, .. } = candidate("1.1.1.1", None) else {
            panic!("fixture");
        };
        let start = std::time::Instant::now();
        assert!(verify_listener(port, &fullchain_pem).is_err());
        assert!(start.elapsed() < std::time::Duration::from_secs(8));
        server.join().unwrap();
    }
    use crate::system::{MockAction, MockSystemRunner};
    fn fixture_runner() -> MockSystemRunner {
        let r = MockSystemRunner::new();
        r.set_response("nginx -T",0,b"# configuration file /etc/nginx/nginx.conf:\nevents {} http { server {listen 17890;} }\n",b"");
        r
    }
    fn candidate(ip: &str, expected: Option<String>) -> PanelTlsRequest {
        let ip = ip.parse().unwrap();
        let mut params = rcgen::CertificateParams::default();
        params.subject_alt_names = vec![rcgen::SanType::IpAddress(ip)];
        params.not_before = time::OffsetDateTime::now_utc() - time::Duration::minutes(1);
        params.not_after = time::OffsetDateTime::now_utc() + time::Duration::hours(160);
        let key = rcgen::KeyPair::generate().unwrap();
        PanelTlsRequest::Activate {
            id: uuid::Uuid::new_v4().to_string(),
            ip,
            port: 19443,
            expected_fingerprint: expected,
            fullchain_pem: params.self_signed(&key).unwrap().pem(),
            private_key_pem: key.serialize_pem(),
        }
    }
    #[test]
    fn validated_candidate_is_private_scoped_idempotent_and_preserves_previous_files() {
        let r = fixture_runner();
        let first = candidate("1.1.1.1", None);
        let active = execute(&r, first.clone()).unwrap();
        assert_eq!(execute(&r, first).unwrap(), active);
        let config = r.read_file(Path::new(CONFIG)).unwrap();
        let config = String::from_utf8(config).unwrap();
        assert!(
            config.contains("listen 19443 ssl;")
                && config.contains("[::]:19443 ssl;")
                && config.contains("Host $http_host")
        );
        assert!(!r.path_exists(Path::new("/etc/nginx/sites-available/nusa-panel.conf")));
        assert!(r.recorded_actions().iter().any(|a|matches!(a,MockAction::WriteFile{path,mode,..} if path.to_string_lossy().contains("privkey.pem") && *mode==0o600)));
        let next = execute(&r, candidate("1.1.1.1", active.fingerprint.clone())).unwrap();
        assert_ne!(next.active_id, active.active_id);
        assert!(r.path_exists(Path::new(&format!(
            "/etc/panel/panel-tls/{}/privkey.pem",
            active.active_id.unwrap()
        ))));
    }
    #[test]
    fn failed_syntax_or_reload_restores_old_config_and_manifest() {
        for failed in ["nginx -t", "systemctl reload nginx", "panel tls verify"] {
            let r = fixture_runner();
            let active = execute(&r, candidate("2606:4700:4700::1111", None)).unwrap();
            let old = r.read_file(Path::new(CONFIG)).unwrap();
            let manifest = r.read_file(Path::new(MANIFEST)).unwrap();
            r.set_response(failed, 1, b"", b"fixture failure");
            assert!(
                execute(
                    &r,
                    candidate("2606:4700:4700::1111", active.fingerprint.clone())
                )
                .is_err()
            );
            assert_eq!(r.read_file(Path::new(CONFIG)).unwrap(), old);
            assert_eq!(r.read_file(Path::new(MANIFEST)).unwrap(), manifest);
            assert_eq!(inspect(&r).unwrap(), active);
        }
    }
    #[test]
    fn port_identity_and_custom_configuration_fail_closed() {
        let r = fixture_runner();
        for port in [80, 443, 7700, 17890] {
            let mut request = candidate("1.1.1.1", None);
            if let PanelTlsRequest::Activate { port: p, .. } = &mut request {
                *p = port;
            }
            assert!(execute(&r, request).is_err());
        }
        let active = execute(&r, candidate("1.1.1.1", None)).unwrap();
        assert!(execute(&r, candidate("1.1.1.1", None)).is_err());
        assert!(execute(&r, candidate("8.8.8.8", active.fingerprint)).is_err());
        r.set_file_content(CONFIG, b"operator custom configuration");
        assert!(inspect(&r).is_err());
        assert!(execute(&r, candidate("1.1.1.1", None)).is_err());
    }
    #[test]
    fn challenge_is_ip_specific_fixed_path_and_non_challenge_hosts_stay_rejected() {
        let r = fixture_runner();
        let token = "abcdefgh12345678";
        execute(
            &r,
            PanelTlsRequest::Challenge {
                ip: "1.1.1.1".parse().unwrap(),
                token: token.into(),
                key_authorization: format!("{token}.thumbprint"),
            },
        )
        .unwrap();
        let config = String::from_utf8(r.read_file(Path::new(ACME_CONFIG)).unwrap()).unwrap();
        assert!(
            config.contains("server_name 1.1.1.1;")
                && config.contains("location / { return 444; }")
                && config.contains("listen 80;")
        );
        assert!(!r.path_exists(Path::new("/etc/nginx/conf.d/00-qpanel-reject-unknown.conf")));
        assert!(r.path_exists(Path::new(&format!("{CHALLENGE}/{token}"))));
        execute(
            &r,
            PanelTlsRequest::Cleanup {
                token: token.into(),
            },
        )
        .unwrap();
        assert!(!r.path_exists(Path::new(&format!("{CHALLENGE}/{token}"))));
        assert!(
            execute(
                &r,
                PanelTlsRequest::Cleanup {
                    token: "../../secret".into()
                }
            )
            .is_err()
        );
        r.set_file_content(ACME_CONFIG, b"operator customized");
        assert!(
            execute(
                &r,
                PanelTlsRequest::Challenge {
                    ip: "8.8.8.8".parse().unwrap(),
                    token: token.into(),
                    key_authorization: format!("{token}.thumbprint")
                }
            )
            .is_err()
        );
    }
}
