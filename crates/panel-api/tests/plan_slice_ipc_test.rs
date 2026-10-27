#![allow(clippy::unwrap_used, clippy::expect_used)]

use panel_agent::slice::SliceManager;
use panel_agent::system::{MockAction, MockSystemRunner, SystemRunner};
use panel_agent::user::UserLifecycleManager;
use panel_agent::{AgentConfig, AgentServer};
use panel_api::agent_client::{AgentClient, plan_to_slice_apply_request};
use panel_db::models::CreatePlan;
use panel_db::repositories::SqlitePlanRepository;
use panel_db::{DbPool, init_pool, run_migrations};
use std::sync::Arc;
use std::time::Duration;

async fn setup_test_db() -> DbPool {
    let pool = init_pool("sqlite::memory:").await.expect("in-memory db");
    run_migrations(&pool).await.expect("run migrations");
    pool
}

#[tokio::test]
async fn test_plan_crud_and_slice_limits_over_ipc() {
    // 1. Setup in-memory DB and create custom Plan
    let pool = setup_test_db().await;
    let plan_repo = SqlitePlanRepository::new(pool);

    let plan = plan_repo
        .create(&CreatePlan {
            name: "HighPerformance".to_string(),
            description: Some("Custom high performance plan for isolation testing".to_string()),
            cpu_quota_percent: Some(250),
            memory_limit_bytes: Some(2 * 1024 * 1024 * 1024), // 2 GB
            memory_high_bytes: Some(1536 * 1024 * 1024),      // 1.5 GB
            tasks_max: Some(500),
            io_weight: Some(300),
            disk_limit_bytes: Some(50 * 1024 * 1024 * 1024),
            inode_limit: Some(1_000_000),
            max_sites: Some(10),
            max_domains: Some(20),
            max_databases: Some(5),
            max_cron_jobs: Some(15),
            max_terminal_sessions: Some(2),
            max_upload_bytes: Some(100 * 1024 * 1024),
            shell_policy: Some("sftp".to_string()),
            allow_external_upstream: Some(false),
        })
        .await
        .expect("create plan in db");

    assert_eq!(plan.name, "HighPerformance");
    assert_eq!(plan.cpu_quota_percent, 250);
    assert_eq!(plan.memory_limit_bytes, 2 * 1024 * 1024 * 1024);

    // 2. Setup AgentServer with MockSystemRunner and SliceManager
    let tmp = tempfile::tempdir().expect("tempdir");
    let sock_path = tmp.path().join("agent.sock");

    let mock_runner = MockSystemRunner::new();
    let actions_ref = Arc::clone(&mock_runner.actions);
    let runner_arc: Arc<dyn SystemRunner> = Arc::new(mock_runner);

    let user_manager = Arc::new(UserLifecycleManager::new(Arc::clone(&runner_arc)));
    let slice_manager = Arc::new(SliceManager::new(Arc::clone(&runner_arc)));
    let quota_manager = Arc::new(panel_agent::quota::QuotaManager::new(Arc::clone(
        &runner_arc,
    )));

    let config = AgentConfig {
        socket_path: sock_path.clone(),
        allowed_uid: None,
    };

    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
    let resource_reader = Arc::new(panel_agent::resource::CgroupReader::new(tmp.path()));
    let hardening_manager = Arc::new(panel_agent::hardening::HardeningManager::new(Arc::clone(
        &runner_arc,
    )));
    let server = Arc::new(AgentServer::with_managers(
        config,
        user_manager,
        slice_manager,
        quota_manager,
        resource_reader,
        hardening_manager,
    ));

    let server_task = tokio::spawn(async move {
        server.run(shutdown_rx).await.expect("agent server run");
    });

    // Give server a moment to bind
    tokio::time::sleep(Duration::from_millis(100)).await;

    let client = AgentClient::new(sock_path);

    // 3. Map Plan to SliceApplyRequest for tenant UID 10001
    let slice_req = plan_to_slice_apply_request(10001, &plan);
    assert_eq!(slice_req.uid, 10001);
    assert_eq!(slice_req.cpu_quota, Some("250%".to_string()));
    assert_eq!(slice_req.memory_max, Some("2048M".to_string()));
    assert_eq!(slice_req.memory_high, Some("1536M".to_string()));
    assert_eq!(slice_req.tasks_max, Some(500));
    assert_eq!(slice_req.io_weight, Some(300));

    // 4. Apply slice via AgentClient over IPC
    let apply_res = client
        .apply_slice(slice_req)
        .await
        .expect("apply slice over IPC");

    assert_eq!(apply_res.uid, 10001);
    assert_eq!(apply_res.slice_name, "panel-u10001.slice");
    assert!(apply_res.dropin_path.contains("panel-u10001.slice.d"));
    assert!(apply_res.dropin_path.ends_with("50-limits.conf"));

    // Verify mock actions recorded on the server side
    let actions = actions_ref.lock().map(|l| l.clone()).unwrap_or_default();
    let dropin_written = actions.iter().find_map(|a| match a {
        MockAction::WriteFile {
            path,
            content,
            mode,
        } => Some((
            path.clone(),
            String::from_utf8_lossy(content).to_string(),
            *mode,
        )),
        _ => None,
    });

    assert!(dropin_written.is_some(), "50-limits.conf must be written");
    let (path, content, mode) = dropin_written.unwrap();
    assert!(path.ends_with("panel-u10001.slice.d/50-limits.conf"));
    assert_eq!(mode, 0o644);
    assert!(content.contains("[Slice]"));
    assert!(content.contains("MemoryMax=2048M"));
    assert!(content.contains("MemoryHigh=1536M"));
    assert!(content.contains("CPUQuota=250%"));
    assert!(content.contains("TasksMax=500"));
    assert!(content.contains("IOWeight=300"));

    // Verify systemctl daemon-reload was called
    assert!(actions.iter().any(|a| matches!(
        a,
        MockAction::Command { binary, args } if binary == "systemctl" && args == &["daemon-reload"]
    )));

    // 5. Remove slice via AgentClient over IPC
    let remove_res = client
        .remove_slice(10001)
        .await
        .expect("remove slice over IPC");

    assert_eq!(remove_res.uid, 10001);
    assert_eq!(remove_res.slice_name, "panel-u10001.slice");

    let actions2 = actions_ref.lock().map(|l| l.clone()).unwrap_or_default();
    assert!(actions2.iter().any(|a| matches!(
        a,
        MockAction::RemoveFile { path } if path.ends_with("panel-u10001.slice.d/50-limits.conf")
    )));

    // 6. Reject invalid slice request (non-tenant UID)
    let invalid_plan = plan_to_slice_apply_request(999, &plan);
    let invalid_res = client.apply_slice(invalid_plan).await;
    assert!(invalid_res.is_err(), "Non-tenant UID must be rejected");

    let _ = shutdown_tx.send(());
    let _ = server_task.await;
}
