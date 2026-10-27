#![allow(clippy::unwrap_used, clippy::expect_used)]
//! In-process API, temporary SQLite and optional typed RPC stub only. No daemon.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use panel_api::{
    auth::{LockoutManager, SESSION_COOKIE_NAME},
    config::AuthConfig,
    routes::AppState,
};
use panel_core::connectivity::*;
use panel_db::{
    CreateDomain, CreateSession, CreateSite, CreateUser, DbPool, SqliteConnectivityRepository,
    SqliteDomainRepository, SqliteSessionRepository, SqliteSiteRepository, SqliteUserRepository,
};
use serde_json::{Value, json};
use std::{num::NonZeroU16, sync::Arc};
use tower::ServiceExt;

struct Fixture {
    app: axum::Router,
    pool: DbPool,
    users: Vec<i64>,
    sites: Vec<i64>,
    task: Option<tokio::task::JoinHandle<()>>,
    _tmp: tempfile::TempDir,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}
async fn fixture() -> Fixture {
    fixture_rpc(false).await
}
async fn fixture_rpc(with_rpc: bool) -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let pool = panel_db::init_pool("sqlite::memory:").await.unwrap();
    panel_db::run_migrations(&pool).await.unwrap();
    let mut users = vec![];
    let mut sites = vec![];
    for (i, role) in ["admin", "user", "user"].iter().enumerate() {
        let user = SqliteUserRepository::new(pool.clone())
            .create(&CreateUser {
                username: format!("actor{i}"),
                password_hash: "fixture".into(),
                role: role.to_string(),
                email: None,
                linux_username: if i > 0 {
                    Some(format!("p_actor{i}"))
                } else {
                    None
                },
                linux_uid: if i > 0 { Some(10000 + i as i64) } else { None },
                plan_id: None,
            })
            .await
            .unwrap();
        users.push(user.id);
        SqliteUserRepository::new(pool.clone())
            .update_shell_policy(user.id, Some(if i == 2 { "sftp" } else { "full" }))
            .await
            .unwrap();
        SqliteSessionRepository::new(pool.clone())
            .create(&CreateSession {
                id: format!("sid-{i}"),
                user_id: user.id,
                token_hash: panel_core::hash_token(&format!("cookie-{i}")),
                ip_address: None,
                user_agent: None,
                expires_at: chrono::Utc::now().timestamp() + 3600,
            })
            .await
            .unwrap();
        if i > 0 {
            let site = SqliteSiteRepository::new(pool.clone())
                .create(&CreateSite {
                    user_id: user.id,
                    kind: "static".into(),
                    docroot: "/sites/app/public".into(),
                    runtime: None,
                    version: None,
                    status: None,
                    settings_json: None,
                })
                .await
                .unwrap();
            SqliteDomainRepository::new(pool.clone())
                .create(&CreateDomain {
                    site_id: site.id,
                    name: format!("site{i}.example.com"),
                    unicode_name: format!("site{i}.example.com"),
                    is_primary: true,
                    redirect_mode: None,
                    redirect_target: None,
                    status: None,
                })
                .await
                .unwrap();
            sites.push(site.id);
        }
    }
    let rpc_path = tmp.path().join("mock-rpc");
    let task = if with_rpc {
        #[cfg(unix)]
        let listener = tokio::net::UnixListener::bind(&rpc_path).unwrap();
        #[cfg(not(unix))]
        let listener = {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            std::fs::write(&rpc_path, listener.local_addr().unwrap().port().to_string()).unwrap();
            listener
        };
        Some(tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let mut rpc = panel_proto::RpcStream::new(stream);
                let req = rpc.recv_request().await.unwrap();
                assert_eq!(req.payload, panel_proto::RequestPayload::SshHostKeys);
                rpc.send_response(&panel_proto::RpcResponse::ok(
                    req.request_id,
                    panel_proto::ResponsePayload::SshHostKeys(vec![
                        panel_core::ssh::SshHostKeyFingerprint {
                            key_type: "ssh-ed25519".into(),
                            fingerprint_sha256: "SHA256:fixturehost".into(),
                        },
                    ]),
                ))
                .await
                .unwrap();
            }
        }))
    } else {
        None
    };
    let state = AppState::new(
        Arc::new(panel_api::agent_client::AgentClient::new(rpc_path)),
        pool.clone(),
        "development".into(),
        AuthConfig {
            cookie_secure: Some(false),
            ..Default::default()
        },
        Arc::new(LockoutManager::new(5, 60)),
    );
    Fixture {
        app: panel_api::create_router(state),
        pool,
        users,
        sites,
        task,
        _tmp: tmp,
    }
}
async fn call(f: &Fixture, actor: Option<usize>, query: &str) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .uri(format!("/api/v1/ssh/connection-info{query}"))
        .header("host", "panel-tunnel.example.com");
    if let Some(i) = actor {
        req = req.header("cookie", format!("{SESSION_COOKIE_NAME}=cookie-{i}"));
    }
    let res = f
        .app
        .clone()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({"text":String::from_utf8_lossy(&bytes)})),
    )
}
async fn endpoint(
    f: &Fixture,
    scope: EndpointScope,
    transport: EndpointTransport,
    host: &str,
    port: u16,
) -> String {
    SqliteConnectivityRepository::new(f.pool.clone())
        .create(
            f.users[0],
            &EndpointConfig {
                service: EndpointService::Ssh,
                scope,
                transport,
                host: EndpointHost::parse(host).unwrap(),
                port: NonZeroU16::new(port).unwrap(),
                tls_mode: EndpointTlsMode::Ssh,
                certificate_id: None,
                enabled: true,
                primary: true,
            },
        )
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn missing_never_infers_website_or_request_host() {
    let f = fixture().await;
    for q in [String::new(), format!("?site_id={}", f.sites[0])] {
        let (s, v) = call(&f, Some(1), &q).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["endpoint_state"], "missing");
        assert!(v["host"].is_null());
        assert!(v["port"].is_null());
        assert!(v["ssh_command"].is_null());
        assert!(v["sftp_command"].is_null());
        assert!(v["ssh_config_snippet"].is_null());
    }
}
#[tokio::test]
async fn host_direct_ipv6_and_project_primary_use_explicit_ports() {
    let f = fixture().await;
    let global = endpoint(
        &f,
        EndpointScope::Host,
        EndpointTransport::Direct,
        "2001:db8::1",
        2222,
    )
    .await;
    let (_, v) = call(&f, Some(1), "").await;
    assert_eq!(v["endpoint_id"], global);
    assert_eq!(v["endpoint_state"], "configured");
    assert_eq!(v["port"], 2222);
    assert!(
        v["ssh_command"]
            .as_str()
            .unwrap()
            .starts_with("ssh -p 2222 -t p_actor1@2001:db8::1 ")
    );
    assert!(
        v["sftp_url"]
            .as_str()
            .unwrap()
            .starts_with("sftp://p_actor1@[2001:db8::1]:2222/")
    );
    assert_eq!(v["host_keys_status"], "unavailable");
    assert_eq!(v["host_keys"], json!([]));
    let project = endpoint(
        &f,
        EndpointScope::Project {
            site_id: f.sites[0],
        },
        EndpointTransport::Direct,
        "ssh.example.com",
        2200,
    )
    .await;
    let (_, v) = call(&f, Some(1), &format!("?site_id={}", f.sites[0])).await;
    assert_eq!(v["endpoint_id"], project);
    assert_eq!(v["host"], "ssh.example.com");
    assert_eq!(v["port"], 2200);
    assert_eq!(v["site_domain"], "site1.example.com");
    let (_, v) = call(&f, Some(2), "").await;
    assert_eq!(v["endpoint_id"], global);
}
#[tokio::test]
async fn non_direct_project_does_not_fall_back_to_global_direct() {
    let f = fixture().await;
    endpoint(
        &f,
        EndpointScope::Host,
        EndpointTransport::Direct,
        "ssh.example.com",
        22,
    )
    .await;
    endpoint(
        &f,
        EndpointScope::Project {
            site_id: f.sites[0],
        },
        EndpointTransport::CloudflareAccess,
        "access.example.com",
        22,
    )
    .await;
    let (_, v) = call(&f, Some(1), &format!("?site_id={}", f.sites[0])).await;
    assert_eq!(v["endpoint_state"], "unsupported_transport");
    assert_eq!(v["transport"], "cloudflare_access");
    assert!(v["ssh_command"].is_null());
    assert!(v["sftp_command"].is_null());
    assert!(v["host"].is_null());
}
#[tokio::test]
async fn disabled_or_secondary_endpoint_is_not_selected() {
    let f = fixture().await;
    let id = endpoint(
        &f,
        EndpointScope::Host,
        EndpointTransport::Direct,
        "ssh.example.com",
        22,
    )
    .await;
    for (enabled, primary) in [(1, 0), (0, 0)] {
        sqlx::query("UPDATE service_endpoints SET enabled=?,is_primary=? WHERE id=?")
            .bind(enabled)
            .bind(primary)
            .bind(&id)
            .execute(&f.pool)
            .await
            .unwrap();
        let (_, v) = call(&f, Some(1), "").await;
        assert_eq!(v["endpoint_state"], "missing");
    }
}
#[tokio::test]
async fn policies_and_chroot_path_are_preserved() {
    let f = fixture().await;
    endpoint(
        &f,
        EndpointScope::Host,
        EndpointTransport::Direct,
        "ssh.example.com",
        2022,
    )
    .await;
    let (_, v) = call(&f, Some(2), "?path=/sites/app/public").await;
    assert_eq!(v["shell_policy"], "sftp");
    assert_eq!(v["can_shell"], false);
    assert_eq!(v["can_sftp"], true);
    assert_eq!(v["sftp_path"], "/p_actor2/sites/app/public");
    assert!(v["ssh_command"].is_null());
    assert!(
        v["sftp_cd_command"]
            .as_str()
            .unwrap()
            .contains("/p_actor2/sites/app/public")
    );
    SqliteUserRepository::new(f.pool.clone())
        .update_shell_policy(f.users[2], Some("none"))
        .await
        .unwrap();
    let (_, v) = call(&f, Some(2), "").await;
    assert_eq!(v["can_sftp"], false);
    assert!(v["ssh_config_snippet"].is_null());
    assert!(v["sftp_url"].is_null());
}
#[tokio::test]
async fn manual_preview_is_validated_and_never_publishes_identity() {
    let f = fixture().await;
    endpoint(
        &f,
        EndpointScope::Host,
        EndpointTransport::Direct,
        "ssh.example.com",
        2222,
    )
    .await;
    let (_, v) = call(
        &f,
        Some(1),
        "?host=2001:db8::2&port=2200&path=sites/bob%27s%20%22%24HOME%22%20%60id%60%20%24%28id%29%20%5Bapp%5D",
    )
    .await;
    assert_eq!(v["endpoint_state"], "manual_preview");
    assert!(v["endpoint_id"].is_null());
    assert_eq!(v["host_keys_status"], "untrusted_override");
    assert_eq!(v["host_keys"], json!([]));
    assert!(
        v["sftp_url"]
            .as_str()
            .unwrap()
            .contains("bob%27s%20%22%24HOME%22%20%60id%60%20%24%28id%29%20%5Bapp%5D/")
    );
    // A real POSIX parser checks these generated fixture commands separately,
    // using shell functions in place of every SSH/SFTP client and remote action.
    std::fs::write(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/ssh-direct-generated.json"),
        serde_json::to_vec(&v).unwrap(),
    )
    .unwrap();
    let (_, original) = call(&f, Some(1), "").await;
    assert_eq!(original["host"], "ssh.example.com");
    assert_eq!(original["port"], 2222);
    for q in [
        "?host=-oProxyCommand=id&port=22",
        "?host=host%0AProxyCommand%20id&port=22",
        "?host=https://host.test&port=22",
        "?host=host.test:22&port=22",
        "?host=%5B::1%5D&port=22",
        "?host=host.test&port=0",
    ] {
        assert_eq!(call(&f, Some(1), q).await.0, StatusCode::BAD_REQUEST, "{q}");
    }
}
#[tokio::test]
async fn no_endpoint_manual_host_and_port_must_be_explicit() {
    let f = fixture().await;
    for q in ["?port=2222", "?host=ssh.example.com"] {
        assert_eq!(call(&f, Some(1), q).await.0, StatusCode::BAD_REQUEST);
    }
    assert_eq!(
        call(&f, Some(1), "?host=ssh.example.com&port=2222").await.0,
        StatusCode::OK
    );
}
#[tokio::test]
async fn folder_validation_enforces_tenant_boundary_and_preserves_spaces() {
    let f = fixture().await;
    assert_eq!(
        call(&f, Some(1), &format!("?path={}", "a".repeat(3990)))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    for q in [
        "?path=../p_actor2",
        "?path=/home/p_actor1/../p_actor2",
        "?path=x%00y",
        "?path=x%0Ay",
        "?path=x%5Cy",
    ] {
        assert_eq!(call(&f, Some(1), q).await.0, StatusCode::BAD_REQUEST, "{q}");
    }
    for q in ["?path=/home/p_actor2/a", "?path=/home/p_actor1evil/a"] {
        assert_eq!(call(&f, Some(1), q).await.0, StatusCode::FORBIDDEN);
    }
    let (_, v) = call(&f, Some(1), "?path=sites/a%20").await;
    assert_eq!(v["active_path"], "/home/p_actor1/sites/a ");
}
#[tokio::test]
async fn rbac_active_identity_and_site_isolation() {
    let f = fixture().await;
    assert_eq!(call(&f, None, "").await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(call(&f, Some(0), "").await.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        call(&f, Some(1), "?tenant=actor2").await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&f, Some(1), &format!("?site_id={}", f.sites[1]))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(call(&f, Some(0), "?tenant=actor2").await.0, StatusCode::OK);
    assert_eq!(
        call(
            &f,
            Some(0),
            &format!("?tenant=actor2&site_id={}", f.sites[0])
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    sqlx::query("UPDATE users SET status='suspended' WHERE id=?")
        .bind(f.users[2])
        .execute(&f.pool)
        .await
        .unwrap();
    assert_eq!(
        call(&f, Some(0), "?tenant=actor2").await.0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn typed_rpc_identity_is_exposed_only_for_configured_direct_endpoint() {
    let f = fixture_rpc(true).await;
    endpoint(
        &f,
        EndpointScope::Host,
        EndpointTransport::Direct,
        "ssh.example.com",
        2222,
    )
    .await;
    let (_, v) = call(&f, Some(1), "").await;
    assert_eq!(v["host_keys_status"], "local_configuration");
    assert_eq!(
        v["host_keys"][0]["fingerprint_sha256"],
        "SHA256:fixturehost"
    );
    let (_, v) = call(&f, Some(1), "?host=manual.example.com&port=22").await;
    assert_eq!(v["host_keys"], json!([]));
    assert_eq!(v["host_keys_status"], "untrusted_override");
}
