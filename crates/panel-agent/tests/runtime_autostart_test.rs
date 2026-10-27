//! Local command-runner regressions; never starts an agent or system service.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use panel_agent::node_site::apply_node_site;
use panel_agent::python_site::apply_python_site;
use panel_agent::system::MockSystemRunner;
use panel_proto::{
    NodeSiteApplyRequest, NodeSiteEntrypoint, PythonSiteApplyRequest, PythonSiteEntrypoint,
    PythonSiteFramework, PythonSiteServer,
};
use std::collections::BTreeMap;

#[test]
fn node_enable_failure_cannot_report_success_or_start_and_can_be_retried() {
    let runner = MockSystemRunner::new();
    runner.add_existing_path("/opt/panel/runtimes/node/22.14.0/bin/node");
    runner.add_existing_path("/home/p_alice/sites/app/server.js");
    let request = NodeSiteApplyRequest {
        site_id: "site_1".into(),
        username: "p_alice".into(),
        uid: 10001,
        node_version: "22.14.0".into(),
        app_dir: "/home/p_alice/sites/app".into(),
        port: 20042,
        entrypoint: NodeSiteEntrypoint::NodeScript("server.js".into()),
        env_vars: BTreeMap::new(),
        auto_restart: false,
        env_file_path: None,
        unit_path: None,
        base_runtime_dir: None,
    };
    runner.set_response(
        "systemctl enable site-site_1.service",
        1,
        b"",
        b"enable denied",
    );
    let error = apply_node_site(&runner, &request).unwrap_err();
    assert!(error.to_string().contains("enable denied"));
    assert!(
        !runner
            .get_commands()
            .iter()
            .any(|c| c.starts_with("systemctl restart"))
    );

    runner.set_response("systemctl enable site-site_1.service", 0, b"", b"");
    assert!(apply_node_site(&runner, &request).unwrap().active);
    assert_enabled_before_restart(&runner);
}

#[test]
fn python_enable_failure_cannot_report_success_or_start_and_can_be_retried() {
    let runner = MockSystemRunner::new();
    runner.add_existing_path("/opt/panel/runtimes/python/3.12.9/bin/python3");
    runner.add_existing_path("/home/p_alice/sites/app/app.py");
    let request = PythonSiteApplyRequest {
        site_id: "site_1".into(),
        username: "p_alice".into(),
        uid: 10001,
        python_version: "3.12.9".into(),
        app_dir: "/home/p_alice/sites/app".into(),
        venv_dir: "/home/p_alice/sites/app/.venv".into(),
        socket_path: "/run/panel/python/site_1/app.sock".into(),
        framework: PythonSiteFramework::Generic,
        server_type: PythonSiteServer::GunicornWsgi,
        entrypoint: PythonSiteEntrypoint::Wsgi("app:app".into()),
        workers: 1,
        env_vars: BTreeMap::new(),
        auto_restart: false,
        env_file_path: None,
        unit_path: None,
        base_runtime_dir: None,
    };
    runner.set_response(
        "systemctl enable site-site_1.service",
        1,
        b"",
        b"enable denied",
    );
    let error = apply_python_site(&runner, &request).unwrap_err();
    assert!(error.to_string().contains("enable denied"));
    assert!(
        !runner
            .get_commands()
            .iter()
            .any(|c| c.starts_with("systemctl restart"))
    );

    runner.set_response("systemctl enable site-site_1.service", 0, b"", b"");
    assert!(apply_python_site(&runner, &request).unwrap().active);
    assert_enabled_before_restart(&runner);
}

fn assert_enabled_before_restart(runner: &MockSystemRunner) {
    let commands = runner.get_commands();
    let enable = commands
        .iter()
        .rposition(|c| c == "systemctl enable site-site_1.service")
        .unwrap();
    let restart = commands
        .iter()
        .rposition(|c| c == "systemctl restart site-site_1.service")
        .unwrap();
    assert!(enable < restart);
}
