#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Pure router/SQLite tests. No agent server or system services are started.
use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use panel_api::{
    agent_client::AgentClient,
    auth::{AuthenticatedUser, LockoutManager},
    config::AuthConfig,
    routes::AppState,
    tenant_context::resolve_tenant_context,
};
use panel_db::{
    models::CreateUser,
    pool::{init_pool, run_migrations},
    repositories::SqliteUserRepository,
};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

async fn fixture() -> (AppState, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp
        .path()
        .join("panel.db")
        .to_string_lossy()
        .replace('\\', "/");
    let pool = init_pool(&format!("sqlite://{path}")).await.unwrap();
    run_migrations(&pool).await.unwrap();
    sqlx::query("INSERT INTO settings (key,value,updated_at) VALUES ('setup.first_run_token','test-secret',0)")
        .execute(&pool)
        .await
        .unwrap();
    let state = AppState::new(
        Arc::new(AgentClient::new(tmp.path().join("no-agent"))),
        pool,
        "development".into(),
        AuthConfig {
            cookie_secure: Some(false),
            ..Default::default()
        },
        Arc::new(LockoutManager::new(5, 900)),
    );
    (state, tmp)
}
async fn post(state: &AppState, path: &str, payload: Value) -> (StatusCode, Value) {
    let res = panel_api::create_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    (status, value)
}
#[tokio::test]
async fn public_setup_is_retired_even_with_a_valid_legacy_token() {
    let (state, _tmp) = fixture().await;
    for path in ["/api/v1/auth/setup/validate", "/api/v1/auth/setup"] {
        for token in ["test-secret", "wrong"] {
            let (status, body) = post(
                &state,
                path,
                json!({"token":token,"username":"admin","password":"StrongSecret123!"}),
            )
            .await;
            assert_eq!(status, StatusCode::GONE);
            assert!(
                body["error"]
                    .as_str()
                    .unwrap()
                    .contains("panelctl admin bootstrap")
            );
        }
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users")
            .fetch_one(&state.pool)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn generic_jobs_cannot_bypass_tenant_feature_authorization() {
    use axum::{Json, response::IntoResponse};
    let (state, _tmp) = fixture().await;
    for name in [
        "php.install",
        "backup.restore",
        "files.search",
        "quota.scan",
    ] {
        let payload = serde_json::from_value(
            json!({"name":name,"payload":{"username":"p_other","uid":10043}}),
        )
        .unwrap();
        let response = panel_api::jobs::handlers::create_job_handler(
            State(state.clone()),
            panel_api::auth::RequireCsrf(actor(42, "customer", "user")),
            Json(payload),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM jobs")
            .fetch_one(&state.pool)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn terminal_denies_none_and_sftp_policies_including_admin_selected_tenant() {
    use axum::{Json, http::HeaderMap};
    let (state, _tmp) = fixture().await;
    let owner = tenant(&state, "customer", Some("p_actual"), Some(10042)).await;
    for policy in ["none", "sftp"] {
        sqlx::query("UPDATE users SET shell_policy=? WHERE id=?")
            .bind(policy)
            .bind(owner.id)
            .execute(&state.pool)
            .await
            .unwrap();
        for current in [
            actor(owner.id, "customer", "user"),
            actor(999, "admin", "admin"),
        ] {
            let payload = serde_json::from_value(json!({"tenant":owner.id.to_string()})).unwrap();
            let error = panel_api::terminal::handlers::create_session_handler(
                State(state.clone()),
                current,
                HeaderMap::new(),
                Json(payload),
            )
            .await
            .err()
            .unwrap();
            assert_eq!(error.0, StatusCode::FORBIDDEN);
            assert!(
                error.1.0["error"]
                    .as_str()
                    .unwrap()
                    .contains("Kebijakan shell")
            );
        }
    }
}
async fn tenant(
    state: &AppState,
    name: &str,
    linux: Option<&str>,
    uid: Option<i64>,
) -> panel_db::models::User {
    SqliteUserRepository::new(state.pool.clone())
        .create(&CreateUser {
            username: name.into(),
            password_hash: "unused".into(),
            role: "user".into(),
            email: None,
            linux_username: linux.map(str::to_owned),
            linux_uid: uid,
            plan_id: None,
        })
        .await
        .unwrap()
}
fn actor(id: i64, name: &str, role: &str) -> AuthenticatedUser {
    AuthenticatedUser {
        id,
        username: name.into(),
        role: role.into(),
        email: None,
        totp_enabled: false,
        session_id: "test".into(),
        csrf_token: "test".into(),
        impersonated_by: None,
    }
}

// A protocol fixture returns typed responses only; it never executes agent code.
async fn mock_rpc(
    tmp: &tempfile::TempDir,
    fail: &'static str,
) -> (
    Arc<AgentClient>,
    Arc<tokio::sync::Mutex<Vec<Value>>>,
    tokio::task::JoinHandle<()>,
) {
    let socket = tmp.path().join("fixture.sock");
    #[cfg(unix)]
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    #[cfg(not(unix))]
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    #[cfg(not(unix))]
    std::fs::write(&socket, listener.local_addr().unwrap().port().to_string()).unwrap();
    let calls = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let captured = calls.clone();
    let task = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let mut rpc = panel_proto::RpcStream::new(stream);
            let req = rpc.recv_request().await.unwrap();
            let mut value = serde_json::to_value(&req.payload).unwrap();
            value["type"] = value["cmd"].clone();
            captured.lock().await.push(value.clone());
            let kind = value["type"].as_str().unwrap();
            if fail.strip_prefix("Disconnect:") == Some(kind) {
                continue;
            }
            let data = &value["data"];
            let response = if fail.split('|').any(|operation| operation == kind) {
                panel_proto::RpcResponse::err(req.request_id, "Injected fixture failure")
            } else {
                let (response_kind, body) = match kind {
                    "PhpRuntimeList" => (
                        "PhpRuntimeList",
                        json!({"ppa_configured":true,"versions":[{"version":"8.3","installed":true,"fpm_binary_exists":true,"cli_binary_exists":true,"service_masked":true,"status":"installed","fpm_binary_path":"/usr/sbin/php-fpm8.3","cli_binary_path":"/usr/bin/php8.3","service_name":"php8.3-fpm"}]}),
                    ),
                    "SiteProvision" => (
                        "SiteProvisioned",
                        json!({"docroot":data["docroot"],"placeholder_written":true,"acls_applied":true}),
                    ),
                    "NodeSiteApply" => (
                        "NodeSiteApplied",
                        json!({"site_id":data["site_id"],"service_name":"panel-node-fixture","unit_path":"/fixture/node.service","env_path":"/fixture/node.env","port":data["port"],"active":true,"message":"fixture only"}),
                    ),
                    "PythonSiteApply" => (
                        "PythonSiteApplied",
                        json!({"site_id":data["site_id"],"service_name":"panel-python-fixture","unit_path":"/fixture/python.service","env_path":"/fixture/python.env","socket_path":data["socket_path"],"active":true,"message":"fixture only"}),
                    ),
                    "PhpFpmMasterEnsure" => (
                        "PhpFpmMasterEnsure",
                        json!({"service_name":"panel-php","config_dir":"/etc/panel/php","default_socket_path":"/run/panel/php/p_actual-8.3/fpm.sock","is_active":true}),
                    ),
                    "PhpFpmPoolApply" => (
                        "PhpFpmPoolApply",
                        json!({"pool_name":data["pool_name"],"pool_conf_path":"/etc/panel/php/pool.conf","socket_path":"/run/panel/php/p_actual-8.3/fpm.sock","reloaded":true}),
                    ),
                    "NodeSiteStatus" => (
                        "NodeSiteStatus",
                        json!({"site_id":data["site_id"],"service_name":"fixture","active":true,"sub_state":"running","healthy":true,"probe_status_code":200}),
                    ),
                    "PythonSiteStatus" => (
                        "PythonSiteStatus",
                        json!({"site_id":data["site_id"],"service_name":"fixture","active":true,"sub_state":"running","healthy":true,"socket_exists":true}),
                    ),
                    "NodeSiteRemove" => (
                        "NodeSiteRemoved",
                        json!({"site_id":data["site_id"],"action":"remove","success":true,"message":"removed"}),
                    ),
                    "PythonSiteRemove" => (
                        "PythonSiteRemoved",
                        json!({"site_id":data["site_id"],"action":"remove","success":true,"message":"removed"}),
                    ),
                    "SiteApply" => (
                        "SiteApplied",
                        json!({"site_id":data["site_id"],"available_path":"/etc/nginx/sites-available/site","enabled_path":"/etc/nginx/sites-enabled/site","enabled":true,"reloaded":true}),
                    ),
                    "PhpFpmHealthCheck" => (
                        "PhpFpmHealthChecked",
                        json!({"healthy":true,"service_active":true,"socket_exists":true,"pool_exists":true,"message":"healthy"}),
                    ),
                    "SiteRemove" => (
                        "SiteRemoved",
                        json!({"site_id":data["site_id"],"reloaded":true}),
                    ),
                    "PhpFpmPoolRemove" => (
                        "PhpFpmPoolRemove",
                        json!({"pool_name":data["pool_name"],"removed":true,"remaining_pools":0,"master_stopped":true,"reloaded":false}),
                    ),
                    _ => panic!("Unexpected host operation: {kind}"),
                };
                panel_proto::RpcResponse::ok(
                    req.request_id,
                    serde_json::from_value(json!({"type":response_kind,"data":body})).unwrap(),
                )
            };
            rpc.send_response(&response).await.unwrap();
        }
    });
    (Arc::new(AgentClient::new(socket)), calls, task)
}

#[tokio::test]
async fn runtime_provisioning_errors_keep_json_details_and_cleanup_instead_of_502() {
    use axum::{Json, response::IntoResponse};
    for (kind, failure) in [
        ("nodejs", "NodeSiteApply"),
        ("python", "PythonSiteApply"),
        ("nodejs", "NodeSiteStatus"),
        ("python", "PythonSiteStatus"),
        ("nodejs", "Disconnect:NodeSiteApply"),
        ("python", "Disconnect:PythonSiteApply"),
        ("nodejs", "Disconnect:NodeSiteStatus"),
        ("python", "Disconnect:PythonSiteStatus"),
    ] {
        let (mut state, tmp) = fixture().await;
        let owner = tenant(&state, "customer", Some("p_actual"), Some(10042)).await;
        let (client, calls, task) = mock_rpc(&tmp, failure).await;
        state.agent_client = client;
        let request = serde_json::from_value(json!({
            "domain":"failure.example.test", "user_id":owner.id, "kind":kind
        }))
        .unwrap();
        let response = panel_api::sites::create_static_site_handler(
            State(state.clone()),
            actor(999, "admin", "admin"),
            Json(request),
        )
        .await
        .into_response();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 16 * 1024)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        let disconnected = failure.starts_with("Disconnect:");
        assert_eq!(
            status,
            if disconnected {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            },
            "{failure}: {body}"
        );
        assert_eq!(body["code"], "site_provisioning_failed");
        let detail = body["error"].as_str().unwrap();
        assert!(
            detail.contains(if kind == "nodejs" {
                "Node.js"
            } else {
                "Python"
            }),
            "{body}"
        );
        if disconnected {
            assert!(!detail.contains("fixture.sock"));
            assert!(detail.contains("Agent tidak dapat dihubungi"));
        } else {
            assert!(detail.contains("Injected fixture failure"), "{body}");
        }
        let calls = calls.lock().await;
        assert!(!calls.iter().any(|call| call["type"] == "SiteApply"));
        assert!(calls.iter().any(|call| call["type"] == "SiteRemove"));
        assert!(calls.iter().any(|call| call["type"]
            == if kind == "nodejs" {
                "NodeSiteRemove"
            } else {
                "PythonSiteRemove"
            }));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sites")
                .fetch_one(&state.pool)
                .await
                .unwrap(),
            0
        );
        task.abort();
    }
}

#[tokio::test]
async fn node_and_python_create_and_reapply_send_valid_optional_nginx_directives() {
    use axum::{Json, response::IntoResponse};
    use panel_db::repositories::{SqliteDomainRepository, SqliteSiteRepository};
    for kind in ["nodejs", "python"] {
        let (mut state, tmp) = fixture().await;
        let owner = tenant(&state, "customer", Some("p_actual"), Some(10042)).await;
        let (client, calls, task) = mock_rpc(&tmp, "").await;
        state.agent_client = client;
        let request = serde_json::from_value(
            json!({"domain":"app.example.test","user_id":owner.id,"kind":kind,"port":20042}),
        )
        .unwrap();
        let response = panel_api::sites::create_static_site_handler(
            State(state.clone()),
            actor(999, "admin", "admin"),
            Json(request),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::CREATED, "{kind}");
        let sites = SqliteSiteRepository::new(state.pool.clone())
            .find_by_user_id(owner.id)
            .await
            .unwrap();
        assert_eq!(sites.len(), 1);
        let site = &sites[0];
        let domains = SqliteDomainRepository::new(state.pool.clone())
            .find_by_site_id(site.id)
            .await
            .unwrap();
        // Existing sites with JSON nulls use the same defaults on regeneration.
        let mut site = site.clone();
        let mut settings: Value = serde_json::from_str(&site.settings_json).unwrap();
        for field in [
            "upload_max_size",
            "proxy_connect_timeout",
            "proxy_send_timeout",
            "proxy_read_timeout",
            "proxy_buffering",
        ] {
            settings[field] = Value::Null;
        }
        site.settings_json = settings.to_string();
        panel_api::sites::recompile_and_apply_site_vhost(&state, &site, &owner, &domains)
            .await
            .unwrap();
        let captured = calls.lock().await;
        let vhosts: Vec<_> = captured
            .iter()
            .filter(|c| c["type"] == "SiteApply")
            .collect();
        assert_eq!(vhosts.len(), 2);
        for request in vhosts {
            assert_eq!(request["data"]["test_config"], true);
            assert_eq!(request["data"]["reload"], true);
            let config = request["data"]["config_content"].as_str().unwrap();
            for directive in [
                "client_max_body_size 100M;",
                "proxy_connect_timeout 60s;",
                "proxy_send_timeout 60s;",
                "proxy_read_timeout 60s;",
                "proxy_buffering on;",
            ] {
                assert!(config.contains(directive), "{kind}: missing {directive}");
            }
        }
        let runtime = if kind == "nodejs" {
            "NodeSiteApply"
        } else {
            "PythonSiteApply"
        };
        let runtime_request = captured
            .iter()
            .find(|call| call["type"] == runtime)
            .unwrap();
        assert_eq!(runtime_request["data"]["username"], "p_actual");
        assert_eq!(runtime_request["data"]["uid"], 10042);
        task.abort();
    }
}

#[tokio::test]
async fn php_presets_use_installed_runtime_authoritative_identity_and_health_check() {
    use axum::{Json, response::IntoResponse};
    for preset in ["generic", "wordpress", "laravel", "custom"] {
        let (mut state, tmp) = fixture().await;
        let owner = tenant(&state, "customer", Some("p_actual"), Some(10042)).await;
        let (client, calls, task) = mock_rpc(&tmp, "").await;
        state.agent_client = client;
        let payload=serde_json::from_value(json!({"domain":"example.test","user_id":owner.id,"kind":"php","php_version":"8.3","preset":preset,"custom_entry_file":"index.php"})).unwrap();
        let response = panel_api::sites::create_static_site_handler(
            State(state.clone()),
            actor(999, "admin", "admin"),
            Json(payload),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::CREATED, "{preset}");
        let calls = calls.lock().await;
        assert!(calls.iter().any(|c| c["type"] == "PhpFpmHealthCheck"));
        let health = calls
            .iter()
            .position(|c| c["type"] == "PhpFpmHealthCheck")
            .unwrap();
        let vhost = calls.iter().position(|c| c["type"] == "SiteApply").unwrap();
        assert!(
            health < vhost,
            "FPM must be ready before publishing the vhost"
        );
        let provision = calls.iter().find(|c| c["type"] == "SiteProvision").unwrap();
        assert_eq!(provision["data"]["username"], "p_actual");
        assert_eq!(provision["data"]["uid"], 10042);
        if preset == "laravel" {
            assert!(
                provision["data"]["docroot"]
                    .as_str()
                    .unwrap()
                    .ends_with("/app/public")
            );
        }
        task.abort();
    }
}

#[tokio::test]
async fn php_provisioning_failures_remove_site_and_pool_instead_of_reporting_success() {
    use axum::{Json, response::IntoResponse};
    for failure in [
        "SiteProvision",
        "PhpFpmMasterEnsure",
        "PhpFpmPoolApply",
        "SiteApply",
        "PhpFpmHealthCheck",
    ] {
        let (mut state, tmp) = fixture().await;
        let owner = tenant(&state, "customer", Some("p_actual"), Some(10042)).await;
        let (client, calls, task) = mock_rpc(&tmp, failure).await;
        state.agent_client = client;
        let payload = serde_json::from_value(
            json!({"domain":"example.test","user_id":owner.id,"kind":"php","php_version":"8.3"}),
        )
        .unwrap();
        let response = panel_api::sites::create_static_site_handler(
            State(state.clone()),
            actor(999, "admin", "admin"),
            Json(payload),
        )
        .await
        .into_response();
        assert_eq!(
            response.status(),
            if failure == "SiteApply" {
                StatusCode::INTERNAL_SERVER_ERROR
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            },
            "{failure}"
        );
        let body = axum::body::to_bytes(response.into_body(), 16 * 1024)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .contains("Injected fixture failure"),
            "{body}"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sites")
                .fetch_one(&state.pool)
                .await
                .unwrap(),
            0
        );
        let calls = calls.lock().await;
        if failure != "SiteApply" {
            assert!(!calls.iter().any(|call| call["type"] == "SiteApply"));
        }
        assert!(calls.iter().any(|c| c["type"] == "SiteRemove"));
        assert!(calls.iter().any(|c| c["type"] == "PhpFpmPoolRemove"));
        task.abort();
    }
}
#[tokio::test]
async fn context_uses_database_identity_and_enforces_selection_and_rbac() {
    let (state, _tmp) = fixture().await;
    let first = tenant(&state, "customer", Some("p_actual"), Some(10042)).await;
    let other = tenant(&state, "other", Some("p_other"), Some(10043)).await;
    let own = actor(first.id, "customer", "user");
    let admin = actor(999, "admin", "admin");
    let context = resolve_tenant_context(&state, &own, None, None)
        .await
        .unwrap();
    assert_eq!(context.linux_username, "p_actual");
    assert_eq!(context.uid, 10042);
    let site = panel_db::repositories::SqliteSiteRepository::new(state.pool.clone())
        .create(&panel_db::models::CreateSite {
            user_id: first.id,
            kind: "static".into(),
            docroot: "/home/p_actual/sites/example.test/public".into(),
            runtime: None,
            version: None,
            status: None,
            settings_json: None,
        })
        .await
        .unwrap();
    assert_eq!(
        resolve_tenant_context(&state, &admin, None, Some(site.id))
            .await
            .unwrap()
            .uid,
        10042
    );
    assert_eq!(
        resolve_tenant_context(&state, &admin, Some(&other.id.to_string()), Some(site.id))
            .await
            .unwrap_err()
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        resolve_tenant_context(
            &state,
            &actor(other.id, "other", "user"),
            None,
            Some(site.id)
        )
        .await
        .unwrap_err()
        .0,
        StatusCode::FORBIDDEN
    );
    let mut invalid = first.clone();
    invalid.linux_uid = Some(0);
    assert_eq!(
        panel_api::tenant_context::execution_identity(&invalid)
            .unwrap_err()
            .0,
        StatusCode::BAD_REQUEST
    );
    invalid = first.clone();
    invalid.status = "suspended".into();
    assert_eq!(
        panel_api::tenant_context::execution_identity(&invalid)
            .unwrap_err()
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        resolve_tenant_context(&state, &admin, None, None)
            .await
            .unwrap_err()
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        resolve_tenant_context(&state, &own, Some(&other.id.to_string()), None)
            .await
            .unwrap_err()
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        resolve_tenant_context(&state, &admin, Some(&first.id.to_string()), None)
            .await
            .unwrap()
            .linux_username,
        "p_actual"
    );
    sqlx::query("UPDATE users SET linux_username='p_admin' WHERE id=?")
        .bind(first.id)
        .execute(&state.pool)
        .await
        .unwrap();
    assert_eq!(
        resolve_tenant_context(&state, &own, None, None)
            .await
            .unwrap_err()
            .0,
        StatusCode::BAD_REQUEST
    );
    sqlx::query("UPDATE users SET linux_username=NULL WHERE id=?")
        .bind(first.id)
        .execute(&state.pool)
        .await
        .unwrap();
    assert_eq!(
        resolve_tenant_context(&state, &own, None, None)
            .await
            .unwrap_err()
            .0,
        StatusCode::BAD_REQUEST
    );
}
#[tokio::test]
async fn metrics_distinguish_waiting_ready_stale_failed_and_forbidden() {
    let (state, _tmp) = fixture().await;
    let storage = panel_analytics::AnalyticsStorage::connect_in_memory()
        .await
        .unwrap();
    let state = state.with_analytics(Arc::new(storage.clone()));
    let admin = actor(999, "admin", "admin");
    assert_eq!(
        panel_api::telemetry::current_metrics(State(state.clone()), admin.clone())
            .await
            .unwrap()
            .0
            .state,
        "waiting"
    );
    let mut point:panel_analytics::ServerMetricsPoint=serde_json::from_value(json!({"timestamp":chrono::Utc::now().timestamp(),"cpu_percent":37.5,"cpu_cores":4,"memory_used_bytes":512,"memory_total_bytes":1024,"swap_used_bytes":0,"swap_total_bytes":0,"load1":0.5,"load5":0.4,"load15":0.3,"disk_used_bytes":4096,"disk_total_bytes":8192,"inodes_used":1,"inodes_total":2,"net_rx_bytes_sec":12.0,"net_tx_bytes_sec":4.0,"io_read_bytes_sec":0.0,"io_write_bytes_sec":0.0})).unwrap();
    storage.insert_server_point(&point).await.unwrap();
    let result = panel_api::telemetry::current_metrics(State(state.clone()), admin.clone())
        .await
        .unwrap()
        .0;
    assert_eq!(result.state, "ready");
    assert_eq!(result.sample.unwrap(), point);
    sqlx::query("DELETE FROM metrics_server_raw")
        .execute(storage.write_pool())
        .await
        .unwrap();
    point.timestamp -= 120;
    storage.insert_server_point(&point).await.unwrap();
    assert_eq!(
        panel_api::telemetry::current_metrics(State(state.clone()), admin.clone())
            .await
            .unwrap()
            .0
            .state,
        "stale"
    );
    state.collector_status.write().await.failed = true;
    assert_eq!(
        panel_api::telemetry::current_metrics(State(state.clone()), admin)
            .await
            .unwrap()
            .0
            .state,
        "failed"
    );
    assert_eq!(
        panel_api::telemetry::current_metrics(State(state), actor(1, "tenant", "user"))
            .await
            .err()
            .unwrap()
            .0,
        StatusCode::FORBIDDEN
    );
}
#[tokio::test]
async fn spa_deep_links_work_without_swallowing_services_or_missing_assets() {
    use axum::response::IntoResponse;
    for (path, status) in [
        ("/sites/7/settings/runtime", StatusCode::OK),
        ("/tenants/3", StatusCode::OK),
        ("/api/v1/unknown", StatusCode::NOT_FOUND),
        ("/ws/unknown", StatusCode::NOT_FOUND),
        ("/assets/missing", StatusCode::NOT_FOUND),
        ("/fonts/missing.woff2", StatusCode::NOT_FOUND),
    ] {
        let response = panel_api::static_files::static_handler(path.parse().unwrap())
            .await
            .into_response();
        assert_eq!(response.status(), status, "{path}");
    }
}

#[tokio::test]
async fn collector_worker_persists_proc_fixture_and_stops_cleanly() {
    use panel_analytics::{
        AnalyticsStorage, CgroupMetricsCollector, DiskUsageReader, ProcReader, RetentionConfig,
        SystemMetricsCollector,
    };
    let (state, tmp) = fixture().await;
    let proc = tmp.path().join("proc");
    std::fs::create_dir_all(proc.join("net")).unwrap();
    for (name, content) in [
        (
            "stat",
            "cpu 500 100 200 4200 0 0 0 0 0 0\ncpu0 500 100 200 4200 0 0 0 0 0 0\n",
        ),
        (
            "meminfo",
            "MemTotal: 8192000 kB\nMemAvailable: 4096000 kB\nMemFree: 2048000 kB\nSwapTotal: 1024000 kB\nSwapFree: 512000 kB\n",
        ),
        ("loadavg", "0.50 0.80 0.90 2/200 9999\n"),
        (
            "net/dev",
            "hdr\nhdr2\neth0: 1048576 100 0 0 0 0 0 0 524288 50 0 0 0 0 0 0\n",
        ),
        ("diskstats", "8 0 sda 10 0 2048 0 20 0 4096 0 0 0 0\n"),
    ] {
        std::fs::write(proc.join(name), content).unwrap();
    }
    let storage = AnalyticsStorage::connect(tmp.path().join("metrics.db"))
        .await
        .unwrap();
    let disk = DiskUsageReader::new(tmp.path());
    disk.set_mock_stats(panel_analytics::disk::DiskStats {
        disk_total_bytes: 8192,
        disk_used_bytes: 4096,
        inodes_total: 100,
        inodes_used: 10,
    });
    let collector = SystemMetricsCollector::new(
        Arc::new(ProcReader::new(&proc)),
        Arc::new(CgroupMetricsCollector::new(tmp.path().join("cgroups"))),
        Arc::new(disk),
        storage.clone(),
        RetentionConfig::default(),
    );
    let (stop, rx) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(panel_api::telemetry::run_collector(
        state.clone(),
        collector,
        rx,
    ));
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while state
            .collector_status
            .read()
            .await
            .last_success_at
            .is_none()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    stop.send(true).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    let point = storage.latest_server_point().await.unwrap().unwrap();
    assert_eq!(point.memory_total_bytes, 8192000 * 1024);
    assert_eq!(point.disk_total_bytes, 8192);
    assert_eq!(point.load1, 0.5);
    assert!(!state.collector_status.read().await.failed);
}

#[tokio::test]
async fn same_domain_can_be_deleted_and_recreated_across_runtimes_with_matching_starters() {
    use axum::{Json, response::IntoResponse};
    use panel_db::repositories::SqliteSiteRepository;
    let (mut state, tmp) = fixture().await;
    let owner = tenant(&state, "customer", Some("p_actual"), Some(10042)).await;
    let (client, calls, task) = mock_rpc(&tmp, "").await;
    state.agent_client = client;
    for (kind, starter) in [
        ("static", "html"),
        ("nodejs", "nodejs"),
        ("python", "python_app"),
        ("php", "php"),
    ] {
        let request = serde_json::from_value(json!({"domain":"reused.example.test","user_id":owner.id,"kind":kind,"php_version":"8.3"})).unwrap();
        let response = panel_api::sites::create_static_site_handler(
            State(state.clone()),
            actor(999, "admin", "admin"),
            Json(request),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::CREATED, "{kind}");
        let site = SqliteSiteRepository::new(state.pool.clone())
            .find_by_user_id(owner.id)
            .await
            .unwrap()
            .pop()
            .unwrap();
        {
            let captured = calls.lock().await;
            let provision = captured
                .iter()
                .rev()
                .find(|c| c["type"] == "SiteProvision")
                .unwrap();
            assert_eq!(provision["data"]["starter"], starter);
            if kind == "nodejs" || kind == "python" {
                let ready = if kind == "nodejs" {
                    "NodeSiteStatus"
                } else {
                    "PythonSiteStatus"
                };
                let health = captured.iter().rposition(|c| c["type"] == ready).unwrap();
                let publish = captured
                    .iter()
                    .rposition(|c| c["type"] == "SiteApply")
                    .unwrap();
                assert!(health < publish, "publish only after {kind} is ready");
            }
        }
        let response = panel_api::sites::delete_site_handler(
            State(state.clone()),
            actor(999, "admin", "admin"),
            axum::extract::Path(site.id),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::OK, "{kind}");
        assert!(
            SqliteSiteRepository::new(state.pool.clone())
                .find_by_user_id(owner.id)
                .await
                .unwrap()
                .is_empty()
        );
    }
    let captured = calls.lock().await;
    assert!(captured.iter().any(|c| c["type"] == "NodeSiteRemove"));
    let removal = captured
        .iter()
        .find(|c| c["type"] == "PythonSiteRemove")
        .unwrap();
    assert!(
        removal["data"]["socket_path"]
            .as_str()
            .unwrap()
            .ends_with("/app.sock")
    );
    task.abort();
}

#[tokio::test]
async fn failed_runtime_health_never_publishes_and_failed_cleanup_retains_domain() {
    use axum::{Json, response::IntoResponse};
    use panel_db::repositories::SqliteSiteRepository;
    for kind in ["nodejs", "python"] {
        let (mut state, tmp) = fixture().await;
        let owner = tenant(&state, "customer", Some("p_actual"), Some(10042)).await;
        let failure = if kind == "nodejs" {
            "NodeSiteStatus|NodeSiteRemove"
        } else {
            "PythonSiteStatus|PythonSiteRemove"
        };
        let (client, calls, task) = mock_rpc(&tmp, failure).await;
        state.agent_client = client;
        let request = serde_json::from_value(
            json!({"domain":"failed.example.test","user_id":owner.id,"kind":kind}),
        )
        .unwrap();
        let response = panel_api::sites::create_static_site_handler(
            State(state.clone()),
            actor(999, "admin", "admin"),
            Json(request),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!calls.lock().await.iter().any(|c| c["type"] == "SiteApply"));
        let sites = SqliteSiteRepository::new(state.pool.clone())
            .find_by_user_id(owner.id)
            .await
            .unwrap();
        assert_eq!(sites.len(), 1, "retain ownership until cleanup succeeds");
        assert_eq!(sites[0].status, "suspended");
        task.abort();
    }
}

#[tokio::test]
async fn failed_python_delete_does_not_free_the_domain() {
    use axum::{Json, response::IntoResponse};
    use panel_db::repositories::SqliteSiteRepository;
    let (mut state, tmp) = fixture().await;
    let owner = tenant(&state, "customer", Some("p_actual"), Some(10042)).await;
    let (client, _, task) = mock_rpc(&tmp, "PythonSiteRemove").await;
    state.agent_client = client;
    let request = serde_json::from_value(
        json!({"domain":"kept.example.test","user_id":owner.id,"kind":"python"}),
    )
    .unwrap();
    let response = panel_api::sites::create_static_site_handler(
        State(state.clone()),
        actor(999, "admin", "admin"),
        Json(request),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::CREATED);
    let site = SqliteSiteRepository::new(state.pool.clone())
        .find_by_user_id(owner.id)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let response = panel_api::sites::delete_site_handler(
        State(state.clone()),
        actor(999, "admin", "admin"),
        axum::extract::Path(site.id),
    )
    .await
    .into_response();
    assert!(response.status().is_client_error() || response.status().is_server_error());
    assert!(
        SqliteSiteRepository::new(state.pool.clone())
            .find_by_id(site.id)
            .await
            .unwrap()
            .is_some()
    );
    task.abort();
}

#[tokio::test]
async fn python_presets_seed_entrypoints_with_the_selected_wsgi_or_asgi_protocol() {
    use axum::{Json, response::IntoResponse};
    for (framework, entry, protocol, starter) in [
        (
            "django",
            "myproject.wsgi:application",
            "wsgi",
            "python_django",
        ),
        ("flask", "app:app", "wsgi", "python_app"),
        ("fastapi", "main:app", "asgi", "python_main"),
    ] {
        let (mut state, tmp) = fixture().await;
        let owner = tenant(&state, "customer", Some("p_actual"), Some(10042)).await;
        let (client, calls, task) = mock_rpc(&tmp, "").await;
        state.agent_client = client;
        let request = serde_json::from_value(json!({"domain":"preset.example.test","user_id":owner.id,"kind":"python","framework":framework,"entrypoint_value":entry,"entrypoint_type":protocol})).unwrap();
        let response = panel_api::sites::create_static_site_handler(
            State(state.clone()),
            actor(999, "admin", "admin"),
            Json(request),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::CREATED, "{framework}");
        let captured = calls.lock().await;
        let provision = captured
            .iter()
            .find(|c| c["type"] == "SiteProvision")
            .unwrap();
        assert_eq!(provision["data"]["starter"], starter);
        let code = provision["data"]["index_content"].as_str().unwrap();
        assert_eq!(code.contains("async def app"), protocol == "asgi");
        assert!(code.contains("application = app"));
        task.abort();
    }
}
