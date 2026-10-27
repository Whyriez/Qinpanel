#![allow(clippy::unwrap_used, clippy::expect_used)]
//! In-process requests and SQLite fixtures only; no agent or panel daemon.
use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{HeaderMap, HeaderValue, Request, StatusCode},
};
use http_body_util::BodyExt;
use panel_api::{
    auth::{CSRF_HEADER_NAME, LockoutManager, SESSION_COOKIE_NAME, generate_csrf_token},
    config::AuthConfig,
    public_url::{PanelOrigin, request_https},
    routes::AppState,
};
use panel_db::{CreateSession, CreateUser, DbPool, SqliteSessionRepository, SqliteUserRepository};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

struct Fixture {
    app: axum::Router,
    pool: DbPool,
    cookies: Vec<String>,
    csrf: Vec<String>,
    _tmp: tempfile::TempDir,
}
async fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let pool = panel_db::init_pool("sqlite::memory:").await.unwrap();
    panel_db::run_migrations(&pool).await.unwrap();
    let mut cookies = vec![];
    let mut csrf = vec![];
    for (index, role) in ["admin", "user"].into_iter().enumerate() {
        let user = SqliteUserRepository::new(pool.clone())
            .create(&CreateUser {
                username: format!("actor{index}"),
                password_hash: panel_core::hash_password("fixture-password").unwrap(),
                role: role.into(),
                email: None,
                linux_username: None,
                linux_uid: None,
                plan_id: None,
            })
            .await
            .unwrap();
        let sid = format!("session-{index}");
        let token = format!("token-{index}");
        SqliteSessionRepository::new(pool.clone())
            .create(&CreateSession {
                id: sid.clone(),
                user_id: user.id,
                token_hash: panel_core::hash_token(&token),
                ip_address: None,
                user_agent: None,
                expires_at: chrono::Utc::now().timestamp() + 3600,
            })
            .await
            .unwrap();
        cookies.push(format!("{SESSION_COOKIE_NAME}={token}"));
        csrf.push(generate_csrf_token(&sid));
    }
    let state = AppState::new(
        Arc::new(panel_api::agent_client::AgentClient::new(
            tmp.path().join("missing-rpc"),
        )),
        pool.clone(),
        "production".into(),
        AuthConfig {
            cookie_secure: Some(false),
            ..Default::default()
        },
        Arc::new(LockoutManager::new(5, 60)),
    );
    Fixture {
        app: panel_api::create_router(state),
        pool,
        cookies,
        csrf,
        _tmp: tmp,
    }
}
#[allow(clippy::too_many_arguments)]
async fn call(
    f: &Fixture,
    method: &str,
    path: &str,
    host: &str,
    https: bool,
    actor: Option<usize>,
    csrf: bool,
    body: Value,
) -> (StatusCode, HeaderMap, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("host", host)
        .header("content-type", "application/json");
    if let Some(actor) = actor {
        req = req.header("cookie", &f.cookies[actor]);
        if csrf {
            req = req.header(CSRF_HEADER_NAME, &f.csrf[actor]);
        }
    }
    if https {
        req = req.header("x-forwarded-proto", "https");
    }
    let mut req = req
        .body(if body.is_null() {
            Body::empty()
        } else {
            Body::from(body.to_string())
        })
        .unwrap();
    req.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:49000".parse::<std::net::SocketAddr>().unwrap(),
    ));
    let response = f.app.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        headers,
        serde_json::from_slice(&body)
            .unwrap_or_else(|_| json!({"text":String::from_utf8_lossy(&body)})),
    )
}
async fn endpoint(f: &Fixture, host: &str, tls: &str, port: u16) -> Value {
    let (status,_,body)=call(f,"POST","/api/v1/connectivity","old.panel.test:17890",false,Some(0),true,json!({"service":"panel","scope":{"kind":"host"},"transport":"direct","host":host,"port":port,"tls_mode":tls,"certificate_id":null,"enabled":true,"primary":true})).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body
}
#[test]
fn origins_preserve_ipv6_ports_and_compare_normalized_default_ports_without_substrings() {
    assert_eq!(
        PanelOrigin::parse("http://LOCALHOST:5173").unwrap().url(),
        "http://localhost:5173"
    );
    let origin = PanelOrigin::parse("https://[2001:db8::7]:19443").unwrap();
    assert_eq!(origin.url(), "https://[2001:db8::7]:19443");
    assert!(
        PanelOrigin::parse("https://PANEL.test:443")
            .unwrap()
            .same(&PanelOrigin::parse("https://panel.test").unwrap())
    );
    for value in [
        "https://panel.test/",
        "https://panel.test?next=x",
        "https://panel.test#x",
        "https://user@panel.test",
        "https://panel.test:0",
        "https://panel.test:65536",
        "https://2001:db8::7",
        "https://panel.test,evil.test",
        "https://panel.test\\evil",
        "https://panel.test:bad",
        "https://panel.test%2eevil",
    ] {
        assert!(PanelOrigin::parse(value).is_err(), "{value}");
    }
    let origin = PanelOrigin::parse("https://panel.test:8443").unwrap();
    let mut headers = HeaderMap::new();
    for value in [
        "https://evil.test/panel.test",
        "https://panel.test.evil.test:8443",
        "http://panel.test:8443",
        "https://panel.test:443",
        "https://localhost.evil.test",
        "null",
    ] {
        headers.insert("origin", HeaderValue::from_str(value).unwrap());
        assert!(!origin.matches_origin(&headers, true), "{value}");
    }
    headers.insert(
        "origin",
        HeaderValue::from_static("https://panel.test:8443"),
    );
    assert!(origin.matches_origin(&headers, true));
    headers.append("origin", HeaderValue::from_static("https://evil.test"));
    assert!(!origin.matches_origin(&headers, true));
    let headers = HeaderMap::new();
    assert!(!origin.matches_origin(&headers, true));
    assert_eq!(
        origin.action_url("/sites/1?tab=ssl").unwrap(),
        "https://panel.test:8443/sites/1?tab=ssl"
    );
    for path in [
        "//evil.test",
        "https://evil.test",
        "/\\evil.test",
        "/sites/../jobs",
        "/%2f%2fevil.test",
        "/sites\n",
    ] {
        assert!(origin.action_url(path).is_err());
    }
}
#[test]
fn only_the_actual_loopback_peer_and_sanitized_proxy_headers_can_assert_tls() {
    for peer in [
        None,
        Some("203.0.113.3:90"),
        Some("127.0.0.1:90"),
        Some("[::1]:90"),
    ] {
        let mut req = Request::builder()
            .header("cf-visitor", r#"{"scheme":"https"}"#)
            .body(Body::empty())
            .unwrap();
        if let Some(peer) = peer {
            req.extensions_mut()
                .insert(ConnectInfo(peer.parse::<std::net::SocketAddr>().unwrap()));
        }
        assert!(!request_https(&req));
        req.headers_mut().insert(
            "x-qpanel-cf-visitor",
            HeaderValue::from_static(r#"{"scheme":"https"}"#),
        );
        assert_eq!(
            request_https(&req),
            peer.is_some_and(|p| p.starts_with("127.") || p.starts_with('['))
        );
        req.headers_mut().append(
            "x-qpanel-cf-visitor",
            HeaderValue::from_static(r#"{"scheme":"https"}"#),
        );
        assert!(!request_https(&req));
    }
}
#[tokio::test]
async fn tunnel_browser_asset_requests_require_trusted_https_metadata() {
    let f = fixture().await;
    // Browser module/preload requests can carry Origin even on GET/HEAD.
    // Model HTTP Nginx -> API after Cloudflare terminates public HTTPS.
    for (peer, visitor, raw_visitor, origin, accepted) in [
        (
            "127.0.0.1:49000",
            Some(r#"{"scheme":"https"}"#),
            None,
            "https://panel.test:19443",
            true,
        ),
        (
            "[::1]:49000",
            Some(r#"{"scheme":"https"}"#),
            None,
            "https://panel.test:19443",
            true,
        ),
        (
            "127.0.0.1:49000",
            None,
            None,
            "https://panel.test:19443",
            false,
        ),
        (
            "127.0.0.1:49000",
            None,
            Some(r#"{"scheme":"https"}"#),
            "https://panel.test:19443",
            false,
        ),
        (
            "203.0.113.3:49000",
            Some(r#"{"scheme":"https"}"#),
            None,
            "https://panel.test:19443",
            false,
        ),
        (
            "127.0.0.1:49000",
            Some(r#"{"scheme":"https"}"#),
            None,
            "https://panel.test:19444",
            false,
        ),
        (
            "127.0.0.1:49000",
            Some(r#"{"scheme":"https"}"#),
            None,
            "https://evil.test:19443",
            false,
        ),
    ] {
        for method in ["GET", "HEAD"] {
            for (path, status, content_type) in [
                ("/", StatusCode::OK, "text/html"),
                ("/panel-url-check.css", StatusCode::OK, "text/css"),
                (
                    "/assets/qpanel-origin-missing.css",
                    StatusCode::NOT_FOUND,
                    "text/plain",
                ),
                (
                    "/assets/qpanel-origin-missing.js",
                    StatusCode::NOT_FOUND,
                    "text/plain",
                ),
            ] {
                let mut request = Request::builder()
                    .method(method)
                    .uri(path)
                    .header("host", "panel.test:19443")
                    .header("origin", origin)
                    .header("sec-fetch-site", "same-origin")
                    .header("sec-fetch-mode", "cors")
                    .header("x-forwarded-proto", "http")
                    .extension(ConnectInfo(peer.parse::<std::net::SocketAddr>().unwrap()));
                if let Some(visitor) = visitor {
                    request = request.header("x-qpanel-cf-visitor", visitor);
                }
                if let Some(visitor) = raw_visitor {
                    request = request.header("cf-visitor", visitor);
                }
                let response = f
                    .app
                    .clone()
                    .oneshot(request.body(Body::empty()).unwrap())
                    .await
                    .unwrap();
                assert_eq!(
                    response.status(),
                    if accepted {
                        status
                    } else {
                        StatusCode::FORBIDDEN
                    },
                    "{method} {path}, peer={peer}, origin={origin}, visitor={visitor:?}"
                );
                assert!(
                    response.headers()["content-type"]
                        .to_str()
                        .unwrap()
                        .starts_with(if accepted {
                            content_type
                        } else {
                            "application/problem+json"
                        })
                );
            }
        }
    }
}

#[tokio::test]
async fn explicit_activation_keeps_current_access_uses_revisions_and_rejects_unsafe_changes() {
    let f = fixture().await;
    let e = endpoint(&f, "2001:db8::7", "tls", 19443).await;
    let body = json!({"endpoint_id":e["id"],"expected_endpoint_revision":1,"expected_revision":0,"reachable_confirmed":true});
    for (actor, csrf, expected) in [
        (None, false, StatusCode::UNAUTHORIZED),
        (Some(1), true, StatusCode::FORBIDDEN),
        (Some(0), false, StatusCode::FORBIDDEN),
    ] {
        assert_eq!(
            call(
                &f,
                "POST",
                "/api/v1/connectivity/panel-url/activate",
                "old.panel.test:17890",
                false,
                actor,
                csrf,
                body.clone()
            )
            .await
            .0,
            expected
        );
    }
    let mut unconfirmed = body.clone();
    unconfirmed["reachable_confirmed"] = json!(false);
    assert_eq!(
        call(
            &f,
            "POST",
            "/api/v1/connectivity/panel-url/activate",
            "old.panel.test:17890",
            false,
            Some(0),
            true,
            unconfirmed
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (status, _, preview) = call(
        &f,
        "GET",
        "/connectivity/verify",
        "[2001:db8::7]:19443",
        true,
        None,
        false,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        preview["text"]
            .as_str()
            .unwrap()
            .contains("Alamat panel dapat dibuka")
    );
    let (status, _, active) = call(
        &f,
        "POST",
        "/api/v1/connectivity/panel-url/activate",
        "old.panel.test:17890",
        false,
        Some(0),
        true,
        body.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{active}");
    assert_eq!(active["canonical_url"], "https://[2001:db8::7]:19443");
    assert_eq!(active["aliases"], json!(["http://old.panel.test:17890"]));
    assert_eq!(
        call(
            &f,
            "GET",
            "/api/v1/auth/me",
            "old.panel.test:17890",
            false,
            Some(0),
            false,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &f,
            "GET",
            "/api/v1/auth/me",
            "[2001:db8::7]:19443",
            true,
            Some(0),
            false,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &f,
            "GET",
            "/api/v1/auth/me",
            "evil.test",
            false,
            Some(0),
            false,
            Value::Null
        )
        .await
        .0,
        StatusCode::MISDIRECTED_REQUEST
    );
    assert_eq!(
        call(
            &f,
            "POST",
            "/api/v1/connectivity/panel-url/activate",
            "old.panel.test:17890",
            false,
            Some(0),
            true,
            body
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let revoke = json!({"origin":"http://old.panel.test:17890","expected_revision":1});
    assert_eq!(
        call(
            &f,
            "POST",
            "/api/v1/connectivity/panel-url/revoke",
            "old.panel.test:17890",
            false,
            Some(0),
            true,
            revoke.clone()
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            &f,
            "POST",
            "/api/v1/connectivity/panel-url/revoke",
            "[2001:db8::7]:19443",
            true,
            Some(0),
            true,
            revoke
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &f,
            "GET",
            "/api/v1/auth/me",
            "old.panel.test:17890",
            false,
            Some(0),
            false,
            Value::Null
        )
        .await
        .0,
        StatusCode::MISDIRECTED_REQUEST
    );
    let jobs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(jobs, 0);
}
#[tokio::test]
async fn cookies_use_effective_tls_for_login_logout_and_credential_rotation_and_ws_rejects_foreign_origins()
 {
    let f = fixture().await;
    for (host, https) in [
        ("203.0.113.2:17890", false),
        ("panel.test", true),
        ("[2001:db8::2]:19443", true),
    ] {
        let (status, headers, body) = call(
            &f,
            "POST",
            "/api/v1/auth/login",
            host,
            https,
            None,
            false,
            json!({"username":"actor0","password":"fixture-password"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let cookie = headers["set-cookie"].to_str().unwrap();
        assert_eq!(cookie.contains("; Secure"), https);
        assert!(
            cookie.contains("HttpOnly")
                && cookie.contains("SameSite=Strict")
                && !cookie.contains("Domain=")
        );
    }
    let (status, headers, _) = call(
        &f,
        "POST",
        "/api/v1/auth/logout",
        "panel.test",
        true,
        Some(1),
        true,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers["set-cookie"].to_str().unwrap().contains("; Secure"));
    let (status, headers, body) = call(
        &f,
        "POST",
        "/api/v1/auth/password",
        "panel.test",
        true,
        Some(0),
        true,
        json!({"current_password":"fixture-password","new_password":"NewFixture-Password123!"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(headers["set-cookie"].to_str().unwrap().contains("; Secure"));
    let request = Request::builder()
        .uri("/ws/terminal/missing")
        .header("host", "panel.test")
        .header("origin", "https://panel.test.evil.test")
        .header("x-forwarded-proto", "https")
        .body(Body::empty())
        .unwrap();
    let response = f.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
#[tokio::test]
async fn metadata_edits_do_not_silently_switch_activated_urls_and_https_downgrade_is_rejected() {
    let f = fixture().await;
    let e = endpoint(&f, "panel.test", "tls", 8443).await;
    let activate = json!({"endpoint_id":e["id"],"expected_endpoint_revision":1,"expected_revision":0,"reachable_confirmed":true});
    assert_eq!(
        call(
            &f,
            "POST",
            "/api/v1/connectivity/panel-url/activate",
            "old.panel.test:17890",
            false,
            Some(0),
            true,
            activate
        )
        .await
        .0,
        StatusCode::OK
    );
    let config = json!({"service":"panel","scope":{"kind":"host"},"transport":"direct","host":"future.panel.test","port":17890,"tls_mode":"none","certificate_id":null,"enabled":true,"primary":true});
    let path = format!("/api/v1/connectivity/{}", e["id"].as_str().unwrap());
    assert_eq!(
        call(
            &f,
            "PUT",
            &path,
            "old.panel.test:17890",
            false,
            Some(0),
            true,
            json!({"expected_revision":1,"config":config})
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, _, policy) = call(
        &f,
        "GET",
        "/api/v1/connectivity/panel-url",
        "old.panel.test:17890",
        false,
        Some(0),
        false,
        Value::Null,
    )
    .await;
    assert_eq!(policy["canonical_url"], "https://panel.test:8443");
    let apply = json!({"endpoint_id":e["id"],"expected_endpoint_revision":2,"expected_revision":1,"reachable_confirmed":true});
    assert_eq!(
        call(
            &f,
            "POST",
            "/api/v1/connectivity/panel-url/activate",
            "old.panel.test:17890",
            false,
            Some(0),
            true,
            apply
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
async fn browser_navigation_precheck_and_ambiguous_headers_obey_the_active_policy() {
    let f = fixture().await;
    let e = endpoint(&f, "panel.test", "tls", 8443).await;
    assert_eq!(call(&f, "POST", "/api/v1/connectivity/panel-url/activate", "old.panel.test:17890", false, Some(0), true, json!({"endpoint_id":e["id"],"expected_endpoint_revision":1,"expected_revision":0,"reachable_confirmed":true})).await.0, StatusCode::OK);
    // A newly saved alternative is not activated, but can be checked before switching.
    let draft = json!({"service":"panel","scope":{"kind":"host"},"transport":"direct","host":"future.test","port":8443,"tls_mode":"tls","certificate_id":null,"enabled":true,"primary":false});
    assert_eq!(
        call(
            &f,
            "POST",
            "/api/v1/connectivity",
            "old.panel.test:17890",
            false,
            Some(0),
            true,
            draft
        )
        .await
        .0,
        StatusCode::CREATED
    );
    assert_eq!(
        call(
            &f,
            "GET",
            "/api/v1/auth/me",
            "future.test:8443",
            true,
            Some(0),
            false,
            Value::Null
        )
        .await
        .0,
        StatusCode::MISDIRECTED_REQUEST
    );
    for path in ["/connectivity/verify", "/api/v1/auth/me"] {
        let req = Request::builder()
            .uri(path)
            .header(
                "host",
                if path == "/connectivity/verify" {
                    "future.test:8443"
                } else {
                    "panel.test:8443"
                },
            )
            .header("x-forwarded-proto", "https")
            .header("sec-fetch-site", "cross-site")
            .header("cookie", &f.cookies[0])
            .extension(ConnectInfo(
                "127.0.0.1:90".parse::<std::net::SocketAddr>().unwrap(),
            ))
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            f.app.clone().oneshot(req).await.unwrap().status(),
            StatusCode::OK
        );
    }
    for (name, value, expected) in [
        ("host", "evil.test", StatusCode::BAD_REQUEST),
        ("origin", "https://panel.test:8443", StatusCode::FORBIDDEN),
    ] {
        let mut req = Request::builder()
            .uri("/api/v1/auth/me")
            .header("host", "panel.test:8443")
            .header("x-forwarded-proto", "https")
            .extension(ConnectInfo(
                "127.0.0.1:90".parse::<std::net::SocketAddr>().unwrap(),
            ))
            .body(Body::empty())
            .unwrap();
        req.headers_mut()
            .append(name, HeaderValue::from_str(value).unwrap());
        if name == "origin" {
            req.headers_mut()
                .append(name, HeaderValue::from_static("https://evil.test"));
        }
        assert_eq!(f.app.clone().oneshot(req).await.unwrap().status(), expected);
    }
}

#[tokio::test]
async fn internal_api_port_is_not_a_public_panel_url() {
    let f = fixture().await;
    let e = endpoint(&f, "panel.test", "none", 7700).await;
    assert_eq!(call(&f,"POST","/api/v1/connectivity/panel-url/activate","old.panel.test:17890",false,Some(0),true,json!({"endpoint_id":e["id"],"expected_endpoint_revision":1,"expected_revision":0,"reachable_confirmed":true})).await.0,StatusCode::UNPROCESSABLE_ENTITY);
}
