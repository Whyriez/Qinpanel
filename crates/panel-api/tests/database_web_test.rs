#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Temporary SQLite/router and typed RPC fixtures only; never starts AgentServer or PHP.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use panel_api::{
    auth::{CSRF_HEADER_NAME, LockoutManager, SESSION_COOKIE_NAME, generate_csrf_token},
    config::AuthConfig,
    routes::AppState,
};
use panel_db::{
    models::{CreateDatabaseInput, CreateSession, CreateUser},
    repositories::{SqliteDatabaseRepository, SqliteSessionRepository, SqliteUserRepository},
};
use panel_proto::{DatabaseWebCredentials, DatabaseWebStatus, RequestPayload, ResponsePayload};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

struct Fixture {
    state: AppState,
    app: axum::Router,
    cookies: Vec<String>,
    csrf: Vec<String>,
    ids: Vec<i64>,
    _tmp: tempfile::TempDir,
}
async fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp
        .path()
        .join("db.sqlite")
        .to_string_lossy()
        .replace('\\', "/");
    let pool = panel_db::pool::init_pool(&format!("sqlite://{path}"))
        .await
        .unwrap();
    panel_db::pool::run_migrations(&pool).await.unwrap();
    let mut cookies = vec![];
    let mut csrf = vec![];
    let mut ids = vec![];
    for (i, role) in ["admin", "user", "user"].iter().enumerate() {
        let user = SqliteUserRepository::new(pool.clone())
            .create(&CreateUser {
                username: format!("person{i}"),
                password_hash: "unused-in-session-fixture".into(),
                role: (*role).into(),
                email: None,
                plan_id: None,
                linux_username: if i > 0 {
                    Some(format!("p_user{i}"))
                } else {
                    None
                },
                linux_uid: if i > 0 { Some(10000 + i as i64) } else { None },
            })
            .await
            .unwrap();
        let session_id = format!("fixture-session-{i}");
        let token = format!("fixture-cookie-{i}");
        SqliteSessionRepository::new(pool.clone())
            .create(&CreateSession {
                id: session_id.clone(),
                user_id: user.id,
                token_hash: panel_core::hash_token(&token),
                ip_address: None,
                user_agent: None,
                expires_at: chrono::Utc::now().timestamp() + 3600,
            })
            .await
            .unwrap();
        cookies.push(format!("{SESSION_COOKIE_NAME}={token}"));
        csrf.push(generate_csrf_token(&session_id));
        if i > 0 {
            ids.push(
                SqliteDatabaseRepository::new(pool.clone())
                    .create(&CreateDatabaseInput {
                        user_id: user.id,
                        site_id: None,
                        engine: if i == 1 { "mariadb" } else { "postgresql" }.into(),
                        name: format!("p_user{i}_shop"),
                        db_user: format!("p_user{i}_app"),
                        remote_access: false,
                        allowed_hosts: "localhost".into(),
                        collation: "utf8mb4_unicode_ci".into(),
                        character_set: "utf8mb4".into(),
                    })
                    .await
                    .unwrap()
                    .id,
            );
        }
    }
    let state = AppState::new(
        Arc::new(panel_api::agent_client::AgentClient::new(
            tmp.path().join("rpc"),
        )),
        pool,
        "development".into(),
        AuthConfig {
            cookie_secure: Some(false),
            ..Default::default()
        },
        Arc::new(LockoutManager::new(5, 60)),
    );
    let app = panel_api::create_router(state.clone());
    Fixture {
        state,
        app,
        cookies,
        csrf,
        ids,
        _tmp: tmp,
    }
}
async fn call(
    f: &Fixture,
    actor: usize,
    csrf: bool,
    method: &str,
    path: &str,
) -> axum::response::Response {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("cookie", &f.cookies[actor]);
    if csrf {
        request = request.header(CSRF_HEADER_NAME, &f.csrf[actor]);
    }
    f.app
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}
async fn json_body(response: axum::response::Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

#[tokio::test]
async fn tools_install_requires_admin_csrf_deduplicates_and_running_job_cannot_cancel() {
    let f = fixture().await;
    let path = "/api/v1/databases/web-tools/install";
    assert_eq!(
        call(&f, 1, true, "POST", path).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&f, 0, false, "POST", path).await.status(),
        StatusCode::FORBIDDEN
    );
    let response = call(&f, 0, true, "POST", path).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let first = json_body(response).await;
    let second = json_body(call(&f, 0, true, "POST", path).await).await;
    assert_eq!(first["id"], second["id"]);
    assert_eq!(first["name"], "database.web.install");
    let id = first["id"].as_str().unwrap();
    sqlx::query("UPDATE jobs SET status='running' WHERE id=?")
        .bind(id)
        .execute(&f.state.operations_pool)
        .await
        .unwrap();
    assert!(matches!(
        f.state.job_runner.cancel(id).await,
        Err(panel_db::DbError::Conflict(_))
    ));
}

#[tokio::test]
async fn ownership_and_panel_auth_are_checked_before_any_rpc_or_tool_request() {
    let f = fixture().await;
    for tail in ["web-access", "web/", "web/themes/style.css"] {
        let method = if tail == "web-access" { "POST" } else { "GET" };
        let path = format!("/api/v1/databases/{}/{tail}", f.ids[1]);
        assert_eq!(
            call(&f, 1, true, method, &path).await.status(),
            StatusCode::FORBIDDEN
        );
    }
    let path = format!("/api/v1/databases/{}/web/", f.ids[0]);
    let response = f
        .app
        .clone()
        .oneshot(Request::builder().uri(&path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    sqlx::query(
        "DELETE FROM sessions WHERE user_id=(SELECT id FROM users WHERE username='person1')",
    )
    .execute(&f.state.pool)
    .await
    .unwrap();
    assert_eq!(
        call(&f, 1, false, "GET", &path).await.status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn suspended_tenant_missing_identity_and_csrf_fail_closed() {
    let f = fixture().await;
    let path = format!("/api/v1/databases/{}/web-access", f.ids[0]);
    assert_eq!(
        call(&f, 1, false, "POST", &path).await.status(),
        StatusCode::FORBIDDEN
    );
    sqlx::query("UPDATE users SET linux_uid=NULL WHERE username='person1'")
        .execute(&f.state.pool)
        .await
        .unwrap();
    assert_eq!(
        call(&f, 0, true, "POST", &path).await.status(),
        StatusCode::BAD_REQUEST
    );
    sqlx::query("UPDATE users SET status='suspended' WHERE username='person1'")
        .execute(&f.state.pool)
        .await
        .unwrap();
    assert_eq!(
        call(&f, 0, true, "POST", &path).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&f, 0, true, "POST", "/api/v1/databases/99999/web-access")
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn status_never_reports_ready_when_agent_is_unavailable() {
    let f = fixture().await;
    assert_eq!(
        call(&f, 1, false, "GET", "/api/v1/databases/web-tools")
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn install_worker_uses_typed_operation_and_persists_completion() {
    let f = fixture().await;
    let accepted = call(&f, 0, true, "POST", "/api/v1/databases/web-tools/install").await;
    let id = json_body(accepted).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let socket = f._tmp.path().join("rpc");
    #[cfg(unix)]
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    #[cfg(not(unix))]
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    #[cfg(not(unix))]
    std::fs::write(&socket, listener.local_addr().unwrap().port().to_string()).unwrap();
    let rpc = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = panel_proto::RpcStream::new(stream);
        let request = stream.recv_request().await.unwrap();
        assert!(matches!(
            request.payload,
            RequestPayload::InstallWithProgress(panel_proto::InstallOperation::DatabaseWebTools)
        ));
        stream
            .send_response(&panel_proto::RpcResponse::ok(
                &request.request_id,
                ResponsePayload::InstallationProgress(panel_proto::InstallationProgress {
                    message: "Fixture installation progress".into(),
                }),
            ))
            .await
            .unwrap();
        stream
            .send_response(&panel_proto::RpcResponse::ok(
                request.request_id,
                ResponsePayload::DatabaseWebStatus(DatabaseWebStatus {
                    installed: true,
                    ready: true,
                }),
            ))
            .await
            .unwrap();
    });
    assert!(
        panel_api::jobs::process_one_job(&f.state.job_runner)
            .await
            .unwrap()
    );
    rpc.await.unwrap();
    let result = f
        .state
        .job_runner
        .job_repo
        .find_by_id(&id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.status, "succeeded");
    assert_eq!(result.progress, 100);
    let logs = f
        .state
        .job_runner
        .job_repo
        .events_after(&id, 0)
        .await
        .unwrap();
    assert!(
        logs.iter()
            .any(|event| event.message.as_deref() == Some("Fixture installation progress"))
    );
}

#[tokio::test]
async fn failed_native_deletion_preserves_the_database_record() {
    let f = fixture().await;
    assert_eq!(
        call(
            &f,
            1,
            true,
            "DELETE",
            &format!("/api/v1/databases/{}", f.ids[0])
        )
        .await
        .status(),
        StatusCode::BAD_GATEWAY
    );
    assert!(
        SqliteDatabaseRepository::new(f.state.pool.clone())
            .find_by_id(f.ids[0])
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn launch_returns_internal_url_only_and_preserves_remote_access_setting() {
    let f = fixture().await;
    let socket = f._tmp.path().join("rpc");
    #[cfg(unix)]
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    #[cfg(not(unix))]
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    #[cfg(not(unix))]
    std::fs::write(&socket, listener.local_addr().unwrap().port().to_string()).unwrap();
    let rpc = tokio::spawn(async move {
        for _ in 0..4 {
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = panel_proto::RpcStream::new(stream);
            let request = stream.recv_request().await.unwrap();
            let response = match request.payload {
                RequestPayload::DatabaseWebStatus => {
                    ResponsePayload::DatabaseWebStatus(DatabaseWebStatus {
                        installed: true,
                        ready: true,
                    })
                }
                RequestPayload::DatabaseWebCredentials(req) => {
                    assert!(req.provision);
                    assert!(req.tenant_uid >= 10000);
                    assert!(req.name.starts_with(&format!("{}_", req.tenant_username)));
                    ResponsePayload::DatabaseWebCredentials(DatabaseWebCredentials {
                        username: "qpw_mock".into(),
                        password: "secret-only-in-rpc".into(),
                    })
                }
                _ => panic!("Unexpected typed request"),
            };
            stream
                .send_response(&panel_proto::RpcResponse::ok(request.request_id, response))
                .await
                .unwrap();
        }
    });
    for (index, tool) in [(0, "phpMyAdmin"), (1, "Adminer")] {
        let response = call(
            &f,
            index + 1,
            true,
            "POST",
            &format!("/api/v1/databases/{}/web-access", f.ids[index]),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store");
        let body = json_body(response).await;
        assert_eq!(
            body,
            json!({"url":format!("/api/v1/databases/{}/web/",f.ids[index]),"tool":tool})
        );
        assert!(!body.to_string().contains("secret-only-in-rpc"));
    }
    rpc.await.unwrap();
    let remote: i64 = sqlx::query_scalar("SELECT SUM(remote_access) FROM databases")
        .fetch_one(&f.state.pool)
        .await
        .unwrap();
    assert_eq!(remote, 0);
}

#[tokio::test]
async fn cross_origin_posts_are_rejected_without_forwarding() {
    let f = fixture().await;
    let response = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/databases/{}/web/", f.ids[0]))
                .header("cookie", &f.cookies[1])
                .header("host", "panel.test")
                .header("origin", "https://evil.test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn tunnel_bootstrap_posts_pass_origin_guard_before_private_backend_lookup() {
    let f = fixture().await;
    for query in ["route=/config/get", "route=/navigation&ajax_request=1"] {
        let response = f
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/v1/databases/{}/web/index.php?{query}",
                        f.ids[0]
                    ))
                    .extension(axum::extract::ConnectInfo(
                        "127.0.0.1:44000".parse::<std::net::SocketAddr>().unwrap(),
                    ))
                    .header("cookie", &f.cookies[1])
                    .header("host", "panel.test")
                    .header("origin", "https://panel.test")
                    .header("sec-fetch-site", "same-origin")
                    .header("x-forwarded-proto", "http")
                    .header("x-qpanel-cf-visitor", r#"{"scheme":"https"}"#)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // The fixture's private RPC path does not exist: reaching its credential
        // lookup proves the public HTTPS POST passed both origin and ownership.
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(
            json_body(response).await,
            json!({"error":"Buka kembali pengelola melalui tombol pada daftar database"})
        );
    }
}

#[tokio::test]
async fn tunnel_headers_never_allow_foreign_or_opaque_origins() {
    let f = fixture().await;
    for (origin, site) in [
        ("https://evil.test", "same-origin"),
        ("https://panel.test:444", "same-origin"),
        ("null", "same-origin"),
        ("http://panel.test", "same-origin"),
        ("https://panel.test", "cross-site"),
    ] {
        let response = f
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/v1/databases/{}/web/index.php?route=/config/get",
                        f.ids[0]
                    ))
                    .extension(axum::extract::ConnectInfo(
                        "127.0.0.1:44000".parse::<std::net::SocketAddr>().unwrap(),
                    ))
                    .header("cookie", &f.cookies[1])
                    .header("host", "panel.test")
                    .header("origin", origin)
                    .header("sec-fetch-site", site)
                    .header("x-forwarded-proto", "http")
                    .header("x-qpanel-cf-visitor", r#"{"scheme":"https"}"#)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            json_body(response).await["detail"],
            "Permintaan lintas asal ditolak."
        );
    }
}

#[tokio::test]
async fn tool_csp_is_preserved_while_panel_csp_remains_strict_and_download_bytes_survive() {
    let app = axum::Router::new()
        .route(
            "/api/v1/databases/1/web/",
            axum::routing::get(|| async {
                (
                    [(
                        "content-security-policy",
                        "script-src 'nonce-tool'; frame-ancestors 'none'",
                    )],
                    "tool",
                )
            }),
        )
        .route(
            "/panel",
            axum::routing::get(|| async {
                (
                    [("content-security-policy", "script-src 'unsafe-inline'")],
                    "panel",
                )
            }),
        )
        .layer(axum::middleware::from_fn(
            panel_api::security_headers::security_headers_middleware,
        ));
    let tool = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/databases/1/web/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        tool.headers()["content-security-policy"],
        "script-src 'nonce-tool'; frame-ancestors 'none'"
    );
    assert_eq!(tool.headers()["referrer-policy"], "no-referrer");
    let panel = app
        .oneshot(
            Request::builder()
                .uri("/panel")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        panel.headers()["content-security-policy"],
        panel_api::security_headers::CSP_VALUE
    );
    let response = panel_api::database_web::parse_response(
        b"Content-Type: application/octet-stream\r\n\r\n\0\xffdump",
        "/api/v1/databases/1/web/",
        true,
        "panel.test",
    )
    .unwrap();
    assert_eq!(
        &response.into_body().collect().await.unwrap().to_bytes()[..],
        b"\0\xffdump"
    );
}

#[test]
fn path_validation_and_cgi_response_preserve_binary_downloads_and_scoped_cookies() {
    use panel_api::database_web::{parse_response, valid_asset};
    for path in [
        "../config.php",
        "foo/../bar",
        "/etc/passwd",
        "%2e%2e/etc",
        "foo\\bar",
        "a//b",
    ] {
        assert!(!valid_asset(path), "{path}");
    }
    for path in [
        "",
        "index.php",
        "themes/pmahomme/css/theme.css",
        "js/vendor/jquery/jquery.min.js",
    ] {
        assert!(valid_asset(path));
    }
    let raw = b"Content-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=export.sql\r\nSet-Cookie: adminer_sid=hello; Domain=evil.test; Path=/\r\nSet-Cookie: nusa_session=evil\r\n\r\n\0binary\xff";
    let response = parse_response(raw, "/api/v1/databases/1/web/", true, "panel.test").unwrap();
    assert_eq!(response.headers().get_all("set-cookie").iter().count(), 1);
    assert_eq!(
        response.headers()["set-cookie"],
        "adminer_sid=hello; Path=/api/v1/databases/1/web/; HttpOnly; SameSite=Strict; Secure"
    );
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert!(
        parse_response(
            b"Location: https://evil.test\r\n\r\n",
            "/api/v1/databases/1/web/",
            true,
            "panel.test"
        )
        .is_err()
    );
    assert!(
        parse_response(
            b"Location: //evil.test\r\n\r\n",
            "/api/v1/databases/1/web/",
            true,
            "panel.test"
        )
        .is_err()
    );
    assert!(parse_response(b"broken", "/api/v1/databases/1/web/", true, "panel.test").is_err());
    let response = parse_response(
        b"Location: https://panel.test/api/v1/databases/1/web/index.php\r\n\r\n",
        "/api/v1/databases/1/web/",
        true,
        "panel.test",
    )
    .unwrap();
    assert_eq!(
        response.headers()["location"],
        "/api/v1/databases/1/web/index.php"
    );
}

#[test]
fn https_session_cookies_survive_the_gateway_response() {
    use panel_api::database_web::parse_response;
    let prefix = "/api/v1/databases/1/web/";
    for name in [
        "phpMyAdmin",
        "phpMyAdmin_https",
        "__Secure-phpMyAdmin_https",
        "pma_lang_https",
        "__Secure-pma_lang_https",
        "adminer_sid",
    ] {
        let raw = format!(
            "Status: 302 Found\r\nLocation: ./index.php?route=/&reload=1\r\nSet-Cookie: {name}=rotated; Domain=internal; Path=/\r\nSet-Cookie: nusa_session=secret\r\nSet-Cookie: __Secure-nusa_session=secret\r\nSet-Cookie: phpMyAdmin_fake=bad\r\n\r\n"
        );
        let response = parse_response(raw.as_bytes(), prefix, true, "panel.test").unwrap();
        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(
            response.headers()["location"],
            "./index.php?route=/&reload=1"
        );
        assert_eq!(response.headers().get_all("set-cookie").iter().count(), 1);
        assert_eq!(
            response.headers()["set-cookie"],
            format!("{name}=rotated; Path={prefix}; HttpOnly; SameSite=Strict; Secure")
        );
    }
}

#[tokio::test]
async fn fastcgi_transport_sends_post_body_and_keeps_stderr_private() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (client, mut server) = tokio::io::duplex(4096);
    let task = tokio::spawn(async move {
        let mut payload = vec![];
        loop {
            let mut head = [0; 8];
            server.read_exact(&mut head).await.unwrap();
            let size = u16::from_be_bytes([head[4], head[5]]) as usize;
            let mut bytes = vec![0; size];
            server.read_exact(&mut bytes).await.unwrap();
            if head[1] == 5 {
                payload.extend_from_slice(&bytes);
                if size == 0 {
                    break;
                }
            }
        }
        assert_eq!(payload, b"token=csrf&sql=SELECT+1");
        for (kind, bytes) in [
            (7, b"private query diagnostics".as_slice()),
            (
                6,
                b"Content-Type: text/html\r\n\r\n<h1>tables</h1>".as_slice(),
            ),
            (3, [0; 8].as_slice()),
        ] {
            let size = bytes.len() as u16;
            server
                .write_all(&[1, kind, 0, 1, (size >> 8) as u8, size as u8, 0, 0])
                .await
                .unwrap();
            server.write_all(bytes).await.unwrap();
        }
    });
    let output = panel_api::database_web::exchange(
        client,
        &[(
            "SCRIPT_FILENAME".into(),
            "/usr/share/panel/db-web/entry.php".into(),
        )],
        b"token=csrf&sql=SELECT+1",
    )
    .await
    .unwrap();
    assert!(String::from_utf8_lossy(&output).contains("<h1>tables</h1>"));
    assert!(!String::from_utf8_lossy(&output).contains("diagnostics"));
    task.await.unwrap();
}
