#![allow(clippy::unwrap_used, clippy::expect_used)]
use panel_agent::{
    server_backup::execute,
    system::{MockSystemRunner, SystemRunner},
};
use panel_proto::{
    ServerBackupJob, ServerBackupOperation as Operation, ServerBackupRequest as Request,
};
use serde_json::json;
use std::path::Path;
const ID: &str = "12345678-1234-1234-1234-123456789abc";
fn runner() -> MockSystemRunner {
    let runner = MockSystemRunner::new();
    runner.add_existing_path("/usr/local/libexec/qonpanel-server-backup");
    runner.add_existing_path("/etc/systemd/system/qonpanel-server-backup.service");
    runner.set_response(
        "systemctl is-active --quiet qonpanel-server-backup.service",
        3,
        b"",
        b"inactive",
    );
    runner.set_response(
        "systemctl is-active --quiet panel-update.service",
        3,
        b"",
        b"inactive",
    );
    runner.set_response(
        "systemctl show qonpanel-server-backup.service --property=ActiveState,Job --no-pager",
        0,
        b"ActiveState=inactive\nJob=42\n",
        b"",
    );
    runner
}
fn job() -> ServerBackupJob {
    ServerBackupJob {
        id: ID.into(),
        operation: Operation::Create,
        archive_id: None,
        digest: None,
        new_ipv4: None,
        passphrase: "fixture-only-secret-passphrase".into(),
    }
}
#[test]
fn root_handoff_is_typed_redacted_durable_and_idempotent() {
    let r = runner();
    let request = job();
    assert!(!format!("{request:?}").contains("fixture-only"));
    let first = execute(&r, Request::Start(request.clone())).unwrap();
    assert_eq!(first["job"]["phase"], "queued");
    let serialized = r
        .read_file(Path::new("/var/lib/qonpanel-migration/request.json"))
        .unwrap();
    assert!(
        String::from_utf8(serialized)
            .unwrap()
            .contains("fixture-only")
    );
    assert!(
        !serde_json::to_string(&first)
            .unwrap()
            .contains("fixture-only")
    );
    execute(&r, Request::Start(request)).unwrap();
    assert_eq!(
        r.get_commands()
            .iter()
            .filter(
                |cmd| cmd.as_str() == "systemctl start --no-block qonpanel-server-backup.service"
            )
            .count(),
        1
    );
    assert!(
        !r.get_commands()
            .iter()
            .any(|cmd| cmd.contains("python") || cmd.contains("tar ") || cmd.contains("gpg "))
    );
    let mut changed = job();
    changed.operation = Operation::Inspect;
    changed.archive_id = Some(ID.into());
    assert!(execute(&r, Request::Start(changed)).is_err());
}
#[test]
fn unsafe_jobs_paths_short_secrets_and_conflicting_upgrades_never_schedule() {
    for operation in [Operation::Inspect, Operation::Restore] {
        let r = runner();
        let mut request = job();
        request.operation = operation;
        request.archive_id = Some("../etc/shadow".into());
        assert!(execute(&r, Request::Start(request)).is_err());
        assert!(
            r.get_commands()
                .iter()
                .all(|cmd| !cmd.starts_with("systemctl start"))
        );
    }
    let r = runner();
    let mut request = job();
    request.passphrase = "short".into();
    assert!(execute(&r, Request::Start(request)).is_err());
    let r = runner();
    r.set_response(
        "systemctl is-active --quiet panel-update.service",
        0,
        b"active",
        b"",
    );
    assert!(execute(&r, Request::Start(job())).is_err());
    assert!(serde_json::from_value::<Request>(json!({"operation":"start","request":{"id":ID,"operation":"create","passphrase":"long-enough-fixture-password","command":"id"}})).is_err());
}
#[test]
fn start_failure_is_recorded_and_handoff_secret_is_removed() {
    let r = runner();
    r.set_response(
        "systemctl start --no-block qonpanel-server-backup.service",
        1,
        b"",
        b"fixture failure",
    );
    assert!(execute(&r, Request::Start(job())).is_err());
    assert!(!r.path_exists(Path::new("/var/lib/qonpanel-migration/request.json")));
    assert_eq!(
        execute(&r, Request::Status).unwrap()["job"]["phase"],
        "failed"
    );
}

#[test]
fn restore_requires_root_approval_which_cannot_be_created_by_rpc() {
    let r = runner();
    let mut request = job();
    request.operation = Operation::Restore;
    request.archive_id = Some(ID.into());
    request.digest = Some("a".repeat(64));
    assert!(execute(&r, Request::Start(request.clone())).is_err());
    assert!(
        !r.get_commands()
            .iter()
            .any(|command| command.starts_with("systemctl start"))
    );
    assert!(
        serde_json::from_value::<Request>(json!({"operation":"approve","digest":"a".repeat(64)}))
            .is_err()
    );
    r.write_file(
        Path::new("/var/lib/qonpanel-migration/approval.json"),
        &serde_json::to_vec(
            &json!({"digest":"a".repeat(64),"expires_at":chrono::Utc::now().timestamp()+86400}),
        )
        .unwrap(),
        0o600,
    )
    .unwrap();
    assert_eq!(
        execute(&r, Request::Start(request)).unwrap()["job"]["phase"],
        "queued"
    );
}
#[test]
fn reboot_marks_interrupted_only_after_systemd_has_no_queued_job() {
    let r = runner();
    execute(&r, Request::Start(job())).unwrap();
    assert_eq!(
        execute(&r, Request::Status).unwrap()["job"]["phase"],
        "queued"
    );
    r.write_file(
        Path::new("/var/lib/qonpanel-migration/state.json"),
        &serde_json::to_vec(&json!({"id":ID,"phase":"queued","started_at":0})).unwrap(),
        0o600,
    )
    .unwrap();
    r.set_response(
        "systemctl show qonpanel-server-backup.service --property=ActiveState,Job --no-pager",
        0,
        b"ActiveState=inactive\nJob=0\n",
        b"",
    );
    assert_eq!(
        execute(&r, Request::Status).unwrap()["job"]["phase"],
        "interrupted"
    );
    assert_eq!(
        r.get_commands()
            .iter()
            .filter(|cmd| cmd.starts_with("systemctl start"))
            .count(),
        1
    );
}
