//! Unit tests for PTY session management, path confinement, quota enforcement, and idle reaping.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::AgentError;
use panel_proto::{
    PtyCloseRequest, PtyInputRequest, PtyListRequest, PtyOpenRequest, PtyReadOutputRequest,
    PtyResizeRequest,
};
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn test_pty_session_lifecycle() {
    let temp_home = tempfile::tempdir().expect("tempdir");
    let alice_home = temp_home.path().join("p_alice");
    std::fs::create_dir_all(&alice_home).expect("mkdir p_alice");

    let config = PtyManagerConfig {
        home_base: temp_home.path().to_path_buf(),
        default_shell: "/bin/bash".to_string(),
        default_term: "xterm-256color".to_string(),
        max_sessions_per_tenant: 5,
        idle_timeout: Duration::from_secs(300),
        cgroup_base: temp_home.path().join("cgroup"),
    };

    let backend = Arc::new(MockPtyBackend::new());
    let manager = PtySessionManager::new(config, backend);

    // 1. Open session
    let open_req = PtyOpenRequest {
        username: "p_alice".to_string(),
        uid: 10001,
        gid: 10001,
        cwd: None,
        rows: 24,
        cols: 80,
        term: None,
        max_sessions: None,
        idle_timeout_secs: None,
    };

    let open_res = manager.open_session(open_req).await.expect("open_session");
    assert_eq!(open_res.username, "p_alice");
    assert_eq!(open_res.rows, 24);
    assert_eq!(open_res.cols, 80);
    assert!(open_res.pid >= 20000);
    let session_id = open_res.session_id;

    // 2. Read initial welcome banner
    let read_req = PtyReadOutputRequest {
        session_id: session_id.clone(),
        max_bytes: 1024,
    };
    let read_res = manager.read_output(read_req).await.expect("read_output");
    assert!(!read_res.data.is_empty());
    let output_text = String::from_utf8_lossy(&read_res.data);
    assert!(output_text.contains("Welcome to Qinpanel Shell"));

    // 3. Write input command
    let in_req = PtyInputRequest {
        session_id: session_id.clone(),
        data: b"whoami\n".to_vec(),
    };
    let in_res = manager.write_input(in_req).await.expect("write_input");
    assert_eq!(in_res.bytes_written, 7);

    // 4. Read output after command
    let read_res2 = manager
        .read_output(PtyReadOutputRequest {
            session_id: session_id.clone(),
            max_bytes: 1024,
        })
        .await
        .expect("read_output 2");
    assert!(!read_res2.data.is_empty());

    // 5. Resize terminal
    let resize_req = PtyResizeRequest {
        session_id: session_id.clone(),
        rows: 40,
        cols: 120,
    };
    let resize_res = manager.resize(resize_req).await.expect("resize");
    assert_eq!(resize_res.rows, 40);
    assert_eq!(resize_res.cols, 120);

    // 6. List sessions
    let list_res = manager
        .list_sessions(PtyListRequest { username: None })
        .await
        .expect("list_sessions");
    assert_eq!(list_res.sessions.len(), 1);
    assert_eq!(list_res.sessions[0].session_id, session_id);
    assert_eq!(list_res.sessions[0].rows, 40);
    assert_eq!(list_res.sessions[0].cols, 120);

    // 7. Close session
    let close_res = manager
        .close(PtyCloseRequest {
            session_id: session_id.clone(),
            force: false,
        })
        .await
        .expect("close");
    assert!(close_res.closed);

    // 8. Verify list is empty
    let list_res2 = manager
        .list_sessions(PtyListRequest { username: None })
        .await
        .expect("list_sessions after close");
    assert_eq!(list_res2.sessions.len(), 0);
}

#[tokio::test]
async fn test_pty_path_confinement_security() {
    let temp_home = tempfile::tempdir().expect("tempdir");
    let alice_home = temp_home.path().join("p_alice");
    std::fs::create_dir_all(alice_home.join("sites/example.com")).expect("mkdir sites");

    let config = PtyManagerConfig {
        home_base: temp_home.path().to_path_buf(),
        default_shell: "/bin/bash".to_string(),
        default_term: "xterm-256color".to_string(),
        max_sessions_per_tenant: 5,
        idle_timeout: Duration::from_secs(300),
        cgroup_base: temp_home.path().join("cgroup"),
    };

    let backend = Arc::new(MockPtyBackend::new());
    let manager = PtySessionManager::new(config, backend);

    // 1. Valid nested folder inside tenant home succeeds
    let valid_req = PtyOpenRequest {
        username: "p_alice".to_string(),
        uid: 10001,
        gid: 10001,
        cwd: Some("sites/example.com".to_string()),
        rows: 24,
        cols: 80,
        term: None,
        max_sessions: None,
        idle_timeout_secs: None,
    };
    let res = manager.open_session(valid_req).await.expect("valid open");
    assert!(res.cwd.contains("sites"));

    // 2. Traversal attack with '..' is rejected with SecurityViolation
    let traversal_req = PtyOpenRequest {
        username: "p_alice".to_string(),
        uid: 10001,
        gid: 10001,
        cwd: Some("../../etc/passwd".to_string()),
        rows: 24,
        cols: 80,
        term: None,
        max_sessions: None,
        idle_timeout_secs: None,
    };
    let err = manager.open_session(traversal_req).await.unwrap_err();
    assert!(matches!(err, AgentError::SecurityViolation(_)));

    // 3. Absolute path outside tenant home is rejected
    let abs_escape_req = PtyOpenRequest {
        username: "p_alice".to_string(),
        uid: 10001,
        gid: 10001,
        cwd: Some("/var/log".to_string()),
        rows: 24,
        cols: 80,
        term: None,
        max_sessions: None,
        idle_timeout_secs: None,
    };
    let err2 = manager.open_session(abs_escape_req).await.unwrap_err();
    assert!(matches!(err2, AgentError::SecurityViolation(_)));
}

#[tokio::test]
async fn test_pty_concurrent_session_quota_enforcement() {
    let temp_home = tempfile::tempdir().expect("tempdir");
    let bob_home = temp_home.path().join("p_bob");
    std::fs::create_dir_all(&bob_home).expect("mkdir p_bob");

    let config = PtyManagerConfig {
        home_base: temp_home.path().to_path_buf(),
        default_shell: "/bin/bash".to_string(),
        default_term: "xterm-256color".to_string(),
        max_sessions_per_tenant: 2, // Limit to 2 concurrent sessions
        idle_timeout: Duration::from_secs(300),
        cgroup_base: temp_home.path().join("cgroup"),
    };

    let backend = Arc::new(MockPtyBackend::new());
    let manager = PtySessionManager::new(config, backend);

    // Session 1 succeeds
    let s1 = manager
        .open_session(PtyOpenRequest {
            username: "p_bob".to_string(),
            uid: 10002,
            gid: 10002,
            cwd: None,
            rows: 24,
            cols: 80,
            term: None,
            max_sessions: Some(2),
            idle_timeout_secs: None,
        })
        .await
        .expect("open s1");

    // Session 2 succeeds
    let _s2 = manager
        .open_session(PtyOpenRequest {
            username: "p_bob".to_string(),
            uid: 10002,
            gid: 10002,
            cwd: None,
            rows: 24,
            cols: 80,
            term: None,
            max_sessions: Some(2),
            idle_timeout_secs: None,
        })
        .await
        .expect("open s2");

    // Session 3 must fail due to quota limit
    let s3_err = manager
        .open_session(PtyOpenRequest {
            username: "p_bob".to_string(),
            uid: 10002,
            gid: 10002,
            cwd: None,
            rows: 24,
            cols: 80,
            term: None,
            max_sessions: Some(2),
            idle_timeout_secs: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(s3_err, AgentError::InvalidInput(_)));

    // After closing session 1, a new session can be opened
    manager
        .close(PtyCloseRequest {
            session_id: s1.session_id,
            force: true,
        })
        .await
        .expect("close s1");

    let s4 = manager
        .open_session(PtyOpenRequest {
            username: "p_bob".to_string(),
            uid: 10002,
            gid: 10002,
            cwd: None,
            rows: 24,
            cols: 80,
            term: None,
            max_sessions: Some(2),
            idle_timeout_secs: None,
        })
        .await
        .expect("open s4 after close");
    assert_eq!(s4.username, "p_bob");
}

#[tokio::test]
async fn test_pty_idle_reaper() {
    let temp_home = tempfile::tempdir().expect("tempdir");
    let charlie_home = temp_home.path().join("p_charlie");
    std::fs::create_dir_all(&charlie_home).expect("mkdir p_charlie");

    let config = PtyManagerConfig {
        home_base: temp_home.path().to_path_buf(),
        default_shell: "/bin/bash".to_string(),
        default_term: "xterm-256color".to_string(),
        max_sessions_per_tenant: 5,
        idle_timeout: Duration::from_millis(10), // 10ms idle timeout for test
        cgroup_base: temp_home.path().join("cgroup"),
    };

    let backend = Arc::new(MockPtyBackend::new());
    let manager = PtySessionManager::new(config, backend);

    let s1 = manager
        .open_session(PtyOpenRequest {
            username: "p_charlie".to_string(),
            uid: 10003,
            gid: 10003,
            cwd: None,
            rows: 24,
            cols: 80,
            term: None,
            max_sessions: None,
            idle_timeout_secs: None,
        })
        .await
        .expect("open s1");

    // Wait for idle timeout to expire
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Run reaper cleanup
    let reaped = manager.cleanup_idle_sessions().await;
    assert_eq!(reaped, 1);

    // Verify session was removed
    let list = manager
        .list_sessions(PtyListRequest {
            username: Some("p_charlie".to_string()),
        })
        .await
        .expect("list");
    assert!(list.sessions.is_empty());

    // Reading from reaped session now returns not found
    let err = manager
        .read_output(PtyReadOutputRequest {
            session_id: s1.session_id,
            max_bytes: 1024,
        })
        .await
        .unwrap_err();
    assert!(matches!(err, AgentError::InvalidInput(_)));
}
