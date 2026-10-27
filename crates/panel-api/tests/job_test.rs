#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use panel_api::agent_client::AgentClient;
use panel_api::auth::{LockoutManager, SESSION_COOKIE_NAME};
use panel_api::config::AuthConfig;
use panel_api::create_router;
use panel_api::jobs::{JobItem, JobListResponse, JobRunner, process_one_job};
use panel_api::routes::AppState;
use panel_core::auth::hash_password;
use panel_db::models::{CreateJob, CreateUser};
use panel_db::pool::{DbPool, init_pool, run_migrations};
use panel_db::repositories::SqliteUserRepository;
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

async fn setup_test_app() -> (axum::Router, DbPool, Arc<JobRunner>, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.path().join("job_test.db");
    let db_url = format!(
        "sqlite://{}",
        db_path.to_str().expect("to_str").replace('\\', "/")
    );

    let pool = init_pool(&db_url).await.expect("init pool");
    run_migrations(&pool).await.expect("run migrations");

    let user_repo = SqliteUserRepository::new(pool.clone());

    // 1. Create admin user
    let admin_hash = hash_password("AdminPass123!").expect("hash admin");
    user_repo
        .create(&CreateUser {
            username: "admin".to_string(),
            password_hash: admin_hash,
            role: "admin".to_string(),
            email: Some("admin@example.com".to_string()),
            linux_username: None,
            linux_uid: None,
            plan_id: None,
        })
        .await
        .expect("create admin");

    // 2. Create regular user: alice
    let alice_hash = hash_password("AlicePass123!").expect("hash alice");
    user_repo
        .create(&CreateUser {
            username: "alice".to_string(),
            password_hash: alice_hash,
            role: "user".to_string(),
            email: Some("alice@example.com".to_string()),
            linux_username: None,
            linux_uid: None,
            plan_id: None,
        })
        .await
        .expect("create alice");

    // 3. Create regular user: bob
    let bob_hash = hash_password("BobPass123!").expect("hash bob");
    user_repo
        .create(&CreateUser {
            username: "bob".to_string(),
            password_hash: bob_hash,
            role: "user".to_string(),
            email: Some("bob@example.com".to_string()),
            linux_username: None,
            linux_uid: None,
            plan_id: None,
        })
        .await
        .expect("create bob");

    let sock_path = tmp.path().join("agent.sock");
    let auth_config = AuthConfig {
        session_duration_secs: 3600,
        max_failed_attempts: 5,
        lockout_duration_secs: 300,
        cookie_secure: Some(false),
    };

    let lockout_manager = Arc::new(LockoutManager::new(5, 300));
    let agent_client = Arc::new(AgentClient::new(sock_path));
    let operations = panel_db::operations::open(&pool).await.expect("operations");
    let state = AppState::with_operations(
        agent_client,
        pool.clone(),
        operations,
        "development".to_string(),
        auth_config,
        lockout_manager,
    );

    let runner = Arc::clone(&state.job_runner);
    (create_router(state), pool, runner, tmp)
}

fn extract_cookie_value(cookie_header: &str, cookie_name: &str) -> Option<String> {
    for pair in cookie_header.split(';') {
        let (key, val) = pair.trim().split_once('=')?;
        if key == cookie_name {
            return Some(val.to_string());
        }
    }
    None
}

#[tokio::test]
async fn running_package_job_cannot_be_cancelled_and_resubmitted() {
    let (app, pool, runner, _tmp) = setup_test_app().await;
    let admin = SqliteUserRepository::new(pool)
        .find_by_username("admin")
        .await
        .unwrap()
        .unwrap();
    let (job, _) = runner
        .enqueue_database_install(admin.id, "mariadb")
        .await
        .unwrap();
    assert_eq!(
        runner
            .job_repo
            .claim_next_queued()
            .await
            .unwrap()
            .unwrap()
            .id,
        job.id
    );
    let (cookie, csrf) = login(&app, "admin", "AdminPass123!").await;
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/jobs/{}/cancel", job.id))
                .header(header::COOKIE, format!("{SESSION_COOKIE_NAME}={cookie}"))
                .header("X-CSRF-Token", csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let (joined, duplicate) = runner
        .enqueue_database_install(admin.id, "mariadb")
        .await
        .unwrap();
    assert!(duplicate);
    assert_eq!(joined.id, job.id);
    assert_eq!(joined.status, "running");
}

#[tokio::test]
async fn durable_logs_respect_owner_and_resume_from_sequence() {
    let (app, pool, runner, _tmp) = setup_test_app().await;
    let alice = SqliteUserRepository::new(pool)
        .find_by_username("alice")
        .await
        .unwrap()
        .unwrap();
    let (job, _) = runner
        .enqueue(
            Some(alice.id),
            None,
            "system.echo".into(),
            "{}".into(),
            Some(1),
        )
        .await
        .unwrap();
    assert!(process_one_job(&runner).await.unwrap());
    let (alice_cookie, _) = login(&app, "alice", "AlicePass123!").await;
    let (bob_cookie, _) = login(&app, "bob", "BobPass123!").await;
    let (admin_cookie, _) = login(&app, "admin", "AdminPass123!").await;
    let get = |cookie: &str, path: String| {
        Request::builder()
            .uri(path)
            .header(header::COOKIE, format!("{SESSION_COOKIE_NAME}={cookie}"))
            .body(Body::empty())
            .unwrap()
    };
    let response = app
        .clone()
        .oneshot(get(&alice_cookie, format!("/api/v1/jobs/{}/logs", job.id)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let events: Vec<panel_api::jobs::JobProgressEvent> =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(events.len() >= 3);
    assert_eq!(events.last().unwrap().status, "succeeded");
    let sequence = events.last().unwrap().sequence.unwrap();
    let response = app
        .clone()
        .oneshot(get(
            &alice_cookie,
            format!("/api/v1/jobs/{}/logs?after={sequence}", job.id),
        ))
        .await
        .unwrap();
    let events: Vec<panel_api::jobs::JobProgressEvent> =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(events.is_empty());
    let forbidden = app
        .clone()
        .oneshot(get(&bob_cookie, format!("/api/v1/jobs/{}/logs", job.id)))
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
    let allowed = app
        .clone()
        .oneshot(get(&admin_cookie, format!("/api/v1/jobs/{}/logs", job.id)))
        .await
        .unwrap();
    assert_eq!(allowed.status(), StatusCode::OK);
    let sse = app
        .oneshot(get(
            &alice_cookie,
            format!("/api/v1/jobs/{}/events", job.id),
        ))
        .await
        .unwrap();
    assert_eq!(sse.headers().get("x-accel-buffering").unwrap(), "no");
}

#[tokio::test]
async fn deleted_actor_cannot_execute_a_previously_queued_job() {
    let (_app, core, runner, _tmp) = setup_test_app().await;
    let alice = SqliteUserRepository::new(core.clone())
        .find_by_username("alice")
        .await
        .unwrap()
        .unwrap();
    let (job, _) = runner
        .enqueue(
            Some(alice.id),
            None,
            "system.echo".to_owned(),
            "{}".to_owned(),
            Some(1),
        )
        .await
        .unwrap();
    sqlx::query("DELETE FROM users WHERE id = ?")
        .bind(alice.id)
        .execute(&core)
        .await
        .unwrap();
    assert!(process_one_job(&runner).await.unwrap());
    let finished = runner.job_repo.find_by_id(&job.id).await.unwrap().unwrap();
    assert_eq!(finished.status, "failed");
    assert!(finished.error_message.unwrap().contains("tidak aktif"));
}

async fn login(app: &axum::Router, username: &str, password: &str) -> (String, String) {
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "username": username,
                "password": password
            }))
            .expect("json"),
        ))
        .expect("build");

    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);

    let session_cookie = res
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|c| c.to_str().ok())
        .and_then(|h| extract_cookie_value(h, SESSION_COOKIE_NAME))
        .expect("session cookie");

    let body_bytes = res.into_body().collect().await.expect("body").to_bytes();
    let body: Value = serde_json::from_slice(&body_bytes).expect("parse json");
    let csrf_token = body["csrf_token"].as_str().expect("csrf").to_string();

    (session_cookie, csrf_token)
}

#[tokio::test]
async fn test_job_enqueue_and_idempotency() {
    let (app, _pool, _runner, _tmp) = setup_test_app().await;
    let (alice_cookie, alice_csrf) = login(&app, "alice", "AlicePass123!").await;

    // 1. Enqueue job without explicit ID
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/jobs")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, alice_cookie),
        )
        .header("X-CSRF-Token", &alice_csrf)
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "name": "system.echo",
                "payload": { "msg": "hello auto id" }
            }))
            .expect("json"),
        ))
        .expect("req");

    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::CREATED);
    let body_bytes = res.into_body().collect().await.expect("body").to_bytes();
    let job: JobItem = serde_json::from_slice(&body_bytes).expect("parse job");
    assert_eq!(job.name, "system.echo");
    assert_eq!(job.status, "queued");
    assert_eq!(job.progress, 0);

    // 2. Enqueue job with idempotency ID
    let custom_id = "test-idempotency-key-001";
    let req2 = Request::builder()
        .method("POST")
        .uri("/api/v1/jobs")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, alice_cookie),
        )
        .header("X-CSRF-Token", &alice_csrf)
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "id": custom_id,
                "name": "system.echo",
                "payload": { "attempt": 1 }
            }))
            .expect("json"),
        ))
        .expect("req");

    let res2 = app.clone().oneshot(req2).await.expect("res");
    assert_eq!(res2.status(), StatusCode::CREATED);
    let job2: JobItem =
        serde_json::from_slice(&res2.into_body().collect().await.expect("b").to_bytes())
            .expect("parse job");
    assert_eq!(job2.id, custom_id);

    // 3. Retry same enqueue with identical ID -> should return 200 OK (idempotent duplicate)
    let req3 = Request::builder()
        .method("POST")
        .uri("/api/v1/jobs")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, alice_cookie),
        )
        .header("X-CSRF-Token", &alice_csrf)
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "id": custom_id,
                "name": "system.echo",
                "payload": { "attempt": 2 }
            }))
            .expect("json"),
        ))
        .expect("req");

    let res3 = app.clone().oneshot(req3).await.expect("res");
    assert_eq!(res3.status(), StatusCode::OK);
    let job3: JobItem =
        serde_json::from_slice(&res3.into_body().collect().await.expect("b").to_bytes())
            .expect("parse job");
    assert_eq!(job3.id, custom_id);
    assert_eq!(job3.status, "queued");
}

#[tokio::test]
async fn test_job_worker_execution_success() {
    let (_app, _pool, runner, _tmp) = setup_test_app().await;

    // Enqueue an echo job
    let (job, _) = runner
        .enqueue(
            Some(1),
            None,
            "system.echo".to_string(),
            r#"{"output":"test payload"}"#.to_string(),
            Some(3),
        )
        .await
        .expect("enqueue");

    assert_eq!(job.status, "queued");

    // Process the job
    let processed = process_one_job(&runner).await.expect("process");
    assert!(processed);

    // Verify finished job in DB
    let finished = runner
        .job_repo
        .find_by_id(&job.id)
        .await
        .expect("find")
        .expect("exists");

    assert_eq!(finished.status, "succeeded");
    assert_eq!(finished.progress, 100);
    assert_eq!(
        finished.result_json.as_deref(),
        Some(r#"{"output":"test payload"}"#)
    );
    assert!(finished.error_message.is_none());
}

#[tokio::test]
async fn test_job_retry_backoff_and_eventual_failure() {
    let (_app, _pool, runner, _tmp) = setup_test_app().await;

    // Enqueue a job with test.fail and max_retries = 2
    let (job, _) = runner
        .enqueue(
            Some(1),
            None,
            "test.fail".to_string(),
            r#"{"error":"simulated database timeout"}"#.to_string(),
            Some(2),
        )
        .await
        .expect("enqueue");

    // Attempt 1 (retry_count: 0 -> 1, max_retries: 2) => Requeued to "queued"
    let processed = process_one_job(&runner).await.expect("process 1");
    assert!(processed);

    let state1 = runner
        .job_repo
        .find_by_id(&job.id)
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(state1.status, "queued");
    assert_eq!(state1.retry_count, 1);
    assert_eq!(
        state1.error_message.as_deref(),
        Some("simulated database timeout")
    );

    // Attempt 2 (retry_count: 1 -> 2 >= max_retries 2) => Transitions to "failed"
    let processed2 = process_one_job(&runner).await.expect("process 2");
    assert!(processed2);

    let state2 = runner
        .job_repo
        .find_by_id(&job.id)
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(state2.status, "failed");
    assert_eq!(state2.retry_count, 2);
    assert_eq!(
        state2.error_message.as_deref(),
        Some("simulated database timeout")
    );
}

#[tokio::test]
async fn test_job_cancellation_queued_and_running() {
    let (app, _pool, runner, _tmp) = setup_test_app().await;
    let (alice_cookie, alice_csrf) = login(&app, "alice", "AlicePass123!").await;

    // 1. Cancel while in queued state
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/jobs")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, alice_cookie),
        )
        .header("X-CSRF-Token", &alice_csrf)
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "name": "system.echo",
                "payload": { "task": "cancel me" }
            }))
            .expect("json"),
        ))
        .expect("req");

    let res = app.clone().oneshot(req).await.expect("res");
    let job: JobItem =
        serde_json::from_slice(&res.into_body().collect().await.expect("b").to_bytes())
            .expect("parse job");

    // Cancel the queued job via API
    let cancel_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/jobs/{}/cancel", job.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, alice_cookie),
        )
        .header("X-CSRF-Token", &alice_csrf)
        .body(Body::empty())
        .expect("req");

    let cancel_res = app.clone().oneshot(cancel_req).await.expect("res");
    assert_eq!(cancel_res.status(), StatusCode::OK);
    let cancelled_job: JobItem = serde_json::from_slice(
        &cancel_res
            .into_body()
            .collect()
            .await
            .expect("b")
            .to_bytes(),
    )
    .expect("parse");
    assert_eq!(cancelled_job.status, "cancelled");

    // Processing queued jobs should not claim this cancelled job
    let claim_res = runner.job_repo.claim_next_queued().await.expect("claim");
    assert!(claim_res.is_none());

    // 2. Cancel while running (cancellation token check)
    let (sleep_job, _) = runner
        .enqueue(
            Some(2),
            None,
            "test.sleep".to_string(),
            r#"{"duration_ms": 300, "steps": 6}"#.to_string(),
            Some(1),
        )
        .await
        .expect("enqueue sleep");

    // Spawn a worker to process it
    let r = Arc::clone(&runner);
    let worker_task = tokio::spawn(async move {
        process_one_job(&r).await.expect("process sleep");
    });

    // Wait briefly for job to start running
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Trigger cancellation
    let cancelled = runner.cancel(&sleep_job.id).await.expect("cancel");
    assert!(cancelled.is_some());

    worker_task.await.expect("worker task finish");

    let final_job = runner
        .job_repo
        .find_by_id(&sleep_job.id)
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(final_job.status, "cancelled");
}

#[tokio::test]
async fn test_startup_recovery_interrupted_jobs() {
    let (_app, _pool, runner, _tmp) = setup_test_app().await;
    let job_repo = runner.job_repo.clone();

    // Manually insert an interrupted running job
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time")
        .as_secs() as i64;

    job_repo
        .create(&CreateJob {
            id: "interrupted-001".to_string(),
            user_id: Some(1),
            name: "test.sleep".to_string(),
            payload_json: "{}".to_string(),
            max_retries: 3,
        })
        .await
        .expect("create");

    // Force its status to 'running'
    job_repo
        .update_status("interrupted-001", "running", None, None)
        .await
        .expect("update status to running");

    let before = job_repo
        .find_by_id("interrupted-001")
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(before.status, "running");

    // Simulate server startup recovery
    let recovered_count = runner.recover_interrupted().await.expect("recover");
    assert_eq!(recovered_count, 1);

    let after = job_repo
        .find_by_id("interrupted-001")
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(after.status, "failed");
    assert_eq!(
        after.error_message.as_deref(),
        Some("Interrupted by server restart")
    );
    assert!(after.updated_at >= now);
}

#[tokio::test]
async fn test_job_rbac_isolation() {
    let (app, _pool, runner, _tmp) = setup_test_app().await;
    let (alice_cookie, alice_csrf) = login(&app, "alice", "AlicePass123!").await;
    let (bob_cookie, bob_csrf) = login(&app, "bob", "BobPass123!").await;
    let (admin_cookie, admin_csrf) = login(&app, "admin", "AdminPass123!").await;

    // Alice creates a job
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/jobs")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, alice_cookie),
        )
        .header("X-CSRF-Token", &alice_csrf)
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "name": "system.echo",
                "payload": { "owner": "alice" }
            }))
            .expect("json"),
        ))
        .expect("req");

    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::CREATED);
    let alice_job: JobItem =
        serde_json::from_slice(&res.into_body().collect().await.expect("b").to_bytes())
            .expect("parse job");

    // 1. Bob tries to inspect Alice's job -> 403 Forbidden
    let bob_get = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/jobs/{}", alice_job.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, bob_cookie),
        )
        .body(Body::empty())
        .expect("req");

    let bob_res = app.clone().oneshot(bob_get).await.expect("res");
    assert_eq!(bob_res.status(), StatusCode::FORBIDDEN);

    // 2. Bob tries to cancel Alice's job -> 403 Forbidden
    let bob_cancel = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/jobs/{}/cancel", alice_job.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, bob_cookie),
        )
        .header("X-CSRF-Token", &bob_csrf)
        .body(Body::empty())
        .expect("req");

    let bob_cancel_res = app.clone().oneshot(bob_cancel).await.expect("res");
    assert_eq!(bob_cancel_res.status(), StatusCode::FORBIDDEN);

    // 3. Bob tries to stream Alice's job events -> 403 Forbidden
    let bob_stream = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/jobs/{}/events", alice_job.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, bob_cookie),
        )
        .body(Body::empty())
        .expect("req");

    let bob_stream_res = app.clone().oneshot(bob_stream).await.expect("res");
    assert_eq!(bob_stream_res.status(), StatusCode::FORBIDDEN);

    // 4. Bob lists jobs -> Alice's job is NOT included
    let bob_list = Request::builder()
        .method("GET")
        .uri("/api/v1/jobs")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, bob_cookie),
        )
        .body(Body::empty())
        .expect("req");

    let bob_list_res = app.clone().oneshot(bob_list).await.expect("res");
    assert_eq!(bob_list_res.status(), StatusCode::OK);
    let bob_list_body: JobListResponse = serde_json::from_slice(
        &bob_list_res
            .into_body()
            .collect()
            .await
            .expect("b")
            .to_bytes(),
    )
    .expect("parse list");
    assert_eq!(bob_list_body.total, 0);

    // 5. Alice lists jobs -> sees her own job
    let alice_list = Request::builder()
        .method("GET")
        .uri("/api/v1/jobs")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, alice_cookie),
        )
        .body(Body::empty())
        .expect("req");

    let alice_list_res = app.clone().oneshot(alice_list).await.expect("res");
    assert_eq!(alice_list_res.status(), StatusCode::OK);
    let alice_list_body: JobListResponse = serde_json::from_slice(
        &alice_list_res
            .into_body()
            .collect()
            .await
            .expect("b")
            .to_bytes(),
    )
    .expect("parse list");
    assert_eq!(alice_list_body.total, 1);
    assert_eq!(alice_list_body.items[0].id, alice_job.id);

    // 6. Admin can inspect Alice's job -> 200 OK
    let admin_get = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/jobs/{}", alice_job.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, admin_cookie),
        )
        .body(Body::empty())
        .expect("req");

    let admin_res = app.clone().oneshot(admin_get).await.expect("res");
    assert_eq!(admin_res.status(), StatusCode::OK);

    // 7. Admin can cancel Alice's job -> 200 OK
    let admin_cancel = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/jobs/{}/cancel", alice_job.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, admin_cookie),
        )
        .header("X-CSRF-Token", &admin_csrf)
        .body(Body::empty())
        .expect("req");

    let admin_cancel_res = app.clone().oneshot(admin_cancel).await.expect("res");
    assert_eq!(admin_cancel_res.status(), StatusCode::OK);

    let updated_job = runner
        .job_repo
        .find_by_id(&alice_job.id)
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(updated_job.status, "cancelled");
}

#[tokio::test]
async fn test_job_sse_events_stream() {
    let (app, _pool, runner, _tmp) = setup_test_app().await;
    let (alice_cookie, _alice_csrf) = login(&app, "alice", "AlicePass123!").await;

    // Enqueue and complete a job
    let (job, _) = runner
        .enqueue(
            Some(2),
            None,
            "system.echo".to_string(),
            r#"{"status":"done"}"#.to_string(),
            Some(1),
        )
        .await
        .expect("enqueue");

    process_one_job(&runner).await.expect("process");

    // Connect to SSE stream for this already-succeeded job
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/jobs/{}/events", job.id))
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, alice_cookie),
        )
        .body(Body::empty())
        .expect("req");

    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::OK);
    let content_type = res
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok());
    assert_eq!(content_type, Some("text/event-stream"));

    let body_bytes = res.into_body().collect().await.expect("body").to_bytes();
    let sse_text = String::from_utf8_lossy(&body_bytes);
    assert!(sse_text.contains("event: progress"));
    assert!(sse_text.contains(&job.id));
    assert!(sse_text.contains("succeeded"));
}

#[tokio::test]
async fn test_job_csrf_enforcement() {
    let (app, _pool, _runner, _tmp) = setup_test_app().await;
    let (alice_cookie, _alice_csrf) = login(&app, "alice", "AlicePass123!").await;

    // POST /api/v1/jobs without X-CSRF-Token header -> 403 Forbidden
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/jobs")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!("{}={}", SESSION_COOKIE_NAME, alice_cookie),
        )
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "name": "system.echo",
                "payload": {}
            }))
            .expect("json"),
        ))
        .expect("req");

    let res = app.clone().oneshot(req).await.expect("res");
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

/// No AgentServer and no worker are started: enqueue must succeed without RPC.
#[tokio::test]
async fn node_install_returns_persisted_job_without_a_running_agent() {
    let (app, core, runner, tmp) = setup_test_app().await;
    let (cookie, csrf) = login(&app, "admin", "AdminPass123!").await;
    let request = || {
        Request::builder()
            .method("POST")
            .uri("/api/v1/runtimes/node/install")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::COOKIE, format!("{SESSION_COOKIE_NAME}={cookie}"))
            .header("X-CSRF-Token", csrf.as_str())
            .body(Body::from(
                r#"{"version":"v22.14.0","enable_corepack":true}"#,
            ))
            .expect("request")
    };

    // An absent catalog must still fail closed, before any job or agent RPC.
    let rejected = app.clone().oneshot(request()).await.expect("response");
    let rejected_status = rejected.status();
    let rejected_body = rejected.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        rejected_status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        String::from_utf8_lossy(&rejected_body)
    );
    assert_eq!(runner.job_repo.count_filtered(None, None).await.unwrap(), 0);

    // Synthetic metadata tests enqueueing, not today's official release policy.
    // Relative dates keep this fixture valid without a network request or agent.
    let now = chrono::Utc::now();
    let today = now.date_naive();
    let catalog: panel_api::runtime_catalog::NodeCatalog = serde_json::from_value(serde_json::json!({
        "releases": [{"version":"v22.14.0","date":(today - chrono::Duration::days(1)).to_string(),"lts":"Fixture","files":[]}],
        "schedule": {"v22": {
            "start": (today - chrono::Duration::days(365)).to_string(),
            "lts": (today - chrono::Duration::days(180)).to_string(),
            "end": (today + chrono::Duration::days(365)).to_string()
        }}
    })).expect("node catalog fixture");
    panel_api::runtime_catalog::store_refresh(
        &core,
        "runtime.catalog.node",
        Ok(catalog),
        now.timestamp(),
    )
    .await
    .expect("seed node catalog");

    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        app.clone().oneshot(request()),
    )
    .await
    .expect("enqueue must not wait for installation")
    .expect("response");
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let job: JobItem = serde_json::from_slice(&body).unwrap();
    assert_eq!(job.name, "node.install");
    assert_eq!(job.status, "queued");
    assert!(job.result_json.is_none());
    assert!(!tmp.path().join("agent.sock").exists());

    // A separate database connection sees the job after the request has ended.
    let (_writer, reader) = panel_db::operations::open(&core).await.unwrap();
    let saved = panel_db::repositories::SqliteJobRepository::new(reader)
        .find_by_id(&job.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.status, "queued");
    assert_eq!(saved.user_id, job.user_id);
    let payload: Value = serde_json::from_str(&saved.payload_json).unwrap();
    assert_eq!(payload["version"], "22.14.0");
    assert_eq!(payload["enable_corepack"], true);
    assert_eq!(runner.job_repo.count_filtered(None, None).await.unwrap(), 1);
}

#[tokio::test]
async fn node_install_rejects_invalid_or_unauthorized_enqueue_without_agent() {
    let (app, _core, runner, _tmp) = setup_test_app().await;
    let (admin_cookie, admin_csrf) = login(&app, "admin", "AdminPass123!").await;
    let (user_cookie, user_csrf) = login(&app, "alice", "AlicePass123!").await;
    for (cookie, csrf, version, expected) in [
        (None, None, "22.14.0", StatusCode::UNAUTHORIZED),
        (
            Some(user_cookie.as_str()),
            Some(user_csrf.as_str()),
            "22.14.0",
            StatusCode::FORBIDDEN,
        ),
        (
            Some(admin_cookie.as_str()),
            None,
            "22.14.0",
            StatusCode::FORBIDDEN,
        ),
        (
            Some(admin_cookie.as_str()),
            Some(admin_csrf.as_str()),
            "../../etc",
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let mut request = Request::builder()
            .method("POST")
            .uri("/api/v1/runtimes/node/install")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(cookie) = cookie {
            request = request.header(header::COOKIE, format!("{SESSION_COOKIE_NAME}={cookie}"));
        }
        if let Some(csrf) = csrf {
            request = request.header("X-CSRF-Token", csrf);
        }
        let request = request
            .body(Body::from(
                serde_json::json!({"version":version}).to_string(),
            ))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), expected);
    }
    assert_eq!(runner.job_repo.count_filtered(None, None).await.unwrap(), 0);
}
