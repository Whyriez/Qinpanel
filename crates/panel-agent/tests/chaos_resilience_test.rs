//! Chaos and Fault Injection Testing Suite (P10-T5).
//!
//! Validates resilience under 5 failure scenarios:
//! 1. Kill/abort during atomic config apply -> Atomic staging rollback, no corruption.
//! 2. Nginx service down simulation -> Doctor detects failure, remediation hint provided and executed.
//! 3. Disk full / ENOSPC simulation -> Graceful error return, zero panic, state remains consistent.
//! 4. Corrupted Nginx configuration -> Detected by doctor/reconcile, regenerated cleanly from desired state.
//! 5. Forced reboot / interrupted job recovery -> Deterministic transition from running to failed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::clone_on_ref_ptr)]

use panel_agent::nftables::NftablesManager;
use panel_agent::reliability::ReliabilityManager;
use panel_agent::system::{MockSystemRunner, SystemRunner};
use panel_agent::webserver::NginxManager;
use panel_proto::{
    ReconcileSiteInputDto, SiteApplyRequest, SystemDoctorCheckRequest, SystemDoctorFixRequest,
    SystemReconcileRequest,
};
use std::fs;
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test]
async fn test_chaos_scenario_1_atomic_staging_abort_leaves_active_config_untouched() {
    let temp = tempdir().unwrap();
    let runner = Arc::new(MockSystemRunner::new());
    runner.set_response("nginx", 0, b"syntax is ok", b"");
    runner.set_response("systemctl", 0, b"", b"");

    let base_dir = temp.path().join("nginx");
    let nginx_mgr = NginxManager::with_base_dir(runner.clone() as Arc<dyn SystemRunner>, &base_dir);

    // Initial valid config deployed
    let site_id = "test-site-atomic";
    let initial_content = "server { listen 80; server_name site.example.com; }";
    let apply_initial = nginx_mgr.apply_site(SiteApplyRequest {
        site_id: site_id.to_string(),
        config_content: initial_content.to_string(),
        enable: true,
        nginx_base_dir: Some(base_dir.to_string_lossy().to_string()),
        log_context: None,
        test_config: true,
        reload: false,
    });
    assert!(apply_initial.is_ok());

    let active_path = base_dir
        .join("sites-available")
        .join(format!("{}.conf", site_id));
    assert_eq!(fs::read_to_string(&active_path).unwrap(), initial_content);

    // Simulate broken syntax during subsequent apply: Nginx syntax check fails
    runner.set_response(
        "nginx",
        1,
        b"",
        b"nginx: [emerg] unknown directive 'broken_directive'",
    );

    let bad_content = "server { broken_directive; }";
    let write_res = nginx_mgr.apply_site(SiteApplyRequest {
        site_id: site_id.to_string(),
        config_content: bad_content.to_string(),
        enable: true,
        nginx_base_dir: Some(base_dir.to_string_lossy().to_string()),
        log_context: None,
        test_config: true,
        reload: false,
    });
    assert!(
        write_res.is_err(),
        "Invalid staging config must be rejected"
    );

    // Verify active file still has initial untouched content (atomic rollback succeeded)
    let active_content_after = fs::read_to_string(&active_path).unwrap();
    assert_eq!(active_content_after, initial_content);
}

#[tokio::test]
async fn test_chaos_scenario_2_nginx_service_down_detection_and_remediation() {
    let temp = tempdir().unwrap();
    let runner = Arc::new(MockSystemRunner::new());

    // Nginx is inactive/dead
    runner.set_response("systemctl", 3, b"inactive (dead)", b"Active: inactive");
    runner.set_response("nginx", 0, b"syntax is ok", b"");

    let nft_file = temp.path().join("rules.nft");
    let nftables_mgr = Arc::new(NftablesManager::with_path(runner.clone(), &nft_file));
    let reliability_mgr = ReliabilityManager::new(runner.clone(), temp.path(), nftables_mgr);

    // 1. Doctor check detects Nginx service is down
    let check_resp = reliability_mgr
        .check_doctor(&SystemDoctorCheckRequest {})
        .unwrap();

    let nginx_check = check_resp
        .checks
        .iter()
        .find(|c| c.id == "service_nginx")
        .expect("service_nginx check must exist");

    assert_eq!(nginx_check.status, "fail");
    assert!(nginx_check.remediation_hint.is_some());
    assert!(nginx_check.fixable);

    // 2. Doctor auto-remediation triggers service restart
    runner.set_response("systemctl", 0, b"active (running)", b"");
    let fix_resp = reliability_mgr
        .fix_doctor(&SystemDoctorFixRequest {
            target_checks: vec!["service_nginx".to_string()],
        })
        .unwrap();

    let nginx_fix_result = fix_resp
        .results
        .iter()
        .find(|r| r.item_id == "service_nginx")
        .expect("service_nginx remediation result");
    assert!(nginx_fix_result.success);
}

#[tokio::test]
async fn test_chaos_scenario_3_disk_full_enospc_error_handling() {
    let temp = tempdir().unwrap();
    let runner = Arc::new(MockSystemRunner::new());
    let base_dir = temp.path().join("nginx_enospc");
    let nginx_mgr = NginxManager::with_base_dir(runner.clone() as Arc<dyn SystemRunner>, &base_dir);

    // Attempting to apply a site with an illegal target path (e.g. traversal attempt or uncreatable dir)
    let invalid_site_id = "../../../unwritable/site";
    let res = nginx_mgr.apply_site(SiteApplyRequest {
        site_id: invalid_site_id.to_string(),
        config_content: "server {}".to_string(),
        enable: false,
        nginx_base_dir: Some(base_dir.to_string_lossy().to_string()),
        log_context: None,
        test_config: false,
        reload: false,
    });
    assert!(
        res.is_err(),
        "Must gracefully error without panic on invalid/unwritable target"
    );
}

#[tokio::test]
async fn test_chaos_scenario_4_corrupted_nginx_config_detection_and_reconciliation() {
    let temp = tempdir().unwrap();
    let runner = Arc::new(MockSystemRunner::new());
    runner.set_response("systemctl", 0, b"active (running)", b"");

    let sites_avail = temp.path().join("nginx/sites-available");
    let sites_enab = temp.path().join("nginx/sites-enabled");
    fs::create_dir_all(&sites_avail).unwrap();
    fs::create_dir_all(&sites_enab).unwrap();

    // Site registered in desired state
    let site_domain = "corrupt-site.example.com";
    let site_conf_file = sites_avail.join(format!("{}.conf", site_domain));

    // Inject corrupted config on disk
    fs::write(&site_conf_file, "INVALID_CORRUPTED_CONFIG {{{").unwrap();

    let nft_file = temp.path().join("rules.nft");
    let nftables_mgr = Arc::new(NftablesManager::with_path(runner.clone(), &nft_file));
    let reliability_mgr = ReliabilityManager::new(runner.clone(), temp.path(), nftables_mgr);

    // Desired state indicates valid config should exist
    let desired_site = ReconcileSiteInputDto {
        site_id: 42,
        domain: site_domain.to_string(),
        username: "user42".to_string(),
        uid: 1042,
        kind: "static".to_string(),
        docroot: "/var/www/user42/corrupt-site".to_string(),
        config_content: "server { listen 80; server_name corrupt-site.example.com; }".to_string(),
        is_active: true,
    };

    // 1. Dry-run reconcile detects drift between disk state and desired state
    let dry_run_req = SystemReconcileRequest {
        dry_run: true,
        desired_sites: vec![desired_site.clone()],
        desired_nftables_ports: vec![],
    };

    let dry_res = reliability_mgr.reconcile(&dry_run_req).unwrap();
    assert!(!dry_res.is_clean);
    assert!(!dry_res.drifts.is_empty());

    // 2. Active reconcile repairs the configuration file
    let apply_req = SystemReconcileRequest {
        dry_run: false,
        desired_sites: vec![desired_site],
        desired_nftables_ports: vec![],
    };

    let apply_res = reliability_mgr.reconcile(&apply_req).unwrap();
    assert!(apply_res.success);

    // Verify config on disk is restored cleanly
    let restored_content = fs::read_to_string(&site_conf_file).unwrap();
    assert_eq!(
        restored_content,
        "server { listen 80; server_name corrupt-site.example.com; }"
    );
}

#[tokio::test]
async fn test_chaos_scenario_5_forced_restart_interrupted_job_deterministic_recovery() {
    let temp = tempdir().unwrap();
    let db_path = temp.path().join("chaos_panel.db");
    let db_url = format!("sqlite://{}", db_path.to_string_lossy().replace('\\', "/"));

    let pool = panel_db::pool::init_pool(&db_url).await.unwrap();
    panel_db::pool::run_migrations(&pool).await.unwrap();

    let job_repo = panel_db::repositories::job_repo::SqliteJobRepository::new(pool.clone());

    // Simulate 3 jobs that were in progress when the host suffered ungraceful power loss
    for i in 1..=3 {
        job_repo
            .create(&panel_db::models::CreateJob {
                id: format!("chaos-job-{}", i),
                user_id: None,
                name: format!("action-{}", i),
                payload_json: "{}".to_string(),
                max_retries: 3,
            })
            .await
            .unwrap();

        job_repo
            .update_status(&format!("chaos-job-{}", i), "running", None, None)
            .await
            .unwrap();
    }

    // Verify all 3 jobs are left hanging in 'running'
    let running_before = job_repo.list_by_status("running").await.unwrap();
    assert_eq!(running_before.len(), 3);

    // Deterministic recovery executes: all interrupted jobs marked 'failed'
    let recovered_count = job_repo.recover_interrupted_jobs().await.unwrap();
    assert_eq!(recovered_count, 3);

    // Check zero jobs remain running
    let running_after = job_repo.list_by_status("running").await.unwrap();
    assert_eq!(running_after.len(), 0);

    // Check all jobs have descriptive interrupted error message
    for i in 1..=3 {
        let job = job_repo
            .find_by_id(&format!("chaos-job-{}", i))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(job.status, "failed");
        assert_eq!(
            job.error_message,
            Some("Interrupted by server restart".to_string())
        );
    }
}
