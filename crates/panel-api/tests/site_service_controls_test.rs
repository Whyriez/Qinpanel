#![allow(clippy::unwrap_used, clippy::expect_used)]
//! SQLite + typed RPC fixtures only. No AgentServer or host services.
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use http_body_util::BodyExt;
use panel_api::{
    agent_client::AgentClient,
    auth::{AuthenticatedUser, LockoutManager},
    config::AuthConfig,
    routes::AppState,
    sites::{SiteQuickActionRequest, get_site_overview_handler, site_quick_action_handler},
};
use panel_db::{
    models::{CreateDomain, CreateSite, CreateUser, Site},
    repositories::{SqliteDomainRepository, SqliteSiteRepository, SqliteUserRepository},
};
use serde_json::{Value, json};
use std::sync::Arc;
struct Fixture {
    state: AppState,
    site: Site,
    calls: Arc<tokio::sync::Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
    _temp: tempfile::TempDir,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
fn actor() -> AuthenticatedUser {
    AuthenticatedUser {
        id: 100,
        username: "admin".into(),
        role: "admin".into(),
        email: None,
        totp_enabled: false,
        session_id: "fixture".into(),
        csrf_token: "fixture".into(),
        impersonated_by: None,
    }
}
async fn fixture(kind: &str, active: bool, failure: &'static str) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let socket = temp.path().join("fixture.sock");
    #[cfg(unix)]
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    #[cfg(not(unix))]
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    #[cfg(not(unix))]
    std::fs::write(&socket, listener.local_addr().unwrap().port().to_string()).unwrap();
    let calls = Arc::new(tokio::sync::Mutex::new(vec![]));
    let captured = Arc::clone(&calls);
    let task = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let mut rpc = panel_proto::RpcStream::new(stream);
            let request = rpc.recv_request().await.unwrap();
            let value = serde_json::to_value(request.payload).unwrap();
            captured.lock().await.push(value.clone());
            let command = value["cmd"].as_str().unwrap();
            let data = &value["data"];
            let reply = if failure == command {
                panel_proto::RpcResponse::err(request.request_id, "fixture agent unavailable")
            } else {
                let (response, body) = match command {
                    "NodeSiteAction" | "PythonSiteAction" => (
                        command,
                        json!({"site_id":data["site_id"],"action":data["action"],"success":failure!="ActionFalse","message":"fixture"}),
                    ),
                    "SiteApply" => (
                        "SiteApplied",
                        json!({"site_id":data["site_id"],"available_path":"/fixture/vhost","enabled_path":null,"enabled":data["enable"],"reloaded":failure!="ReloadFalse"}),
                    ),
                    "NodeSiteStatus" => (
                        command,
                        json!({"site_id":data["site_id"],"service_name":"fixture","active":active,"sub_state":if active {"running"} else {"dead"},"healthy":active,"probe_status_code":if active {json!(200)} else {Value::Null}}),
                    ),
                    "PythonSiteStatus" => (
                        command,
                        json!({"site_id":data["site_id"],"service_name":"fixture","active":active,"sub_state":if active {"running"} else {"dead"},"healthy":active,"socket_exists":active}),
                    ),
                    "PhpFpmPoolStatus" => (
                        command,
                        json!({"pool_name":data["pool_name"],"service_name":"fixture","is_active":active,"socket_exists":active,"metrics":null}),
                    ),
                    _ => {
                        rpc.send_response(&panel_proto::RpcResponse::err(
                            request.request_id,
                            "fixture metric unavailable",
                        ))
                        .await
                        .unwrap();
                        continue;
                    }
                };
                panel_proto::RpcResponse::ok(
                    request.request_id,
                    serde_json::from_value(json!({"type":response,"data":body})).unwrap(),
                )
            };
            rpc.send_response(&reply).await.unwrap();
        }
    });
    let pool = panel_db::init_pool("sqlite::memory:").await.unwrap();
    panel_db::run_migrations(&pool).await.unwrap();
    let owner = SqliteUserRepository::new(pool.clone())
        .create(&CreateUser {
            username: "alice".into(),
            password_hash: "fixture-unused".into(),
            role: "user".into(),
            email: None,
            linux_username: Some("p_alice".into()),
            linux_uid: Some(10001),
            plan_id: None,
        })
        .await
        .unwrap();
    let site = SqliteSiteRepository::new(pool.clone())
        .create(&CreateSite {
            user_id: owner.id,
            kind: kind.into(),
            runtime: Some(kind.into()),
            version: Some(if kind == "php" { "8.3" } else { "22" }.into()),
            docroot: "/home/p_alice/sites/example.test/public".into(),
            status: Some("active".into()),
            settings_json: Some("{}".into()),
        })
        .await
        .unwrap();
    SqliteDomainRepository::new(pool.clone())
        .create(&CreateDomain {
            site_id: site.id,
            name: "example.test".into(),
            unicode_name: "example.test".into(),
            is_primary: true,
            redirect_mode: None,
            redirect_target: None,
            status: Some("active".into()),
        })
        .await
        .unwrap();
    let state = AppState::new(
        Arc::new(AgentClient::new(socket)),
        pool,
        "development".into(),
        AuthConfig::default(),
        Arc::new(LockoutManager::new(5, 60)),
    );
    Fixture {
        state,
        site,
        calls,
        task,
        _temp: temp,
    }
}
async fn action(f: &Fixture, name: &str) -> (StatusCode, Value) {
    let response = site_quick_action_handler(
        State(f.state.clone()),
        actor(),
        Path(f.site.id),
        Json(SiteQuickActionRequest {
            action: name.into(),
        }),
    )
    .await
    .into_response();
    let status = response.status();
    let body =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    (status, body)
}
async fn overview(f: &Fixture) -> Value {
    let response = get_site_overview_handler(State(f.state.clone()), actor(), Path(f.site.id))
        .await
        .into_response();
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
#[tokio::test]
async fn observed_status_distinguishes_unknown_stopped_and_no_local_process() {
    for (kind, command) in [
        ("nodejs", "NodeSiteStatus"),
        ("python", "PythonSiteStatus"),
        ("php", "PhpFpmPoolStatus"),
    ] {
        let f = fixture(kind, false, command).await;
        let data = overview(&f).await;
        assert_eq!(data["status"]["runtime_status"], "unknown");
        assert_eq!(data["status"]["healthy"], false);
        let f = fixture(kind, false, "").await;
        assert_eq!(overview(&f).await["status"]["runtime_status"], "inactive");
        let f = fixture(kind, true, "").await;
        assert_eq!(overview(&f).await["status"]["runtime_status"], "running");
    }
    let f = fixture("static", true, "").await;
    let data = overview(&f).await;
    assert_eq!(data["status"]["runtime_status"], "not_applicable");
    assert!(
        !data["quick_actions_available"]
            .as_array()
            .unwrap()
            .contains(&json!("restart"))
    );
    assert!(
        !data["quick_actions_available"]
            .as_array()
            .unwrap()
            .contains(&json!("clear_cache"))
    );
}
#[tokio::test]
async fn failed_service_or_nginx_acknowledgement_does_not_commit_stopped_state() {
    for failure in ["NodeSiteAction", "ActionFalse", "SiteApply", "ReloadFalse"] {
        let f = fixture("nodejs", true, failure).await;
        let (status, body) = action(&f, "stop").await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{failure}: {body}");
        assert!(body.get("success").is_none());
        assert_eq!(
            SqliteSiteRepository::new(f.state.pool.clone())
                .find_by_id(f.site.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            "active"
        );
    }
}
#[tokio::test]
async fn successful_start_stop_survive_reread_without_changing_runtime_or_data() {
    for kind in ["nodejs", "python", "php", "static"] {
        let f = fixture(kind, true, "").await;
        assert_eq!(action(&f, "stop").await.0, StatusCode::OK);
        assert_eq!(
            overview(&f).await["status"]["webserver_status"],
            "suspended"
        );
        assert_eq!(action(&f, "start").await.0, StatusCode::OK);
        let stored = SqliteSiteRepository::new(f.state.pool.clone())
            .find_by_id(f.site.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, "active");
        assert_eq!(stored.version, f.site.version);
        assert_eq!(stored.docroot, f.site.docroot);
        for call in f
            .calls
            .lock()
            .await
            .iter()
            .filter(|c| c["cmd"] == "SiteApply")
        {
            assert_eq!(call["data"]["test_config"], true);
        }
    }
}
#[tokio::test]
async fn reload_never_reconciles_or_restarts_application_and_legacy_php_restart_is_honest() {
    for kind in ["nodejs", "python", "php", "static"] {
        let f = fixture(kind, true, "").await;
        assert_eq!(action(&f, "reload").await.0, StatusCode::OK);
        let calls = f.calls.lock().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["cmd"], "SiteApply");
        assert!(
            !calls[0]["data"]["config_content"]
                .as_str()
                .unwrap()
                .is_empty()
        );
        drop(calls);
        if kind == "php" {
            let (status, body) = action(&f, "restart").await;
            assert_eq!(status, StatusCode::OK);
            assert!(
                body["message"]
                    .as_str()
                    .unwrap()
                    .contains("Proses aplikasi tidak dimulai ulang")
            );
        }
    }
}
#[tokio::test]
async fn restart_is_app_only_and_returns_agent_failure_without_success() {
    for (kind, command) in [("nodejs", "NodeSiteAction"), ("python", "PythonSiteAction")] {
        let f = fixture(kind, true, "").await;
        assert_eq!(action(&f, "restart").await.0, StatusCode::OK);
        assert_eq!(f.calls.lock().await[0]["cmd"], command);
        assert_eq!(f.calls.lock().await.len(), 1);
        let f = fixture(kind, true, command).await;
        assert_eq!(action(&f, "restart").await.0, StatusCode::BAD_GATEWAY);
    }
}
