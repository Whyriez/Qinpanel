#![allow(clippy::unwrap_used, clippy::expect_used)]

use panel_agent::pty::{MockPtyBackend, PtyManagerConfig, PtySessionManager};
use panel_agent::{AgentConfig, AgentServer};
use panel_api::agent_client::AgentClient;
use panel_proto::{
    PtyCloseRequest, PtyInputRequest, PtyListRequest, PtyOpenRequest, PtyReadOutputRequest,
    PtyResizeRequest,
};
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn test_pty_ipc_lifecycle_and_security() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let sock_path = tmp.path().join("agent.sock");

    let alice_home = tmp.path().join("home/p_alice");
    std::fs::create_dir_all(alice_home.join("sites/example.com")).expect("mkdir alice home");

    let config = AgentConfig {
        socket_path: sock_path.clone(),
        allowed_uid: None,
    };

    let pty_cfg = PtyManagerConfig {
        home_base: tmp.path().join("home"),
        default_shell: "/bin/bash".to_string(),
        default_term: "xterm-256color".to_string(),
        max_sessions_per_tenant: 3,
        idle_timeout: Duration::from_secs(60),
        cgroup_base: tmp.path().join("cgroup"),
    };

    let mock_pty = Arc::new(MockPtyBackend::new());
    let pty_manager = Arc::new(PtySessionManager::new(pty_cfg, mock_pty));

    let server = Arc::new(AgentServer::new(config).with_pty_manager(pty_manager));
    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);

    let server_task = tokio::spawn(async move {
        server.run(shutdown_rx).await.expect("agent server run");
    });

    // Allow socket to bind
    tokio::time::sleep(Duration::from_millis(100)).await;

    let client = AgentClient::new(sock_path);

    // 1. Open PTY session
    let open_res = client
        .pty_open(PtyOpenRequest {
            username: "p_alice".to_string(),
            uid: 10001,
            gid: 10001,
            cwd: Some("sites/example.com".to_string()),
            rows: 24,
            cols: 80,
            term: Some("xterm-256color".to_string()),
            max_sessions: Some(3),
            idle_timeout_secs: Some(60),
        })
        .await
        .expect("pty_open");

    assert_eq!(open_res.username, "p_alice");
    assert_eq!(open_res.rows, 24);
    assert_eq!(open_res.cols, 80);
    let session_id = open_res.session_id;

    // 2. Read initial banner
    let read_res = client
        .pty_read_output(PtyReadOutputRequest {
            session_id: session_id.clone(),
            max_bytes: 1024,
        })
        .await
        .expect("pty_read_output");
    assert!(!read_res.data.is_empty());
    let text = String::from_utf8_lossy(&read_res.data);
    assert!(text.contains("Welcome to Qinpanel Shell"));

    // 3. Write input
    let in_res = client
        .pty_input(PtyInputRequest {
            session_id: session_id.clone(),
            data: b"pwd\n".to_vec(),
        })
        .await
        .expect("pty_input");
    assert_eq!(in_res.bytes_written, 4);

    // 4. Resize terminal
    let resize_res = client
        .pty_resize(PtyResizeRequest {
            session_id: session_id.clone(),
            rows: 50,
            cols: 132,
        })
        .await
        .expect("pty_resize");
    assert_eq!(resize_res.rows, 50);
    assert_eq!(resize_res.cols, 132);

    // 5. List sessions
    let list_res = client
        .pty_list(PtyListRequest {
            username: Some("p_alice".to_string()),
        })
        .await
        .expect("pty_list");
    assert_eq!(list_res.sessions.len(), 1);
    assert_eq!(list_res.sessions[0].session_id, session_id);
    assert_eq!(list_res.sessions[0].rows, 50);
    assert_eq!(list_res.sessions[0].cols, 132);

    // 6. Security rejection: Path traversal attempt
    let traversal_err = client
        .pty_open(PtyOpenRequest {
            username: "p_alice".to_string(),
            uid: 10001,
            gid: 10001,
            cwd: Some("../../etc/shadow".to_string()),
            rows: 24,
            cols: 80,
            term: None,
            max_sessions: None,
            idle_timeout_secs: None,
        })
        .await
        .unwrap_err();
    assert!(
        traversal_err.to_string().contains("Path traversal")
            || traversal_err.to_string().contains("escapes")
    );

    // 7. Close session
    let close_res = client
        .pty_close(PtyCloseRequest {
            session_id: session_id.clone(),
            force: true,
        })
        .await
        .expect("pty_close");
    assert!(close_res.closed);

    // 8. Verify list is now empty
    let list_res2 = client
        .pty_list(PtyListRequest { username: None })
        .await
        .expect("pty_list after close");
    assert_eq!(list_res2.sessions.len(), 0);

    // Shutdown agent
    let _ = shutdown_tx.send(());
    let _ = server_task.await;
}
