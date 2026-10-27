#![allow(clippy::unwrap_used, clippy::expect_used)]

use panel_agent::system::MockSystemRunner;
use panel_agent::{AgentConfig, AgentServer};
use panel_api::agent_client::AgentClient;
use panel_proto::HardeningApplyRequest;
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;

#[tokio::test]
async fn test_hardening_prerequisites_and_apply_over_ipc() {
    let temp_ipc = tempdir().expect("tempdir ipc");
    let temp_sys = tempdir().expect("tempdir sys");

    #[cfg(unix)]
    let sock_path = temp_ipc.path().join("agent.sock");
    #[cfg(not(unix))]
    let sock_path = temp_ipc.path().join("agent.port");

    let cgroup_dir = temp_sys.path().join("cgroup");
    fs::create_dir_all(&cgroup_dir).expect("create cgroup dir");
    fs::write(
        cgroup_dir.join("cgroup.controllers"),
        "cpu memory io pids\n",
    )
    .expect("write cgroup controllers");

    let mounts_path = temp_sys.path().join("mounts");
    fs::write(
        &mounts_path,
        "proc /proc proc rw,nosuid,nodev,noexec,relatime 0 0\n",
    )
    .expect("write mounts without hidepid");

    let group_path = temp_sys.path().join("group");
    fs::write(&group_path, "root:x:0:\n").expect("write group without panel-tenants");

    let runner = Arc::new(MockSystemRunner::new());
    runner.set_response(
        "findmnt --task 1 -n -o OPTIONS --target /proc",
        0,
        b"rw,nosuid,nodev,noexec,hidepid=2,gid=100\n",
        b"",
    );
    let user_manager = Arc::new(panel_agent::user::UserLifecycleManager::new(
        Arc::clone(&runner) as Arc<dyn panel_agent::system::SystemRunner>,
    ));
    let slice_manager = Arc::new(panel_agent::slice::SliceManager::new(
        Arc::clone(&runner) as Arc<dyn panel_agent::system::SystemRunner>
    ));
    let quota_manager = Arc::new(panel_agent::quota::QuotaManager::new(
        Arc::clone(&runner) as Arc<dyn panel_agent::system::SystemRunner>
    ));
    let resource_reader = Arc::new(panel_agent::resource::CgroupReader::new(&cgroup_dir));
    let hardening_manager = Arc::new(panel_agent::hardening::HardeningManager::with_paths(
        Arc::clone(&runner) as Arc<dyn panel_agent::system::SystemRunner>,
        &cgroup_dir,
        &mounts_path,
        &group_path,
    ));

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

    // 1. Initial check: cgroup_v2 ok, acl ok, quota ok, hidepid not ok, group not ok -> overall_ok = false
    let status = client.check_hardening().await.expect("check_hardening");
    assert!(status.cgroup_v2.ok);
    assert!(status.acl.ok);
    assert!(status.quota.ok);
    assert!(!status.hidepid.ok);
    assert!(!status.tenant_group.ok);
    assert!(!status.overall_ok);
    assert!(status.hidepid.message.contains("hidepid=2 belum aktif"));
    assert!(status.tenant_group.message.contains("panel-tenants"));

    // 2. Apply hardening: ensure tenant group and enable hidepid
    let apply_res = client
        .apply_hardening(HardeningApplyRequest {
            enable_hidepid: true,
            ensure_tenant_group: true,
        })
        .await
        .expect("apply_hardening");

    assert!(apply_res.hidepid_applied);
    assert!(apply_res.tenant_group_created);
    assert!(apply_res.message.contains("panel-tenants"));

    // Verify mock commands were recorded
    let actions = runner.recorded_actions();
    assert!(actions.iter().any(|a| matches!(a, panel_agent::system::MockAction::Command { binary, .. } if binary == "groupadd")));
    let commands = runner.get_commands();
    assert!(commands.contains(&"systemctl enable panel-proc-hardening.service".to_owned()));
    assert!(commands.contains(&"systemctl restart panel-proc-hardening.service".to_owned()));
    assert!(!commands.iter().any(|command| command.starts_with("mount ")));

    // 3. Update simulated system state to hardened and re-check
    fs::write(
        &mounts_path,
        "proc /proc proc rw,nosuid,nodev,noexec,relatime,hidepid=2 0 0\n",
    )
    .expect("write hardened mounts");
    fs::write(&group_path, "root:x:0:\npanel-tenants:x:1005:\n")
        .expect("write group with panel-tenants");

    let post_status = client
        .check_hardening()
        .await
        .expect("check_hardening post");
    assert!(post_status.hidepid.ok);
    assert!(post_status.tenant_group.ok);
    assert!(post_status.overall_ok);
    assert!(post_status.hidepid.message.contains("terisolasi"));

    // Clean shutdown
    let _ = shutdown_tx.send(());
    let _ = server_task.await;
}
