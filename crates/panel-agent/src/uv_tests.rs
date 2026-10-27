#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::system::{MockAction, MockSystemRunner};

fn archive(name: &str, link: bool) -> Vec<u8> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    let content = b"fixture executable; never executed";
    let mut header = tar::Header::new_gnu();
    header.set_mode(0o755);
    if link {
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_link_name("/etc/passwd").unwrap();
        header.set_size(0);
        header.set_cksum();
        builder
            .append_data(&mut header, format!("{name}/uv"), &[][..])
            .unwrap();
    } else {
        header.set_size(content.len() as u64);
        header.set_cksum();
        builder
            .append_data(&mut header, format!("{name}/uv"), &content[..])
            .unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap()
}

#[test]
fn existing_global_uv_is_checked_without_downloads() {
    let runner = MockSystemRunner::new();
    runner.add_existing_path(UV_BIN);
    assert_eq!(ensure(&runner, &CancellationToken::new()).unwrap(), UV_BIN);
    assert_eq!(runner.get_commands(), [format!("{UV_BIN} --version")]);
}

#[test]
fn os_uv_is_published_at_a_global_path_without_downloads() {
    let runner = MockSystemRunner::new();
    runner.add_existing_path("/usr/bin/uv");
    assert_eq!(ensure(&runner, &CancellationToken::new()).unwrap(), UV_BIN);
    assert!(
        runner
            .recorded_actions()
            .iter()
            .any(|action| matches!(action, MockAction::CreateSymlink { .. }))
    );
    assert!(
        runner
            .get_commands()
            .iter()
            .all(|command| !command.starts_with("curl"))
    );
}

#[test]
fn missing_uv_bootstraps_verified_binary_before_python_install() {
    let runner = MockSystemRunner::new();
    let bytes = archive("uv-fixture", false);
    let checksum = panel_core::compute_bytes_sha256(&bytes);
    let downloaded = Path::new(DOWNLOAD_DIR).join("uv.tar.gz");
    runner.set_file_content(&downloaded, &bytes);
    bootstrap(&runner, "uv-fixture", &checksum, &CancellationToken::new()).unwrap();
    assert_eq!(
        runner.read_file(Path::new(UV_BIN)).unwrap(),
        b"fixture executable; never executed"
    );
    assert!(!runner.path_exists(&downloaded));
    let result = crate::python_site::install_python_runtime(
        &runner,
        &panel_proto::PythonRuntimeInstallRequest {
            version: "3.15.0".into(),
            base_dir: None,
        },
    )
    .unwrap();
    assert!(result.success);
    let commands = runner.get_commands();
    assert_eq!(
        commands
            .iter()
            .filter(|command| command.starts_with("curl"))
            .count(),
        1
    );
    assert!(
        commands
            .iter()
            .any(|command| command.starts_with(&format!("{UV_BIN} python install 3.15.0")))
    );
    assert!(
        commands
            .iter()
            .all(|command| !command.starts_with("sh ") && !command.starts_with("bash "))
    );
}

#[test]
fn checksum_and_symlink_failures_never_publish_a_binary() {
    for link in [false, true] {
        let runner = MockSystemRunner::new();
        let bytes = archive("uv-fixture", link);
        let checksum = if link {
            panel_core::compute_bytes_sha256(&bytes)
        } else {
            "0".repeat(64)
        };
        assert!(install_verified(&runner, &bytes, "uv-fixture", &checksum).is_err());
        assert!(!runner.path_exists(Path::new(UV_BIN)));
        assert!(
            runner
                .recorded_actions()
                .iter()
                .all(|action| !matches!(action, MockAction::WriteFile { .. }))
        );
    }
}

#[test]
fn failed_download_is_contextual_and_does_not_publish_or_run_python() {
    let runner = MockSystemRunner::new();
    runner.set_response("curl", 22, b"", b"fixture outage");
    let error = ensure(&runner, &CancellationToken::new())
        .unwrap_err()
        .to_string();
    assert!(error.contains("uv"));
    assert!(!runner.path_exists(Path::new(UV_BIN)));
    assert!(
        runner
            .get_commands()
            .iter()
            .all(|command| !command.contains("python install"))
    );
}

#[test]
fn invalid_python_requests_do_not_bootstrap_tools() {
    let runner = MockSystemRunner::new();
    for request in [
        panel_proto::PythonRuntimeInstallRequest {
            version: "../etc".into(),
            base_dir: None,
        },
        panel_proto::PythonRuntimeInstallRequest {
            version: "3.15.0".into(),
            base_dir: Some("/tmp/custom".into()),
        },
    ] {
        assert!(crate::python_site::install_python_runtime(&runner, &request).is_err());
    }
    assert!(runner.recorded_actions().is_empty());
    assert!(artifact("unsupported").is_err());
}
