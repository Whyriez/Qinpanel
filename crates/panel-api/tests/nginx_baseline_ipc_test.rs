#![allow(clippy::unwrap_used, clippy::expect_used)]

use panel_agent::system::MockSystemRunner;
use panel_agent::webserver::NginxManager;
use panel_agent::{AgentConfig, AgentServer};
use panel_api::agent_client::AgentClient;
use panel_proto::NginxBaselineApplyRequest;
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;

#[tokio::test]
async fn test_nginx_baseline_install_and_apply_over_ipc() {
    let temp_ipc = tempdir().expect("tempdir ipc");
    let temp_base = tempdir().expect("tempdir base");

    #[cfg(unix)]
    let sock_path = temp_ipc.path().join("agent.sock");
    #[cfg(not(unix))]
    let sock_path = temp_ipc.path().join("agent.port");

    let nginx_dir = temp_base.path().join("nginx");
    let log_dir = temp_base.path().join("logs");
    let acme_dir = temp_base.path().join("acme");

    let runner = Arc::new(MockSystemRunner::new());
    runner.set_response("nginx", 0, b"syntax is ok", b"");
    runner.set_response("systemctl", 0, b"", b"");

    let user_manager = Arc::new(panel_agent::user::UserLifecycleManager::new(
        Arc::clone(&runner) as Arc<dyn panel_agent::system::SystemRunner>,
    ));
    let slice_manager = Arc::new(panel_agent::slice::SliceManager::new(
        Arc::clone(&runner) as Arc<dyn panel_agent::system::SystemRunner>
    ));
    let quota_manager = Arc::new(panel_agent::quota::QuotaManager::new(
        Arc::clone(&runner) as Arc<dyn panel_agent::system::SystemRunner>
    ));
    let resource_reader = Arc::new(panel_agent::resource::CgroupReader::default());
    let hardening_manager = Arc::new(panel_agent::hardening::HardeningManager::new(Arc::clone(
        &runner,
    )
        as Arc<dyn panel_agent::system::SystemRunner>));
    let webserver_manager = Arc::new(NginxManager::new(
        Arc::clone(&runner) as Arc<dyn panel_agent::system::SystemRunner>
    ));

    let config = AgentConfig {
        socket_path: sock_path.clone(),
        allowed_uid: None,
    };

    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
    let server = Arc::new(AgentServer::with_all_managers(
        config,
        user_manager,
        slice_manager,
        quota_manager,
        resource_reader,
        hardening_manager,
        webserver_manager,
    ));

    let server_task = tokio::spawn(async move {
        server.run(shutdown_rx).await.expect("agent server run");
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let client = AgentClient::new(&sock_path);

    // Apply baseline via IPC
    let req = NginxBaselineApplyRequest {
        base_dir: Some(nginx_dir.to_string_lossy().to_string()),
        log_dir: Some(log_dir.to_string_lossy().to_string()),
        acme_dir: Some(acme_dir.to_string_lossy().to_string()),
        panel_api_port: Some(7700),
        panel_listen_port: Some(17890),
        test_config: true,
        reload: true,
    };

    let res = client
        .apply_nginx_baseline(req)
        .await
        .expect("apply baseline over ipc");

    assert!(res.test_passed);
    assert!(res.reloaded);

    // Verify written filesystem layout
    let nginx_conf = fs::read_to_string(&res.nginx_conf_path).expect("read nginx.conf");
    assert!(nginx_conf.contains("worker_processes auto;"));
    assert!(nginx_conf.contains("worker_connections 4096;"));
    assert!(nginx_conf.contains("server_tokens off;"));
    assert!(nginx_conf.contains("log_format panel_json escape=json"));
    assert!(nginx_conf.contains("\"upstream_response_time\":\"$upstream_response_time\""));
    assert!(nginx_conf.contains("sites-enabled/*.conf;"));

    // Verify 00-default catch-all rejects unknown hosts with HTTP 444
    let default_conf =
        fs::read_to_string(nginx_dir.join("conf.d/00-default.conf")).expect("read 00-default.conf");
    assert!(default_conf.contains("listen 80 default_server;"));
    assert!(default_conf.contains("listen 443 ssl default_server;"));
    assert!(default_conf.contains("ssl_reject_handshake on;"));
    assert!(default_conf.contains("return 444;"));

    // Verify 10-panel proxy vhost
    let panel_conf =
        fs::read_to_string(nginx_dir.join("conf.d/10-panel.conf")).expect("read 10-panel.conf");
    assert!(panel_conf.contains("listen 17890;"));
    assert!(panel_conf.contains("proxy_pass http://127.0.0.1:7700;"));

    // Verify security snippets
    assert!(nginx_dir.join("snippets/acme.conf").exists());
    assert!(nginx_dir.join("snippets/security-headers.conf").exists());
    assert!(nginx_dir.join("snippets/deny-sensitive.conf").exists());
    assert!(nginx_dir.join("snippets/fastcgi-params.conf").exists());
    assert!(nginx_dir.join("sites-available").is_dir());
    assert!(nginx_dir.join("sites-enabled").is_dir());

    // Verify system runner invocations (nginx -t and systemctl reload nginx)
    let actions = runner.recorded_actions();
    assert_eq!(actions.len(), 2);

    let _ = shutdown_tx.send(());
    let _ = server_task.await;
}
