#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Hermetic updater tests: all service, network and filesystem operations mocked.
use panel_agent::{
    panel_update::{execute, release_from_metadata},
    system::{MockSystemRunner, SystemRunner},
};
use panel_proto::{PanelUpdateRequest as Request, UpdatePhase};
use serde_json::{Value, json};
use std::path::Path;

const ID: &str = "12345678-1234-1234-1234-123456789abc";
fn digest() -> String {
    "a".repeat(64)
}
fn metadata() -> Value {
    json!({"tag_name":"v0.2.0","draft":false,"prerelease":false,"body":"Release fixture","assets":[{"name":format!("nusa-panel-v0.2.0-{}-unknown-linux-gnu.tar.gz",std::env::consts::ARCH),"state":"uploaded","size":4096,"digest":format!("sha256:{}",digest())},{"name":"SHA256SUMS","state":"uploaded"}]})
}
fn runner() -> MockSystemRunner {
    let runner = MockSystemRunner::new();
    runner.add_existing_path("/usr/local/libexec/panel-update-worker");
    runner.add_existing_path("/etc/systemd/system/panel-update.service");
    runner.set_response(
        "systemctl is-active --quiet panel-update.service",
        3,
        b"",
        b"inactive",
    );
    runner.set_response("curl", 0, &serde_json::to_vec(&metadata()).unwrap(), b"");
    execute(&runner, Request::Check).unwrap();
    runner
}
fn apply(id: &str) -> Request {
    Request::Apply {
        id: id.into(),
        version: "v0.2.0".into(),
        digest: digest(),
    }
}

#[test]
fn stable_complete_pinned_release_only_and_numeric_versions() {
    assert_eq!(
        release_from_metadata(&metadata(), std::env::consts::ARCH)
            .unwrap()
            .digest,
        digest()
    );
    for (key, value) in [
        ("draft", json!(true)),
        ("prerelease", json!(true)),
        ("tag_name", json!("v0.2.0-rc.1")),
        ("tag_name", json!("v00.2.0")),
        ("assets", json!([])),
    ] {
        let mut bad = metadata();
        bad[key] = value;
        assert!(release_from_metadata(&bad, std::env::consts::ARCH).is_err());
    }
    let mut bad = metadata();
    bad["assets"][0]["digest"] = json!("sha256:invalid");
    assert!(release_from_metadata(&bad, std::env::consts::ARCH).is_err());
    for bad in [
        "v1.2.3;id",
        "v1.2",
        "../v1.2.3",
        "v1.02.3",
        "v999999999.0.0",
        "v1.2.3\n",
    ] {
        assert!(panel_core::update::release_version(bad).is_err());
    }
    assert!(
        panel_core::update::release_version("v1.10.0").unwrap()
            > panel_core::update::release_version("v1.9.99").unwrap()
    );
}
#[test]
fn detached_apply_is_durable_idempotent_and_never_calls_an_installer_in_agent() {
    let runner = runner();
    let status = execute(&runner, apply(ID)).unwrap();
    assert_eq!(status.update.as_ref().unwrap().phase, UpdatePhase::Queued);
    assert_eq!(execute(&runner, apply(ID)).unwrap(), status);
    assert!(execute(&runner, apply("12345678-1234-1234-1234-123456789abd")).is_err());
    let commands = runner.get_commands();
    assert_eq!(
        commands
            .iter()
            .filter(|c| c.starts_with("systemctl start"))
            .count(),
        1
    );
    assert!(
        commands
            .iter()
            .any(|c| c == "systemctl start --no-block panel-update.service")
    );
    assert!(
        !commands
            .iter()
            .any(|c| c.starts_with("bash") || c.starts_with("sh ") || c.contains("stop panel"))
    );
    let environment = String::from_utf8(
        runner
            .read_file(Path::new("/var/lib/panel-updater/request.env"))
            .unwrap(),
    )
    .unwrap();
    assert!(environment.contains(ID));
    assert!(!environment.contains("URL="));
    // A new agent instance reads the same queued state after an API/agent restart.
    assert_eq!(
        execute(&runner, Request::Status).unwrap().update,
        status.update
    );
}
#[test]
fn failures_stale_catalog_changed_digest_and_downgrade_never_schedule_work() {
    let runner = runner();
    let bad = Request::Apply {
        id: ID.into(),
        version: "v0.2.0".into(),
        digest: "b".repeat(64),
    };
    assert!(execute(&runner, bad).is_err());
    for version in ["v0.0.1", "v0.2.0\n", "v0.2.0;id"] {
        assert!(
            execute(
                &runner,
                Request::Apply {
                    id: ID.into(),
                    version: version.into(),
                    digest: digest()
                }
            )
            .is_err()
        );
    }
    let mut cache: Value = serde_json::from_slice(
        &runner
            .read_file(Path::new("/var/lib/panel-updater/catalog.json"))
            .unwrap(),
    )
    .unwrap();
    cache["checked_at"] = json!(chrono::Utc::now().timestamp() - 601);
    runner.set_file_content(
        "/var/lib/panel-updater/catalog.json",
        &serde_json::to_vec(&cache).unwrap(),
    );
    assert!(execute(&runner, apply(ID)).is_err());
    assert!(
        !runner
            .get_commands()
            .iter()
            .any(|c| c.starts_with("systemctl start"))
    );
}
#[test]
fn service_start_failure_is_persisted_and_terminal_state_survives_restart() {
    let runner = runner();
    runner.set_response(
        "systemctl start --no-block panel-update.service",
        1,
        b"",
        b"denied",
    );
    assert!(execute(&runner, apply(ID)).is_err());
    assert_eq!(
        execute(&runner, Request::Status)
            .unwrap()
            .update
            .unwrap()
            .phase,
        UpdatePhase::Failed
    );
    assert_eq!(
        execute(&runner, apply(ID)).unwrap().update.unwrap().phase,
        UpdatePhase::Failed
    );
}
#[test]
fn incomplete_release_clears_apply_eligibility_and_interruption_is_not_replayed() {
    let runner = MockSystemRunner::new();
    runner.set_response("curl", 1, b"", b"provider failed");
    let status = execute(&runner, Request::Check).unwrap();
    assert!(status.available.is_none() && status.check_error.is_some());
    let state = json!({"id":ID,"version":"v0.2.0","digest":digest(),"phase":"running","started_at":chrono::Utc::now().timestamp()-120,"finished_at":null});
    runner.set_file_content(
        "/var/lib/panel-updater/state.json",
        &serde_json::to_vec(&state).unwrap(),
    );
    runner.set_response(
        "systemctl show panel-update.service --property=ActiveState,Job",
        0,
        b"ActiveState=inactive\nJob=0\n",
        b"",
    );
    assert_eq!(
        execute(&runner, Request::Status)
            .unwrap()
            .update
            .unwrap()
            .phase,
        UpdatePhase::Interrupted
    );
    assert!(
        !runner
            .get_commands()
            .iter()
            .any(|c| c.starts_with("systemctl start"))
    );
}

#[test]
fn queued_systemd_job_is_not_misclassified_as_interrupted_and_extra_rpc_fields_are_rejected() {
    let runner = MockSystemRunner::new();
    let state = json!({"id":ID,"version":"v0.2.0","digest":digest(),"phase":"queued","started_at":chrono::Utc::now().timestamp()-120,"finished_at":null});
    runner.set_file_content(
        "/var/lib/panel-updater/state.json",
        &serde_json::to_vec(&state).unwrap(),
    );
    runner.set_response(
        "systemctl show panel-update.service --property=ActiveState,Job",
        0,
        b"ActiveState=inactive\nJob=42 /org/freedesktop/systemd1/job/42\n",
        b"",
    );
    assert_eq!(
        execute(&runner, Request::Status)
            .unwrap()
            .update
            .unwrap()
            .phase,
        UpdatePhase::Queued
    );
    assert!(serde_json::from_value::<Request>(json!({"operation":"apply","id":ID,"version":"v0.2.0","digest":digest(),"url":"https://untrusted.invalid"})).is_err());
    assert!(
        !runner
            .get_commands()
            .iter()
            .any(|c| c.starts_with("systemctl start"))
    );
}

#[test]
fn metadata_prefers_primary_and_accepts_compatibility_packages() {
    let mut value = metadata();
    assert!(release_from_metadata(&value, std::env::consts::ARCH).is_ok());
    let mut canonical = value["assets"][0].clone();
    canonical["name"] = json!(format!(
        "qonpanel-v0.2.0-{}-unknown-linux-gnu.tar.gz",
        std::env::consts::ARCH
    ));
    canonical["digest"] = json!(format!("sha256:{}", "b".repeat(64)));
    value["assets"].as_array_mut().unwrap().push(canonical);
    assert_eq!(
        release_from_metadata(&value, std::env::consts::ARCH)
            .unwrap()
            .digest,
        "b".repeat(64)
    );
    let mut primary = value["assets"][0].clone();
    primary["name"] = json!(format!(
        "qinpanel-v0.2.0-{}-unknown-linux-gnu.tar.gz",
        std::env::consts::ARCH
    ));
    primary["digest"] = json!(format!("sha256:{}", "c".repeat(64)));
    value["assets"].as_array_mut().unwrap().push(primary);
    assert_eq!(
        release_from_metadata(&value, std::env::consts::ARCH)
            .unwrap()
            .digest,
        "c".repeat(64)
    );
    // A corrupt primary must not silently select a different archive/digest.
    value["assets"].as_array_mut().unwrap().last_mut().unwrap()["digest"] = json!("sha256:invalid");
    assert!(release_from_metadata(&value, std::env::consts::ARCH).is_err());
}

#[test]
fn metadata_accepts_canonical_only_assets_for_each_supported_architecture() {
    for arch in ["x86_64", "aarch64"] {
        let mut value = metadata();
        value["assets"][0]["name"] =
            json!(format!("qinpanel-v0.2.0-{arch}-unknown-linux-gnu.tar.gz"));
        assert_eq!(
            release_from_metadata(&value, arch).unwrap().digest,
            digest()
        );
    }
}
