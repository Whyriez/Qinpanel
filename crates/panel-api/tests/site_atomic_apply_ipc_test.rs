#![allow(clippy::unwrap_used, clippy::expect_used)]

use panel_agent::system::MockSystemRunner;
use panel_agent::webserver::NginxManager;
use panel_agent::{AgentConfig, AgentServer};
use panel_api::agent_client::AgentClient;
use panel_proto::{NginxBaselineApplyRequest, SiteApplyRequest};
use panel_webserver::template::{NginxTemplateRenderer, StaticVhostContext};
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;

#[tokio::test]
async fn test_site_atomic_apply_and_rollback_over_ipc() {
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
    // Initial mock responses: nginx -t passes, systemctl reload passes
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

    // 1. Install baseline first
    let baseline_req = NginxBaselineApplyRequest {
        base_dir: Some(nginx_dir.to_string_lossy().to_string()),
        log_dir: Some(log_dir.to_string_lossy().to_string()),
        acme_dir: Some(acme_dir.to_string_lossy().to_string()),
        panel_api_port: Some(7700),
        panel_listen_port: Some(17890),
        test_config: true,
        reload: true,
    };
    let base_res = client
        .apply_nginx_baseline(baseline_req)
        .await
        .expect("apply baseline");
    assert!(base_res.test_passed);

    // 2. Render and apply an initial valid static site
    let renderer = NginxTemplateRenderer::new();
    let site_ctx = StaticVhostContext {
        site_id: "site_prod_1".to_string(),
        primary_domain: "example.org".to_string(),
        alias_domains: vec!["www.example.org".to_string()],
        docroot: "/home/p_alice/sites/example.org/public".to_string(),
        log_dir: log_dir
            .join("p_alice/example.org")
            .to_string_lossy()
            .to_string(),
        snippets_dir: nginx_dir.join("snippets").to_string_lossy().to_string(),
        index_files: Some("index.html".to_string()),
        spa_fallback: false,
        has_ssl: false,
        ssl_cert: None,
        ssl_key: None,
        force_https: false,
        ocsp_stapling: false,
        custom_headers: vec![],
        redirects: vec![],
        site_rules_block: None,
    };

    let valid_conf = renderer.render_static(&site_ctx).expect("render static");
    let apply_req = SiteApplyRequest {
        site_id: "site_prod_1".to_string(),
        config_content: valid_conf.clone(),
        enable: true,
        nginx_base_dir: Some(nginx_dir.to_string_lossy().to_string()),
        log_context: None,
        test_config: true,
        reload: true,
    };

    let apply_res = client
        .apply_site(apply_req)
        .await
        .expect("apply initial site");
    assert!(apply_res.enabled);
    assert_eq!(apply_res.site_id, "site_prod_1");

    let written_conf_path = nginx_dir.join("sites-available/site_prod_1.conf");
    let enabled_link_path = nginx_dir.join("sites-enabled/site_prod_1.conf");
    assert!(written_conf_path.exists());
    assert!(enabled_link_path.exists());

    let saved_content = fs::read_to_string(&written_conf_path).expect("read initial conf");
    assert!(saved_content.contains("server_name example.org www.example.org;"));

    // 3. Now deliberately configure mock runner to simulate `nginx -t` syntax failure
    runner.set_response(
        "nginx",
        1,
        b"",
        b"nginx: [emerg] unknown directive 'invalid_broken_directive' in /etc/panel/nginx/sites-available/site_prod_1.conf:42",
    );

    let broken_conf = "server { invalid_broken_directive; }";
    let bad_apply_req = SiteApplyRequest {
        site_id: "site_prod_1".to_string(),
        config_content: broken_conf.to_string(),
        enable: true,
        nginx_base_dir: Some(nginx_dir.to_string_lossy().to_string()),
        log_context: None,
        test_config: true,
        reload: true,
    };

    let bad_res = client.apply_site(bad_apply_req).await;
    assert!(bad_res.is_err(), "Broken config must fail validation!");
    let err_msg = bad_res.unwrap_err().to_string();
    assert!(
        err_msg.contains("invalid_broken_directive") || err_msg.contains("rolled_back: true"),
        "Error message should mention syntax error and rollback confirmation"
    );

    // 4. CRITICAL AUTOMATIC ROLLBACK VERIFICATION:
    // The file on disk MUST have been restored to its original working state!
    let active_content = fs::read_to_string(&written_conf_path).expect("read restored conf");
    assert_eq!(
        active_content, saved_content,
        "Original configuration must be preserved after automatic rollback!"
    );
    assert!(
        !active_content.contains("invalid_broken_directive"),
        "Broken directive must not be present in active config!"
    );
    assert!(enabled_link_path.exists());

    let _ = shutdown_tx.send(());
    let _ = server_task.await;
}
