#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Hermetic release metadata, SQLite, and router tests; no AgentServer or installers.
use chrono::NaiveDate;
use panel_api::runtime_catalog::*;
use panel_runtime::node::{NodeLtsStatus, NodeVersionEntry};
use serde_json::{Value, json};

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 18).unwrap()
}
fn release(version: &str) -> NodeVersionEntry {
    NodeVersionEntry {
        version: version.into(),
        date: Some("2026-10-01".into()),
        files: vec![],
        lts: NodeLtsStatus::Codename("Test".into()),
        npm: None,
        v8: None,
        uv: None,
        security: None,
    }
}
fn node_fixture() -> NodeCatalog {
    let mut releases = vec![
        release("v20.99.0"),
        release("v26.0.0"),
        release("v22.99.0-rc.1"),
        release("v50.0.0"),
    ];
    for major in [22, 24] {
        for patch in 1..=12 {
            releases.push(release(&format!("v{major}.1.{patch}")));
        }
    }
    releases.push(release("v24.1.12"));
    let schedule = serde_json::from_value(json!({
        "v20":{"start":"2023-04-01","lts":"2023-10-01","end":"2026-04-30"},
        "v22":{"start":"2024-04-01","lts":"2024-10-01","end":"2027-04-30"},
        "v24":{"start":"2025-04-01","lts":"2025-10-01","end":"2028-04-30"},
        "v26":{"start":"2026-05-01","lts":"2026-10-28","end":"2029-04-30"}
    }))
    .unwrap();
    NodeCatalog { releases, schedule }
}

#[test]
fn node_choices_keep_five_numeric_patches_per_supported_lts_line() {
    let catalog = node_fixture();
    let choices = node_choices(&catalog, today());
    assert_eq!(choices.len(), 10);
    assert_eq!(choices[0].version, "v24.1.12");
    assert_eq!(choices[4].version, "v24.1.8");
    assert_eq!(choices[5].version, "v22.1.12");
    assert!(!choices.iter().any(|v| v.version.contains("rc")
        || v.version.starts_with("v20")
        || v.version.starts_with("v26")));
    let later = node_choices(&catalog, NaiveDate::from_ymd_opt(2027, 5, 1).unwrap());
    assert!(
        !later.iter().any(|v| v.version.starts_with("v22")),
        "EOL is reevaluated from cached metadata"
    );
}

#[test]
fn future_release_dates_and_invalid_semver_are_never_choices() {
    let mut catalog = node_fixture();
    let mut future = release("v24.99.0");
    future.date = Some("2027-01-01".into());
    catalog.releases.push(future);
    assert!(
        !node_choices(&catalog, today())
            .iter()
            .any(|v| v.version == "v24.99.0")
    );
    for invalid in ["24.0.1-rc.1", "24.0", "24.0.1.2", "24.0.1/../etc", "--help"] {
        assert!(version_key(invalid).is_none());
    }
}

#[test]
fn python_catalog_uses_latest_stable_supported_patch() {
    let cycles = json!({"metadata":{
        "3.10":{"status":"end-of-life","first_release":"2021-10-01","end_of_life":"2026-10-01"},
        "3.14":{"status":"bugfix","first_release":"2025-10-01","end_of_life":"2030-10-01"},
        "3.15":{"status":"bugfix","first_release":"2026-10-09","end_of_life":"2031-10-01"},
        "3.16":{"status":"prerelease","first_release":"2027-10-01","end_of_life":"2032-10-01"}
    }});
    let item = |minor, patch, prerelease: Value| json!({"name":"cpython","os":"linux","libc":"gnu","arch":{"family":std::env::consts::ARCH},"major":3,"minor":minor,"patch":patch,"prerelease":prerelease,"variant":null});
    let mut wrong_arch = item(14, 99, Value::Null);
    wrong_arch["arch"]["family"] = json!("unsupported-arch");
    // Astral's stable downloads use an empty string, not null, for prerelease.
    let downloads = json!({"old":item(10,99,json!("")),"a":item(14,2,json!("")),"b":item(14,10,json!("")),"c":item(15,0,json!("")),"pre":item(16,0,json!("a1")),"wrong_arch":wrong_arch});
    let versions = parse_python(downloads, &cycles, today()).unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0].version, "3.15.0");
    assert!(versions[0].is_default);
    assert_eq!(versions[1].version, "3.14.10");
    assert!(parse_python(json!({}), &cycles, today()).is_err());
}

#[test]
fn python_catalog_accepts_stable_markers_and_rejects_prerelease_or_invalid_markers() {
    let cycles = json!({"metadata":{
        "3.14":{"status":"bugfix","first_release":"2025-10-07","end_of_life":"2030-10-01"}
    }});
    let item = json!({"name":"cpython","os":"linux","libc":"gnu",
        "arch":{"family":std::env::consts::ARCH,"variant":null},
        "major":3,"minor":14,"patch":8,"variant":null});
    for marker in [Some(Value::Null), Some(json!("")), None] {
        let mut download = item.clone();
        if let Some(marker) = marker {
            download["prerelease"] = marker;
        }
        let versions = parse_python(json!({"stable":download}), &cycles, today()).unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].version, "3.14.8");
        assert!(versions[0].is_default);
        assert!(versions[0].is_lts_or_stable);
    }
    for marker in [
        json!("a1"),
        json!("b1"),
        json!("rc1"),
        json!(" "),
        json!(false),
        json!(0),
        json!([]),
        json!({}),
    ] {
        let mut download = item.clone();
        download["prerelease"] = marker.clone();
        assert!(
            parse_python(json!({"unstable":download}), &cycles, today()).is_err(),
            "Unexpected stable marker: {marker}"
        );
    }
}

#[test]
fn php_catalog_discovers_new_supported_branches_and_patches() {
    let versions =
        parse_php(json!({"8":{"8.4":{"version":"8.4.26"},"8.5":{"version":"8.5.11"}}})).unwrap();
    assert_eq!(versions.len(), 2);
    assert!(
        versions
            .iter()
            .any(|v| v.branch == "8.5" && v.latest == "8.5.11")
    );
    let version = panel_runtime::php::PhpVersion::parse("8.5").unwrap();
    assert!(version.base_packages().contains(&"php8.5-fpm".into()));
    assert_eq!(
        version.fpm_binary_path().to_string_lossy(),
        "/usr/sbin/php-fpm8.5"
    );
    assert!(parse_php(json!({})).is_err());
}

#[test]
fn python_compact_release_cycles_accept_month_eol_and_expire_conservatively() {
    // Same shape as peps.python.org/api/release-cycle.json, including YYYY-MM EOL.
    let mut cycles =
        json!({"3.14":{"status":"bugfix","first_release":"2025-10-07","end_of_life":"2030-10"}});
    let downloads = json!({"stable":{"name":"cpython","os":"linux","libc":"gnu",
        "arch":{"family":std::env::consts::ARCH,"variant":null},
        "major":3,"minor":14,"patch":8,"prerelease":"","variant":null}});
    let versions = parse_python(downloads.clone(), &cycles, today()).unwrap();
    assert_eq!(versions[0].version, "3.14.8");
    assert!(
        parse_python(
            downloads.clone(),
            &cycles,
            NaiveDate::from_ymd_opt(2030, 10, 1).unwrap()
        )
        .is_err()
    );
    assert!(
        parse_python(
            downloads.clone(),
            &cycles,
            NaiveDate::from_ymd_opt(2030, 9, 30).unwrap()
        )
        .is_ok()
    );
    for invalid in ["2030", "2030-13", "2030-02-30", "2030-10-extra"] {
        cycles["3.14"]["end_of_life"] = json!(invalid);
        assert!(
            parse_python(downloads.clone(), &cycles, today()).is_err(),
            "{invalid}"
        );
    }
    cycles["3.14"]["end_of_life"] = json!("2030-10");
    for status in ["feature", "prerelease", "end-of-life"] {
        cycles["3.14"]["status"] = json!(status);
        assert!(parse_python(downloads.clone(), &cycles, today()).is_err());
    }
}

#[tokio::test]
async fn python_inventory_recovers_from_static_fallback_after_catalog_sync() {
    let (state, app, cookie, _csrf, tmp) = router_fixture("admin").await;
    let socket = tmp.path().join("no-agent");
    #[cfg(unix)]
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    #[cfg(not(unix))]
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    #[cfg(not(unix))]
    std::fs::write(&socket, listener.local_addr().unwrap().port().to_string()).unwrap();
    let fixture = tokio::spawn(async move {
        for _ in 0..3 {
            let (stream, _) = listener.accept().await.unwrap();
            let mut rpc = panel_proto::RpcStream::new(stream);
            let request = rpc.recv_request().await.unwrap();
            assert!(matches!(
                request.payload,
                panel_proto::RequestPayload::PythonRuntimeList(_)
            ));
            rpc.send_response(&panel_proto::RpcResponse::ok(
                request.request_id,
                panel_proto::ResponsePayload::PythonRuntimeList(
                    panel_proto::PythonRuntimeListResponse { installed: vec![] },
                ),
            ))
            .await
            .unwrap();
        }
    });
    let (status, before) = call(
        &app,
        &cookie,
        None,
        "GET",
        "/api/v1/runtimes/python/versions",
        json!(null),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(before["catalog_ready"], false);
    assert!(
        before["versions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|version| version["version"] == "3.12.9")
    );
    let downloads = json!({"stable":{"name":"cpython","os":"linux","libc":"gnu",
        "arch":{"family":std::env::consts::ARCH,"variant":null},
        "major":3,"minor":15,"patch":0,"prerelease":"","variant":null}});
    let cycles =
        json!({"3.15":{"status":"bugfix","first_release":"2026-10-09","end_of_life":"2031-10"}});
    let catalog = parse_python(downloads, &cycles, today()).unwrap();
    let now = chrono::Utc::now().timestamp();
    store_refresh(&state.pool, "runtime.catalog.python", Ok(catalog), now)
        .await
        .unwrap();
    for failed_refresh in [false, true] {
        if failed_refresh {
            store_refresh::<Vec<panel_runtime::python::PythonVersionEntry>>(
                &state.pool,
                "runtime.catalog.python",
                Err("Status dukungan Python (PEP): batas waktu unduhan terlampaui".into()),
                now,
            )
            .await
            .unwrap();
        }
        let (status, body) = call(
            &app,
            &cookie,
            None,
            "GET",
            "/api/v1/runtimes/python/versions",
            json!(null),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["catalog_ready"], true);
        assert_eq!(body["versions"].as_array().unwrap().len(), 1);
        assert_eq!(body["versions"][0]["version"], "3.15.0");
        assert_eq!(body["versions"][0]["is_installed"], false);
    }
    fixture.await.unwrap();
}

#[tokio::test]
async fn refresh_failure_preserves_last_good_cache_and_schedule_across_reload() {
    let pool = panel_db::pool::init_pool("sqlite::memory:").await.unwrap();
    panel_db::pool::run_migrations(&pool).await.unwrap();
    store_refresh(&pool, "runtime.catalog.node", Ok(node_fixture()), 1000)
        .await
        .unwrap();
    store_refresh::<NodeCatalog>(
        &pool,
        "runtime.catalog.node",
        Err("provider down".into()),
        2000,
    )
    .await
    .unwrap();
    let saved = node_cache(&pool).await.unwrap();
    assert_eq!(saved.refreshed_at, Some(1000));
    assert_eq!(saved.checked_at, Some(2000));
    assert_eq!(saved.error.as_deref(), Some("provider down"));
    assert!(!saved.data.releases.is_empty());
    assert!(saved.fresh(2000));
    assert!(!saved.fresh(1000 + 8 * 86400));
    assert!(!due(saved.checked_at, 24, 2001));
    assert!(due(saved.checked_at, 24, 2000 + 86400));
    assert!(due(Some(3000), 24, 2000));
    save_setting(
        &pool,
        "runtime.catalog.schedule",
        &Schedule {
            enabled: false,
            interval_hours: 12,
        },
    )
    .await
    .unwrap();
    refresh(&pool, false).await.unwrap(); // Disabled: no network and no installation jobs.
    assert_eq!(node_cache(&pool).await.unwrap().checked_at, Some(2000));
    let loaded: Schedule = read_setting(&pool, "runtime.catalog.schedule")
        .await
        .unwrap();
    assert!(!loaded.enabled);
    assert_eq!(loaded.interval_hours, 12);
    let jobs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(jobs, 0);
}

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use panel_api::{
    auth::{CSRF_HEADER_NAME, LockoutManager, SESSION_COOKIE_NAME},
    config::AuthConfig,
    routes::AppState,
};
use panel_db::{models::CreateUser, repositories::SqliteUserRepository};
use std::sync::Arc;
use tower::ServiceExt;

async fn router_fixture(role: &str) -> (AppState, axum::Router, String, String, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp
        .path()
        .join("catalog.db")
        .to_string_lossy()
        .replace('\\', "/");
    let pool = panel_db::pool::init_pool(&format!("sqlite://{path}"))
        .await
        .unwrap();
    panel_db::pool::run_migrations(&pool).await.unwrap();
    SqliteUserRepository::new(pool.clone())
        .create(&CreateUser {
            username: "tester".into(),
            password_hash: panel_core::auth::hash_password("SecurePass123!").unwrap(),
            role: role.into(),
            email: None,
            plan_id: None,
            linux_username: None,
            linux_uid: None,
        })
        .await
        .unwrap();
    let state = AppState::new(
        Arc::new(panel_api::agent_client::AgentClient::new(
            tmp.path().join("no-agent"),
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
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"username":"tester","password":"SecurePass123!"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    assert!(cookie.starts_with(SESSION_COOKIE_NAME));
    let body: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    (
        state,
        app,
        cookie,
        body["csrf_token"].as_str().unwrap().into(),
        tmp,
    )
}
async fn call(
    app: &axum::Router,
    cookie: &str,
    csrf: Option<&str>,
    method: &str,
    path: &str,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("cookie", cookie)
        .header("content-type", "application/json");
    if let Some(csrf) = csrf {
        request = request.header(CSRF_HEADER_NAME, csrf);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(json!({"text":String::from_utf8_lossy(&bytes)})),
    )
}
#[tokio::test]
async fn admin_can_schedule_and_queue_python_once_without_agent_connection() {
    let (state, app, cookie, csrf, _tmp) = router_fixture("admin").await;
    store_refresh(
        &state.pool,
        "runtime.catalog.node",
        Ok(node_fixture()),
        chrono::Utc::now().timestamp(),
    )
    .await
    .unwrap();
    assert_eq!(
        call(
            &app,
            &cookie,
            Some(&csrf),
            "POST",
            "/api/v1/runtimes/node/install",
            json!({"version":"20.99.0"})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (status, body) = call(
        &app,
        &cookie,
        Some(&csrf),
        "PUT",
        "/api/v1/runtimes/catalog",
        json!({"enabled":false,"interval_hours":12}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["interval_hours"], 12);
    assert_eq!(
        call(
            &app,
            &cookie,
            Some(&csrf),
            "PUT",
            "/api/v1/runtimes/catalog",
            json!({"enabled":true,"interval_hours":0})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &app,
            &cookie,
            None,
            "PUT",
            "/api/v1/runtimes/catalog",
            json!({"enabled":true,"interval_hours":24})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, job) = call(
        &app,
        &cookie,
        Some(&csrf),
        "POST",
        "/api/v1/runtimes/python/install",
        json!({"version":"3.14.10"}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{job}");
    assert_eq!(job["name"], "python.install");
    assert_eq!(job["status"], "queued");
    let (_, again) = call(
        &app,
        &cookie,
        Some(&csrf),
        "POST",
        "/api/v1/runtimes/python/install",
        json!({"version":"3.14.10"}),
    )
    .await;
    assert_eq!(job["id"], again["id"]);
    for version in ["../../etc", "--help", "3.14.1/../../x"] {
        assert_eq!(
            call(
                &app,
                &cookie,
                Some(&csrf),
                "POST",
                "/api/v1/runtimes/python/install",
                json!({"version":version})
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE name='python.install'")
        .fetch_one(&state.operations_pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn python_install_job_executes_typed_rpc_and_persists_success() {
    let (state, app, cookie, csrf, tmp) = router_fixture("admin").await;
    let socket = tmp.path().join("no-agent");
    #[cfg(unix)]
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    #[cfg(not(unix))]
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    #[cfg(not(unix))]
    std::fs::write(&socket, listener.local_addr().unwrap().port().to_string()).unwrap();
    let fixture = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut rpc = panel_proto::RpcStream::new(stream);
        let request = rpc.recv_request().await.unwrap();
        match request.payload {
            panel_proto::RequestPayload::PythonRuntimeInstall(payload) => {
                assert_eq!(payload.version, "3.14.10");
                assert!(payload.base_dir.is_none());
            }
            other => panic!("Unexpected operation: {other:?}"),
        }
        rpc.send_response(&panel_proto::RpcResponse::ok(
            request.request_id,
            panel_proto::ResponsePayload::PythonRuntimeInstalled(
                panel_proto::PythonRuntimeActionResponse {
                    version: "3.14.10".into(),
                    success: true,
                    path: "/opt/panel/runtimes/python/3.14.10".into(),
                    message: "Mock installation complete".into(),
                },
            ),
        ))
        .await
        .unwrap();
    });
    let (status, job) = call(
        &app,
        &cookie,
        Some(&csrf),
        "POST",
        "/api/v1/runtimes/python/install",
        json!({"version":"3.14.10"}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            panel_api::jobs::process_one_job(&state.job_runner)
        )
        .await
        .unwrap()
        .unwrap()
    );
    fixture.await.unwrap();
    let stored = state
        .job_runner
        .job_repo
        .find_by_id(job["id"].as_str().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, "succeeded");
    assert_eq!(stored.progress, 100);
    let result: Value = serde_json::from_str(stored.result_json.as_ref().unwrap()).unwrap();
    assert_eq!(result["version"], "3.14.10");
}

#[tokio::test]
async fn node_inventory_retains_installed_eol_but_offers_only_supported_lts() {
    let (state, app, cookie, _csrf, tmp) = router_fixture("admin").await;
    let mut catalog = node_fixture();
    for release in &mut catalog.releases {
        release.date = Some("2000-01-01".into());
    }
    for (major, line) in &mut catalog.schedule {
        line.start = "2000-01-01".into();
        line.lts = Some("2000-01-01".into());
        line.end = if major == "v20" {
            "2001-01-01"
        } else {
            "2099-01-01"
        }
        .into();
    }
    store_refresh(
        &state.pool,
        "runtime.catalog.node",
        Ok(catalog),
        chrono::Utc::now().timestamp(),
    )
    .await
    .unwrap();
    let socket = tmp.path().join("no-agent");
    #[cfg(unix)]
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    #[cfg(not(unix))]
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    #[cfg(not(unix))]
    std::fs::write(&socket, listener.local_addr().unwrap().port().to_string()).unwrap();
    let fixture = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut rpc = panel_proto::RpcStream::new(stream);
        let request = rpc.recv_request().await.unwrap();
        assert!(matches!(
            request.payload,
            panel_proto::RequestPayload::NodeRuntimeList(_)
        ));
        let response=serde_json::from_value(json!({"type":"NodeRuntimeList","data":{"base_dir":"/opt/panel/runtimes/node","versions":[{
            "version":"20.99.0","raw_version":"v20.99.0","path":"/opt/panel/runtimes/node/20.99.0","node_binary":"/opt/panel/runtimes/node/20.99.0/bin/node","corepack_enabled":false,"is_lts":true,"lts_codename":"Iron"
        }]}})).unwrap();
        rpc.send_response(&panel_proto::RpcResponse::ok(request.request_id, response))
            .await
            .unwrap();
    });
    let (status, body) = call(
        &app,
        &cookie,
        None,
        "GET",
        "/api/v1/runtimes/node/versions",
        json!(null),
    )
    .await;
    fixture.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["installed_count"], 1);
    let versions = body["versions"].as_array().unwrap();
    let old = versions.iter().find(|v| v["version"] == "20.99.0").unwrap();
    assert_eq!(old["is_installed"], true);
    assert_eq!(old["support_status"], "eol");
    assert!(
        versions
            .iter()
            .filter(|v| v["is_installed"] == false)
            .all(|v| v["support_status"] == "lts")
    );
}
#[tokio::test]
async fn php_catalog_keeps_uninstalled_legacy_versions_available_for_manual_install() {
    for role in ["admin", "user"] {
        let (state, app, cookie, csrf, tmp) = router_fixture(role).await;
        store_refresh(
            &state.pool,
            "runtime.catalog.php",
            Ok(vec![PhpRelease {
                branch: "8.5".into(),
                latest: "8.5.11".into(),
            }]),
            chrono::Utc::now().timestamp(),
        )
        .await
        .unwrap();
        let socket = tmp.path().join("no-agent");
        #[cfg(unix)]
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        #[cfg(not(unix))]
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        #[cfg(not(unix))]
        std::fs::write(&socket, listener.local_addr().unwrap().port().to_string()).unwrap();
        let fixture = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut rpc = panel_proto::RpcStream::new(stream);
            let request = rpc.recv_request().await.unwrap();
            assert!(matches!(
                request.payload,
                panel_proto::RequestPayload::PhpRuntimeList(_)
            ));
            let inventory = panel_proto::PhpRuntimeListResponse {
                ppa_configured: true,
                versions: panel_runtime::php::PhpVersion::all_supported()
                    .iter()
                    .map(|version| panel_proto::PhpVersionStatusDto {
                        version: version.as_str().into(),
                        installed: false,
                        fpm_binary_exists: false,
                        cli_binary_exists: false,
                        service_masked: true,
                        status: "not_installed".into(),
                        fpm_binary_path: version.fpm_binary_path().to_string_lossy().into_owned(),
                        cli_binary_path: version.cli_binary_path().to_string_lossy().into_owned(),
                        service_name: version.fpm_service_name(),
                    })
                    .collect(),
            };
            rpc.send_response(&panel_proto::RpcResponse::ok(
                request.request_id,
                panel_proto::ResponsePayload::PhpRuntimeList(inventory),
            ))
            .await
            .unwrap();
        });
        let (status, body) = call(
            &app,
            &cookie,
            None,
            "GET",
            "/api/v1/php/versions",
            Value::Null,
        )
        .await;
        fixture.await.unwrap();
        assert_eq!(status, StatusCode::OK, "{body}");
        let versions = body["versions"].as_array().unwrap();
        let names: Vec<_> = versions
            .iter()
            .map(|version| version["version"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["7.4", "8.0", "8.1", "8.2", "8.3", "8.4", "8.5"]);
        let legacy = versions
            .iter()
            .find(|version| version["version"] == "7.4")
            .unwrap();
        assert_eq!(legacy["installed"], false);
        assert!(legacy["latest_release"].is_null());
        assert_eq!(versions.last().unwrap()["latest_release"], "8.5.11");
        let (status, job) = call(
            &app,
            &cookie,
            Some(&csrf),
            "POST",
            "/api/v1/php/versions/7.4/install",
            Value::Null,
        )
        .await;
        if role == "admin" {
            assert_eq!(status, StatusCode::ACCEPTED, "{job}");
            assert_eq!(job["name"], "php.install");
            assert_eq!(job["status"], "queued");
            let saved = state
                .job_runner
                .job_repo
                .find_by_id(job["id"].as_str().unwrap())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&saved.payload_json).unwrap()["version"],
                "7.4"
            );
        } else {
            assert_eq!(status, StatusCode::FORBIDDEN, "{job}");
        }
    }
}

#[tokio::test]
async fn tenant_cannot_install_python_or_mutate_catalog_schedule() {
    let (_state, app, cookie, csrf, _tmp) = router_fixture("user").await;
    assert_eq!(
        call(
            &app,
            &cookie,
            Some(&csrf),
            "POST",
            "/api/v1/runtimes/python/install",
            json!({"version":"3.14.10"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            &cookie,
            Some(&csrf),
            "PUT",
            "/api/v1/runtimes/catalog",
            json!({"enabled":true,"interval_hours":24})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            &cookie,
            Some(&csrf),
            "POST",
            "/api/v1/runtimes/catalog/refresh",
            json!({})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
}
