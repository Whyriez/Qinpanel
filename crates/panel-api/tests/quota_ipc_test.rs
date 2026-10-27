#![allow(clippy::unwrap_used, clippy::expect_used)]

use panel_agent::quota::QuotaManager;
use panel_agent::slice::SliceManager;
use panel_agent::system::{MockSystemRunner, SystemRunner};
use panel_agent::user::UserLifecycleManager;
use panel_agent::{AgentConfig, AgentServer};
use panel_api::agent_client::AgentClient;
use panel_api::jobs::registry::JobRegistry;
use panel_api::jobs::runner::JobRunner;
use panel_api::jobs::worker::process_one_job;
use panel_db::models::CreateUser;
use panel_db::repositories::SqliteUserRepository;
use panel_db::{DbPool, init_pool, run_migrations};
use panel_proto::{QuotaGetRequest, QuotaScanRequest, QuotaSetRequest};
use std::sync::Arc;
use std::time::Duration;

async fn setup_test_db() -> DbPool {
    let pool = init_pool("sqlite::memory:").await.expect("in-memory db");
    run_migrations(&pool).await.expect("run migrations");
    pool
}

#[tokio::test]
async fn test_quota_management_and_job_over_ipc() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let sock_path = tmp.path().join("agent.sock");
    let home_dir = tmp.path().join("home");
    std::fs::create_dir_all(&home_dir).expect("create home");

    // Populate user directory
    let tenant_home = home_dir.join("p_quota_user");
    std::fs::create_dir_all(&tenant_home).expect("create tenant home");
    std::fs::write(tenant_home.join("app.js"), "console.log('hello world');")
        .expect("write app.js"); // 27 bytes
    std::fs::write(tenant_home.join("data.txt"), "panel quota test data").expect("write data.txt"); // 21 bytes

    let mock_runner = MockSystemRunner::new();
    let runner_arc: Arc<dyn SystemRunner> = Arc::new(mock_runner);

    let user_manager = Arc::new(UserLifecycleManager::new(Arc::clone(&runner_arc)));
    let slice_manager = Arc::new(SliceManager::new(Arc::clone(&runner_arc)));
    let quota_manager = Arc::new(QuotaManager::with_home_base(
        Arc::clone(&runner_arc),
        &home_dir,
    ));

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

    tokio::time::sleep(Duration::from_millis(100)).await;

    let client = Arc::new(AgentClient::new(sock_path));

    // 1. Set Quota via IPC
    let set_res = client
        .set_quota(QuotaSetRequest {
            uid: 10001,
            username: "p_quota_user".to_string(),
            disk_limit_bytes: 100 * 1024 * 1024,
            inode_limit: 50_000,
            mount_point: None,
        })
        .await
        .expect("set quota over IPC");

    assert_eq!(set_res.uid, 10001);
    assert_eq!(set_res.username, "p_quota_user");
    assert!(set_res.applied);

    // 2. Scan Quota via IPC
    let scan_res = client
        .scan_quota(QuotaScanRequest {
            uid: 10001,
            username: "p_quota_user".to_string(),
            path: None,
        })
        .await
        .expect("scan quota over IPC");

    assert_eq!(scan_res.uid, 10001);
    assert_eq!(scan_res.bytes_used, 27 + 21);
    // Inodes: tenant_home (1) + app.js (1) + data.txt (1) = 3
    assert_eq!(scan_res.inodes_used, 3);

    // 3. Get Quota via IPC
    let get_res = client
        .get_quota(QuotaGetRequest {
            uid: 10001,
            username: "p_quota_user".to_string(),
            mount_point: None,
        })
        .await
        .expect("get quota over IPC");

    assert_eq!(get_res.uid, 10001);
    assert_eq!(get_res.bytes_used, 48);
    assert_eq!(get_res.bytes_limit, 100 * 1024 * 1024);
    assert_eq!(get_res.inodes_used, 3);
    assert_eq!(get_res.inodes_limit, 50_000);

    // 4. Reject Path Traversal
    let reject_res = client
        .scan_quota(QuotaScanRequest {
            uid: 10001,
            username: "p_quota_user".to_string(),
            path: Some("/etc".to_string()),
        })
        .await;
    assert!(
        reject_res.is_err(),
        "Scanning outside home must be rejected"
    );

    // 5. Test Background Quota Job Execution
    let pool = setup_test_db().await;
    // The worker resolves execution identity from the database, not the home directory.
    let tenant = SqliteUserRepository::new(pool.clone())
        .create(&CreateUser {
            username: "quota_user".to_string(),
            password_hash: "unused-test-password-hash".to_string(),
            role: "user".to_string(),
            email: None,
            linux_username: Some("p_quota_user".to_string()),
            linux_uid: Some(10001),
            plan_id: None,
        })
        .await
        .expect("create quota tenant fixture");
    let registry = JobRegistry::new_with_agent(Arc::clone(&client));
    let runner = Arc::new(JobRunner::with_registry(pool.clone(), registry));

    let (job, _) = runner
        .enqueue(
            Some(tenant.id),
            None,
            "quota.report".to_string(),
            serde_json::json!({
                "uid": 10001,
                "username": "p_quota_user"
            })
            .to_string(),
            Some(1),
        )
        .await
        .expect("enqueue quota report job");

    // Run worker to execute job
    let executed = process_one_job(&runner).await.expect("process one job");
    assert!(executed, "Job must be claimed and executed");

    let finished_job = runner
        .job_repo
        .find_by_id(&job.id)
        .await
        .expect("find job")
        .expect("job exists");

    assert_eq!(
        finished_job.status, "succeeded",
        "Quota job failed: {:?}",
        finished_job.error_message
    );
    assert_eq!(finished_job.user_id, Some(tenant.id));
    assert!(finished_job.error_message.is_none());
    assert_eq!(finished_job.progress, 100);
    let result_val: serde_json::Value =
        serde_json::from_str(&finished_job.result_json.unwrap()).expect("parse result");
    assert_eq!(result_val["uid"], 10001);
    assert_eq!(result_val["username"], "p_quota_user");
    assert_eq!(result_val["bytes_used"], 48);
    assert_eq!(result_val["inodes_used"], 3);

    let _ = shutdown_tx.send(());
    let _ = server_task.await;
}

#[tokio::test]
async fn test_quota_jobs_reject_unregistered_tenant_before_ipc() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let pool = setup_test_db().await;
    // No agent is started: an unknown tenant must be rejected before connecting.
    let client = Arc::new(AgentClient::new(tmp.path().join("no-agent.sock")));
    let registry = JobRegistry::new_with_agent(client);
    let runner = Arc::new(JobRunner::with_registry(pool, registry));

    for name in ["quota.report", "quota.scan"] {
        let (job, _) = runner
            .enqueue(
                None,
                None,
                name.to_string(),
                serde_json::json!({"uid": 10001, "username": "p_quota_user"}).to_string(),
                Some(1),
            )
            .await
            .expect("enqueue quota job for unregistered tenant");
        assert!(process_one_job(&runner).await.expect("process quota job"));
        let finished_job = runner
            .job_repo
            .find_by_id(&job.id)
            .await
            .expect("find quota job")
            .expect("quota job exists");
        assert_eq!(finished_job.status, "failed");
        assert_eq!(
            finished_job.error_message.as_deref(),
            Some("Identitas tenant pekerjaan tidak ditemukan")
        );
        assert_eq!(finished_job.progress, 0);
        assert!(finished_job.result_json.is_none());
    }
}
