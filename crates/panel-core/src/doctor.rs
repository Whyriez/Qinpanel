//! System Doctor: Comprehensive health checks, diagnostics, and auto-remediation.

use serde::{Deserialize, Serialize};

/// Categories of doctor checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum DoctorCheckCategory {
    OsKernel,
    Prerequisites,
    Services,
    Socket,
    Database,
    Filesystem,
    Firewall,
    Jobs,
}

impl DoctorCheckCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OsKernel => "os_kernel",
            Self::Prerequisites => "prerequisites",
            Self::Services => "services",
            Self::Socket => "socket",
            Self::Database => "database",
            Self::Filesystem => "filesystem",
            Self::Firewall => "firewall",
            Self::Jobs => "jobs",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::OsKernel => "Sistem Operasi & Kernel",
            Self::Prerequisites => "Biner & Prasyarat Sistem",
            Self::Services => "Layanan & Daemon (Systemd)",
            Self::Socket => "Soket IPC Agent",
            Self::Database => "Basis Data Panel (SQLite)",
            Self::Filesystem => "Sistem Berkas & Ruang Disk",
            Self::Firewall => "Firewall & Isolasi nftables",
            Self::Jobs => "Antrean & Integritas Pekerjaan",
        }
    }
}

/// Status of an individual doctor check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum DoctorCheckStatus {
    Pass,
    Warn,
    Fail,
}

impl DoctorCheckStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Warn => "warn",
            Self::Fail => "fail",
        }
    }
}

/// An individual doctor diagnosis check item.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DoctorCheckItem {
    pub id: String,
    pub name: String,
    pub category: DoctorCheckCategory,
    pub status: DoctorCheckStatus,
    pub message: String,
    pub remediation_hint: Option<String>,
    pub fixable: bool,
}

/// Comprehensive report produced by the doctor diagnosis.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DoctorReport {
    pub timestamp: u64,
    pub checks: Vec<DoctorCheckItem>,
    pub total_passed: usize,
    pub total_warnings: usize,
    pub total_failures: usize,
    pub overall_status: DoctorCheckStatus,
    pub healthy: bool,
}

impl DoctorReport {
    /// Constructs a report from a list of check items.
    pub fn new(timestamp: u64, checks: Vec<DoctorCheckItem>) -> Self {
        let mut total_passed = 0;
        let mut total_warnings = 0;
        let mut total_failures = 0;

        for check in &checks {
            match check.status {
                DoctorCheckStatus::Pass => total_passed += 1,
                DoctorCheckStatus::Warn => total_warnings += 1,
                DoctorCheckStatus::Fail => total_failures += 1,
            }
        }

        let overall_status = if total_failures > 0 {
            DoctorCheckStatus::Fail
        } else if total_warnings > 0 {
            DoctorCheckStatus::Warn
        } else {
            DoctorCheckStatus::Pass
        };

        let healthy = total_failures == 0;

        Self {
            timestamp,
            checks,
            total_passed,
            total_warnings,
            total_failures,
            overall_status,
            healthy,
        }
    }

    /// Returns list of check IDs that can be repaired automatically.
    pub fn fixable_items(&self) -> Vec<&DoctorCheckItem> {
        self.checks
            .iter()
            .filter(|c| c.fixable && c.status != DoctorCheckStatus::Pass)
            .collect()
    }
}

/// Result of an automated fix action.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DoctorRemediationResult {
    pub item_id: String,
    pub success: bool,
    pub message: String,
}

/// Format the doctor diagnosis report for CLI terminal display.
pub fn format_doctor_report_cli(report: &DoctorReport, use_color: bool) -> String {
    let mut out = String::new();
    out.push_str(
        "================================================================================\n",
    );
    out.push_str(
        "                        QONPANEL SYSTEM DOCTOR                                \n",
    );
    out.push_str(
        "================================================================================\n\n",
    );

    let mut current_category = None;
    for check in &report.checks {
        if current_category != Some(check.category) {
            current_category = Some(check.category);
            out.push_str(&format!("\n[ {} ]\n", check.category.display_name()));
        }

        let badge = match check.status {
            DoctorCheckStatus::Pass => {
                if use_color {
                    "\x1b[32m[PASS]\x1b[0m"
                } else {
                    "[PASS]"
                }
            }
            DoctorCheckStatus::Warn => {
                if use_color {
                    "\x1b[33m[WARN]\x1b[0m"
                } else {
                    "[WARN]"
                }
            }
            DoctorCheckStatus::Fail => {
                if use_color {
                    "\x1b[31m[FAIL]\x1b[0m"
                } else {
                    "[FAIL]"
                }
            }
        };

        out.push_str(&format!(
            "  {} {:<35} {}\n",
            badge, check.name, check.message
        ));
        if let (Some(hint), true) = (
            &check.remediation_hint,
            check.status != DoctorCheckStatus::Pass,
        ) {
            let hint_prefix = if use_color {
                "      \x1b[36mPetunjuk perbaikan:\x1b[0m "
            } else {
                "      Petunjuk perbaikan: "
            };
            out.push_str(&format!("{}{}\n", hint_prefix, hint));
        }
    }

    out.push_str(
        "\n--------------------------------------------------------------------------------\n",
    );
    let summary_line = format!(
        "Ringkasan: {} lolos, {} peringatan, {} gagal (Status: {})\n",
        report.total_passed,
        report.total_warnings,
        report.total_failures,
        report.overall_status.as_str().to_uppercase()
    );
    out.push_str(&summary_line);
    out.push_str(
        "================================================================================\n",
    );

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_doctor_report_aggregation() {
        let items = vec![
            DoctorCheckItem {
                id: "cgroup_v2".to_string(),
                name: "cgroup v2 support".to_string(),
                category: DoctorCheckCategory::OsKernel,
                status: DoctorCheckStatus::Pass,
                message: "Semua controller aktif".to_string(),
                remediation_hint: None,
                fixable: false,
            },
            DoctorCheckItem {
                id: "disk_free".to_string(),
                name: "Ruang disk bebas".to_string(),
                category: DoctorCheckCategory::Filesystem,
                status: DoctorCheckStatus::Warn,
                message: "Ruang disk bebas 8% (< 10%)".to_string(),
                remediation_hint: Some("Bersihkan direktori log atau cache".to_string()),
                fixable: false,
            },
            DoctorCheckItem {
                id: "stuck_jobs".to_string(),
                name: "Pekerjaan macet".to_string(),
                category: DoctorCheckCategory::Jobs,
                status: DoctorCheckStatus::Fail,
                message: "2 pekerjaan dalam status running tanpa worker".to_string(),
                remediation_hint: Some("Jalankan panelctl doctor --fix".to_string()),
                fixable: true,
            },
        ];

        let report = DoctorReport::new(1720000000, items);
        assert_eq!(report.total_passed, 1);
        assert_eq!(report.total_warnings, 1);
        assert_eq!(report.total_failures, 1);
        assert_eq!(report.overall_status, DoctorCheckStatus::Fail);
        assert!(!report.healthy);

        let fixable = report.fixable_items();
        assert_eq!(fixable.len(), 1);
        assert_eq!(fixable[0].id, "stuck_jobs");

        let cli_output = format_doctor_report_cli(&report, false);
        assert!(cli_output.contains("QONPANEL SYSTEM DOCTOR"));
        assert!(cli_output.contains("[FAIL]"));
        assert!(cli_output.contains("[WARN]"));
        assert!(cli_output.contains("[PASS]"));
    }
}
