//! CI checks for version-controlled release artifacts and performance evaluation.
//!
//! The v1.0 documentation audits need local, ignored release evidence. Run them
//! explicitly with `cargo test -p panel-cli --test release_checklist_test -- --ignored`.

use std::fs;
use std::path::PathBuf;

#[test]
fn test_release_artifacts_presence() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root");

    let required_files = [
        "install.sh",
        "uninstall.sh",
        "scripts/bootstrap-install.sh",
        "scripts/dogfood.sh",
        "scripts/soak-test.sh",
        "scripts/build-release.sh",
        "scripts/compile-release.sh",
        "scripts/check-glibc.sh",
        "templates/systemd/panel-agent.service",
        "templates/systemd/panel-api.service",
        "templates/nginx/nusa-panel.conf",
        ".github/workflows/release.yml",
    ];

    for file_rel in required_files {
        let full_path = workspace_root.join(file_rel);
        assert!(
            full_path.exists(),
            "Required release artifact must exist: {}",
            file_rel
        );

        let content = fs::read_to_string(&full_path)
            .unwrap_or_else(|_| panic!("Failed to read {}", file_rel));
        assert!(
            !content.trim().is_empty(),
            "File must not be empty: {}",
            file_rel
        );
    }
}

#[test]
#[ignore = "manual v1.0 release audit; docs/ is intentionally excluded from Git"]
fn test_release_v1_documentation_presence() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root");

    let required_docs = [
        "docs/release-v1.0-checklist.md",
        "docs/admin-guide.md",
        "docs/tenant-guide.md",
        "docs/architecture.md",
        "docs/security-model.md",
        "docs/api-reference.md",
        "docs/openapi.json",
        "docs/contributing.md",
        "docs/benchmarks/dogfood-soak-report.md",
    ];

    for file_rel in required_docs {
        let content = fs::read_to_string(workspace_root.join(file_rel))
            .unwrap_or_else(|err| panic!("Required v1.0 release document {file_rel}: {err}"));
        assert!(
            !content.trim().is_empty(),
            "Release document must not be empty: {file_rel}"
        );
    }
}

#[test]
#[ignore = "manual v1.0 sign-off audit; requires local docs/release-v1.0-checklist.md"]
fn test_release_checklist_contains_all_signed_off_items() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root");

    let checklist_path = workspace_root.join("docs/release-v1.0-checklist.md");
    let content = fs::read_to_string(checklist_path).expect("read release checklist");

    // Must contain all 6 category headers
    assert!(content.contains("1. Fungsionalitas Inti & Multi-Runtime"));
    assert!(content.contains("2. Keamanan & Kepatuhan Standar"));
    assert!(content.contains("3. Keandalan Sistem & Pengujian Ketahanan"));
    assert!(content.contains("4. Validasi Target Non-Fungsional"));
    assert!(content.contains("5. Pengemasan & Multi-Arsitektur"));
    assert!(content.contains("6. Kelengkapan Dokumentasi & Lisensi"));

    // Every checklist item must be checked off ([x])
    let total_unchecked = content.matches("- [ ]").count();
    assert_eq!(
        total_unchecked, 0,
        "All release checklist items must be signed off, found {} unchecked items",
        total_unchecked
    );

    let total_checked = content.matches("- [x]").count();
    assert!(
        total_checked >= 25,
        "Must have at least 25 signed-off release criteria, found {}",
        total_checked
    );
}

#[test]
fn test_prd_section_9_non_functional_targets_evaluation() {
    use panel_core::memory_profile::evaluate_prd_memory_targets;

    // Fixture values exercise the evaluator, not measurements of this release.
    let evaluated = evaluate_prd_memory_targets(
        Some(18.5),   // api_rss_mb <= 40 MB
        Some(12.2),   // agent_rss_mb <= 20 MB
        Some(61.8),   // total_rss_mb <= 150 MB
        Some(280),    // cold_start_ms < 1000 ms
        Some(2.4),    // api_latency_p95_ms < 50 ms
        Some(145.47), // web_bundle_gzip_kb <= 250 KB
    );

    assert_eq!(
        evaluated.len(),
        6,
        "Must evaluate all 6 PRD Section 9 memory & performance targets"
    );

    // All targets must be marked as passed
    for eval in evaluated {
        assert!(
            eval.passed,
            "Target '{}' failed: measured {} against limit {}",
            eval.name, eval.measured, eval.threshold
        );
    }
}
