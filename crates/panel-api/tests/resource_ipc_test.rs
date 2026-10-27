#![allow(clippy::unwrap_used, clippy::expect_used)]

use panel_agent::system::MockSystemRunner;
use panel_agent::{AgentConfig, AgentServer};
use panel_api::agent_client::AgentClient;
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;

#[tokio::test]
async fn test_cgroup_resource_reading_over_ipc() {
    let temp_ipc = tempdir().expect("tempdir ipc");
    let temp_cgroup = tempdir().expect("tempdir cgroup");

    #[cfg(unix)]
    let sock_path = temp_ipc.path().join("agent.sock");
    #[cfg(not(unix))]
    let sock_path = temp_ipc.path().join("agent.port");

    let runner = Arc::new(MockSystemRunner::default());
    let user_manager = Arc::new(panel_agent::user::UserLifecycleManager::new(
        Arc::clone(&runner) as Arc<dyn panel_agent::system::SystemRunner>,
    ));
    let slice_manager = Arc::new(panel_agent::slice::SliceManager::new(
        Arc::clone(&runner) as Arc<dyn panel_agent::system::SystemRunner>
    ));
    let quota_manager = Arc::new(panel_agent::quota::QuotaManager::new(
        Arc::clone(&runner) as Arc<dyn panel_agent::system::SystemRunner>
    ));
    let resource_reader = Arc::new(panel_agent::resource::CgroupReader::new(temp_cgroup.path()));
    let hardening_manager = Arc::new(panel_agent::hardening::HardeningManager::new(Arc::clone(
        &runner,
    )
        as Arc<dyn panel_agent::system::SystemRunner>));

    let config = AgentConfig {
        socket_path: sock_path.clone(),
        allowed_uid: None,
    };

    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
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

    let client = AgentClient::new(&sock_path);

    // 1. Setup mock cgroup files for UID 10001
    let slice_dir = temp_cgroup.path().join("panel-u10001.slice");
    fs::create_dir_all(&slice_dir).expect("create slice dir");

    let cpu_stat = "usage_usec 2500000\nuser_usec 2000000\nsystem_usec 500000\nnr_periods 300\nnr_throttled 10\nthrottled_usec 80000\n";
    fs::write(slice_dir.join("cpu.stat"), cpu_stat).expect("write cpu.stat");
    fs::write(slice_dir.join("memory.current"), "67108864\n").expect("write memory.current"); // 64 MB
    fs::write(slice_dir.join("memory.peak"), "134217728\n").expect("write memory.peak"); // 128 MB
    fs::write(slice_dir.join("pids.current"), "12\n").expect("write pids.current");
    fs::write(slice_dir.join("pids.peak"), "30\n").expect("write pids.peak");

    let io_stat = "8:0 rbytes=2097152 wbytes=4194304 rios=500 wios=1000 dbytes=0 dios=0\n";
    fs::write(slice_dir.join("io.stat"), io_stat).expect("write io.stat");

    // 2. Read resources over IPC
    let metrics = client
        .read_slice_resources(10001)
        .await
        .expect("read_slice_resources 10001");

    assert_eq!(metrics.uid, 10001);
    assert_eq!(metrics.slice_name, "panel-u10001.slice");
    assert!(metrics.exists);
    assert_eq!(metrics.cpu_usage_usec, 2500000);
    assert_eq!(metrics.cpu_user_usec, 2000000);
    assert_eq!(metrics.cpu_system_usec, 500000);
    assert_eq!(metrics.cpu_nr_periods, 300);
    assert_eq!(metrics.cpu_nr_throttled, 10);
    assert_eq!(metrics.cpu_throttled_usec, 80000);
    assert_eq!(metrics.memory_current_bytes, 67108864);
    assert_eq!(metrics.memory_peak_bytes, Some(134217728));
    assert_eq!(metrics.pids_current, 12);
    assert_eq!(metrics.pids_peak, Some(30));
    assert_eq!(metrics.io_read_bytes, 2097152);
    assert_eq!(metrics.io_write_bytes, 4194304);
    assert_eq!(metrics.io_read_ops, 500);
    assert_eq!(metrics.io_write_ops, 1000);

    // 3. Test non-existent slice returns zeroed metrics with exists: false
    let inactive_metrics = client
        .read_slice_resources(10002)
        .await
        .expect("read_slice_resources 10002");
    assert_eq!(inactive_metrics.uid, 10002);
    assert!(!inactive_metrics.exists);
    assert_eq!(inactive_metrics.cpu_usage_usec, 0);
    assert_eq!(inactive_metrics.memory_current_bytes, 0);
    assert_eq!(inactive_metrics.pids_current, 0);

    // 4. Test invalid UID rejected
    let err_res = client.read_slice_resources(9999).await;
    assert!(err_res.is_err(), "UID 9999 must be rejected");

    let err_high = client.read_slice_resources(60000).await;
    assert!(err_high.is_err(), "UID 60000 must be rejected");

    // Clean shutdown
    let _ = shutdown_tx.send(());
    let _ = server_task.await;
}
