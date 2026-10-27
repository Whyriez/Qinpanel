//! `panel-cli` library for administrative operations and `panelctl` CLI tool.

pub mod commands;
pub mod error;

pub use commands::admin::{
    BootstrapCredentials, bootstrap_admin, create_admin, generate_admin_password, list_admins,
    reset_2fa, reset_password, validate_admin_username,
};
pub use commands::benchmark::{
    BenchmarkComparison, BenchmarkOptions, BenchmarkRunResult, BenchmarkTarget, BenchmarkTool,
    LatencyDistribution, MeasuredSystemMetrics, NonFunctionalTargetsReport, TargetMetricStatus,
    TuningProfile, detect_available_tools, export_results_to_json, generate_markdown_report,
    run_builtin_benchmark, validate_non_functional_targets,
};
pub use commands::doctor::{DoctorCliOptions, run_doctor};
pub use commands::memory_profile::{MemoryProfileOptions, run_memory_profile};
pub use commands::reconcile::{ReconcileCliOptions, run_reconcile};
pub use commands::setup_token::{
    SETTING_SETUP_TOKEN, SETTING_SETUP_TOKEN_CREATED_AT, SetupStatus, generate_setup_token,
    get_setup_status, verify_setup_token,
};
pub use commands::system_backup::{
    SystemBackupCommands, create_system_backup, handle_system_backup_command, list_system_backups,
    restore_system_backup, verify_system_backup,
};
pub use commands::uninstall::{UninstallCliOptions, UninstallResult, run_uninstall};
pub use commands::upgrade::{UpgradeCliOptions, UpgradeResult, run_upgrade};
pub use error::CliError;
