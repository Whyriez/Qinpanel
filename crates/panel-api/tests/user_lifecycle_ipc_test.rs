#![allow(clippy::unwrap_used, clippy::expect_used)]

use panel_agent::user::{MockAction, MockSystemRunner, UserLifecycleManager};
use panel_agent::{AgentConfig, AgentServer};
use panel_api::agent_client::AgentClient;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn test_user_lifecycle_over_ipc() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let sock_path = tmp.path().join("agent.sock");

    let mock_runner = MockSystemRunner::new();
    let actions_ref = Arc::clone(&mock_runner.actions);
    let user_manager = Arc::new(UserLifecycleManager::new(Arc::new(mock_runner)));

    let config = AgentConfig {
        socket_path: sock_path.clone(),
        allowed_uid: None,
    };

    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
    let server = Arc::new(AgentServer::with_user_manager(config, user_manager));

    let server_task = tokio::spawn(async move {
        server.run(shutdown_rx).await.expect("agent server run");
    });

    // Give server a moment to bind
    tokio::time::sleep(Duration::from_millis(100)).await;

    let client = AgentClient::new(sock_path);

    // 1. Create User
    let create_res = client
        .create_user(
            "p_tenant1",
            10001,
            Some("/usr/sbin/nologin".to_string()),
            None,
            Some("www-data".to_string()),
        )
        .await
        .expect("create user over IPC");

    assert_eq!(create_res.username, "p_tenant1");
    assert_eq!(create_res.uid, 10001);
    assert_eq!(create_res.home_dir, "/home/p_tenant1");

    // Inspect mock actions recorded on the server side
    let actions = actions_ref.lock().map(|l| l.clone()).unwrap_or_default();
    assert!(actions.iter().any(|a| matches!(a, MockAction::Command { binary, args } if binary == "groupadd" && args.contains(&"panel-tenants".to_string()))));
    assert!(actions.iter().any(|a| matches!(a, MockAction::Command { binary, args } if binary == "groupadd" && args.contains(&"10001".to_string()) && args.contains(&"p_tenant1".to_string()))));
    assert!(actions.iter().any(|a| matches!(a, MockAction::Command { binary, args } if binary == "useradd" && args.contains(&"p_tenant1".to_string()))));
    assert!(actions.iter().any(|a| matches!(a, MockAction::CreateDir { path, mode, .. } if path == Path::new("/home/p_tenant1") && *mode == 0o700)));
    assert!(actions.iter().any(|a| matches!(a, MockAction::CreateDir { path, mode, .. } if path == Path::new("/home/p_tenant1/sites") && *mode == 0o750)));
    assert!(actions.iter().any(|a| matches!(a, MockAction::SetFacl { user, permissions, .. } if user == "www-data" && permissions == "--x")));

    // 2. Reject Invalid User (Reserved name)
    let reject_res = client.create_user("p_root", 10002, None, None, None).await;
    assert!(reject_res.is_err());

    // 3. Reject Invalid User (Out-of-range UID)
    let reject_uid = client
        .create_user("p_tenant2", 9999, None, None, None)
        .await;
    assert!(reject_uid.is_err());

    // 4. Reject Invalid User (Arbitrary non-canonical home path)
    let reject_home = client
        .create_user("p_tenant3", 10003, None, Some("/var/run".to_string()), None)
        .await;
    assert!(reject_home.is_err());

    // 5. Suspend User
    let suspend_res = client
        .suspend_user("p_tenant1", 10001)
        .await
        .expect("suspend user over IPC");
    assert_eq!(suspend_res.username, "p_tenant1");
    assert_eq!(suspend_res.uid, 10001);

    let actions2 = actions_ref.lock().map(|l| l.clone()).unwrap_or_default();
    assert!(actions2.iter().any(|a| matches!(a, MockAction::Command { binary, args } if binary == "usermod" && args.contains(&"-L".to_string()) && args.contains(&"/usr/sbin/nologin".to_string()))));
    assert!(actions2.iter().any(|a| matches!(a, MockAction::Command { binary, args } if binary == "pkill" && args.contains(&"10001".to_string()))));

    // 6. Unsuspend User
    let unsuspend_res = client
        .unsuspend_user("p_tenant1", 10001, Some("/bin/bash".to_string()))
        .await
        .expect("unsuspend user over IPC");
    assert_eq!(unsuspend_res.username, "p_tenant1");
    assert_eq!(unsuspend_res.uid, 10001);

    let actions3 = actions_ref.lock().map(|l| l.clone()).unwrap_or_default();
    assert!(actions3.iter().any(|a| matches!(a, MockAction::Command { binary, args } if binary == "usermod" && args.contains(&"-U".to_string()) && args.contains(&"/bin/bash".to_string()))));

    // 7. Delete User (Retention mode)
    let del_retention = client
        .delete_user(
            "p_tenant1",
            10001,
            false,
            Some("/var/lib/panel/trash-index".to_string()),
        )
        .await
        .expect("delete user retention over IPC");
    assert!(!del_retention.purged);
    assert!(del_retention.archived_path.is_some());

    let actions4 = actions_ref.lock().map(|l| l.clone()).unwrap_or_default();
    assert!(actions4.iter().any(
        |a| matches!(a, MockAction::MovePath { from, .. } if from == Path::new("/home/p_tenant1"))
    ));
    assert!(actions4.iter().any(|a| matches!(a, MockAction::Command { binary, args } if binary == "userdel" && args.contains(&"p_tenant1".to_string()))));

    // 8. Delete User (Purge mode)
    let del_purge = client
        .delete_user("p_tenant1", 10001, true, None)
        .await
        .expect("delete user purge over IPC");
    assert!(del_purge.purged);
    assert!(del_purge.archived_path.is_none());

    let actions5 = actions_ref.lock().map(|l| l.clone()).unwrap_or_default();
    assert!(actions5.iter().any(
        |a| matches!(a, MockAction::RemoveDir { path } if path == Path::new("/home/p_tenant1"))
    ));

    // Clean shutdown of agent server
    let _ = shutdown_tx.send(());
    let _ = server_task.await;
}
