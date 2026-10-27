use super::*;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn config(port: u16, encryption: &str) -> SmtpConfig {
    SmtpConfig {
        host: "127.0.0.1".into(),
        port,
        encryption: encryption.into(),
        username: Some("username".into()),
        password: Some("super-private".into()),
        from_address: "panel@example.test".into(),
        from_name: "Qinpanel".into(),
    }
}
#[tokio::test]
async fn accepted_data_is_success_even_when_quit_connection_is_lost() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(socket);
        reader
            .get_mut()
            .write_all(b"220 fixture\r\n")
            .await
            .unwrap();
        let mut line = String::new();
        let mut data = false;
        loop {
            line.clear();
            if reader.read_line(&mut line).await.unwrap() == 0 {
                break;
            }
            let reply = if data {
                if line == ".\r\n" {
                    reader
                        .get_mut()
                        .write_all(b"250 Accepted\r\n")
                        .await
                        .unwrap();
                    break;
                } else {
                    continue;
                }
            } else if line == "DATA\r\n" {
                data = true;
                "354 Send data\r\n"
            } else {
                "250 OK\r\n"
            };
            reader.get_mut().write_all(reply.as_bytes()).await.unwrap();
        }
    });
    let mut c = config(port, "none");
    c.username = None;
    c.password = None;
    SmtpClient::send_mail(
        &c,
        &EmailMessage {
            to: "recipient@example.net".into(),
            subject: "Fixture".into(),
            body: "Accepted once".into(),
        },
    )
    .await
    .unwrap();
    server.await.unwrap();
}
#[test]
fn refuses_remote_plaintext_and_does_not_debug_passwords() {
    let mut c = config(25, "none");
    c.host = "smtp.example.com".into();
    assert!(validate_config(&c).is_err());
    c.encryption = "starttls".into();
    assert!(validate_config(&c).is_ok());
    assert!(!format!("{c:?}").contains("super-private"));
}
#[test]
fn rejects_header_injection_and_encodes_unicode_and_dot_lines() {
    let mut msg = EmailMessage {
        to: "user@example.com".into(),
        subject: "Halo café".into(),
        body: "Hello\n.\n秘密".into(),
    };
    let mime = build_message("panel@example.com", "Panel", &msg).unwrap();
    assert!(mime.contains("=?UTF-8?B?"));
    assert!(mime.contains("Content-Transfer-Encoding: base64"));
    let body = mime.split("\r\n\r\n").nth(1).unwrap().replace("\r\n", "");
    assert_eq!(BASE64_STANDARD.decode(body).unwrap(), msg.body.as_bytes());
    msg.subject = "Hello\r\nBcc: intruder@example.com".into();
    assert!(build_message("panel@example.com", "Panel", &msg).is_err());
}
#[tokio::test]
async fn missing_starttls_never_sends_auth() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        socket.write_all(b"220 fixture\r\n").await.unwrap();
        let mut reader = BufReader::new(socket);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        assert!(line.starts_with("EHLO"));
        reader
            .get_mut()
            .write_all(b"250 AUTH LOGIN\r\n")
            .await
            .unwrap();
        line.clear();
        let read = reader.read_line(&mut line).await.unwrap();
        assert_eq!(read, 0);
    });
    assert!(
        SmtpClient::test_connection(&config(port, "starttls"))
            .await
            .unwrap_err()
            .contains("STARTTLS")
    );
    server.await.unwrap();
}
#[tokio::test]
async fn rejects_unbounded_banner_without_allocating_an_unbounded_line() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let _ = socket.write_all(&vec![b'A'; 9000]).await;
    });
    assert!(
        SmtpClient::test_connection(&config(port, "none"))
            .await
            .unwrap_err()
            .contains("panjang")
    );
    server.await.unwrap();
}
#[tokio::test]
async fn starttls_reissues_ehlo_and_authenticates_inside_verified_tls() {
    use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
    let cert = rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).unwrap();
    let cert_der = CertificateDer::from(cert.cert.der().to_vec());
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let server_config = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert_der.clone()],
            PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der()).into(),
        )
        .unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert_der).unwrap();
    let trust = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        socket.write_all(b"220 fixture\r\n").await.unwrap();
        let mut reader = BufReader::new(socket);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        assert!(line.starts_with("EHLO"));
        reader
            .get_mut()
            .write_all(b"250-fixture\r\n250 STARTTLS\r\n")
            .await
            .unwrap();
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        assert_eq!(line, "STARTTLS\r\n");
        reader
            .get_mut()
            .write_all(b"220 Upgrade\r\n")
            .await
            .unwrap();
        let socket = tokio_rustls::TlsAcceptor::from(Arc::new(server_config))
            .accept(reader.into_inner())
            .await
            .unwrap();
        let mut reader = BufReader::new(socket);
        for (expected, response) in [
            ("EHLO qpanel.local\r\n", "250 AUTH LOGIN\r\n"),
            ("AUTH LOGIN\r\n", "334 Username\r\n"),
            ("dXNlcm5hbWU=\r\n", "334 Password\r\n"),
            ("c3VwZXItcHJpdmF0ZQ==\r\n", "235 Accepted\r\n"),
        ] {
            line.clear();
            reader.read_line(&mut line).await.unwrap();
            assert_eq!(line, expected);
            reader
                .get_mut()
                .write_all(response.as_bytes())
                .await
                .unwrap();
        }
    });
    SmtpSession::connect_with_trust(&config(port, "starttls"), Some(Arc::new(trust)))
        .await
        .unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn test_smtp_mock_server_flow() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let local_addr = listener.local_addr().expect("local addr");

    // Spawn mock SMTP server in background
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let (r, mut w) = stream.split();
            let mut reader = BufReader::new(r);
            let _ = w
                .write_all(b"220 mock.smtp.local Qinpanel SMTP Test\r\n")
                .await;

            let mut line = String::new();
            while let Ok(n) = reader.read_line(&mut line).await {
                if n == 0 {
                    break;
                }
                let trimmed = line.trim();
                if trimmed.starts_with("EHLO") {
                    let _ = w
                        .write_all(b"250-mock.smtp.local\r\n250-AUTH LOGIN\r\n250 8BITMIME\r\n")
                        .await;
                } else if trimmed == "AUTH LOGIN" {
                    let _ = w.write_all(b"334 VXNlcm5hbWU6\r\n").await;
                } else if trimmed == "dXNlcm5hbWU=" {
                    // "username"
                    let _ = w.write_all(b"334 UGFzc3dvcmQ6\r\n").await;
                } else if trimmed == "cGFzc3dvcmQ=" {
                    // "password"
                    let _ = w
                        .write_all(b"235 2.7.0 Authentication successful\r\n")
                        .await;
                } else if trimmed.starts_with("MAIL FROM:") {
                    let _ = w.write_all(b"250 2.1.0 Sender OK\r\n").await;
                } else if trimmed.starts_with("RCPT TO:") {
                    let _ = w.write_all(b"250 2.1.5 Recipient OK\r\n").await;
                } else if trimmed == "DATA" {
                    let _ = w
                        .write_all(b"354 Start mail input; end with <CRLF>.<CRLF>\r\n")
                        .await;
                } else if trimmed == "." {
                    let _ = w.write_all(b"250 2.0.0 OK queued\r\n").await;
                } else if trimmed == "QUIT" {
                    let _ = w.write_all(b"221 2.0.0 Bye\r\n").await;
                    break;
                }
                line.clear();
            }
        }
    });

    let config = SmtpConfig {
        host: "127.0.0.1".to_string(),
        port: local_addr.port(),
        username: Some("username".to_string()),
        password: Some("password".to_string()),
        encryption: "none".to_string(),
        from_address: "panel@example.test".to_string(),
        from_name: "Qinpanel".to_string(),
    };

    // Test connection
    let conn_res = SmtpClient::test_connection(&config).await;
    assert!(
        conn_res.is_ok(),
        "Test connection failed: {:?}",
        conn_res.err()
    );

    // Test sending mail
    let send_res = SmtpClient::send_mail(
        &config,
        &EmailMessage {
            to: "admin@example.test".to_string(),
            subject: "[Test] Alert".to_string(),
            body: "This is a test notification message.".to_string(),
        },
    )
    .await;
    assert!(send_res.is_ok(), "Send mail failed: {:?}", send_res.err());
}
