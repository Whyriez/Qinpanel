//! System State Reconciler: Compares desired state (database) with actual host state and reconciles drift.

use serde::{Deserialize, Serialize};

/// Type of state drift detected between database and actual system state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum DriftType {
    MissingNginxConfig,
    OrphanedNginxConfig,
    MissingDocrootDirectory,
    DirectoryPermissionMismatch,
    NftablesBindingMismatch,
    InterruptedJobStuck,
}

impl DriftType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MissingNginxConfig => "missing_nginx_config",
            Self::OrphanedNginxConfig => "orphaned_nginx_config",
            Self::MissingDocrootDirectory => "missing_docroot_directory",
            Self::DirectoryPermissionMismatch => "directory_permission_mismatch",
            Self::NftablesBindingMismatch => "nftables_binding_mismatch",
            Self::InterruptedJobStuck => "interrupted_job_stuck",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::MissingNginxConfig => "Konfigurasi Nginx Hilang",
            Self::OrphanedNginxConfig => "Konfigurasi Nginx Yatim (Tidak Terdaftar)",
            Self::MissingDocrootDirectory => "Direktori Docroot Hilang",
            Self::DirectoryPermissionMismatch => "Izin Kepemilikan Direktori Tidak Sesuai",
            Self::NftablesBindingMismatch => "Binding Port nftables Tidak Sesuai",
            Self::InterruptedJobStuck => "Pekerjaan Macet / Terinterupsi",
        }
    }
}

/// An individual drift item between database state and actual system state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DriftItem {
    pub id: String,
    pub drift_type: DriftType,
    pub resource_type: String,
    pub resource_id: String,
    pub description: String,
    pub desired_state: String,
    pub actual_state: String,
    pub remediation_action: String,
}

/// Comprehensive report of state reconciliation analysis.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReconcileReport {
    pub timestamp: u64,
    pub total_resources_checked: usize,
    pub drifts: Vec<DriftItem>,
    pub is_clean: bool,
}

impl ReconcileReport {
    pub fn new(timestamp: u64, total_resources_checked: usize, drifts: Vec<DriftItem>) -> Self {
        let is_clean = drifts.is_empty();
        Self {
            timestamp,
            total_resources_checked,
            drifts,
            is_clean,
        }
    }
}

/// Execution outcome of applying reconciliation fixes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReconcileExecutionResult {
    pub timestamp: u64,
    pub reconciled_items: Vec<String>,
    pub failed_items: Vec<String>,
    pub success: bool,
    pub message: String,
}

/// Format reconcile report for CLI display.
pub fn format_reconcile_report_cli(report: &ReconcileReport, use_color: bool) -> String {
    let mut out = String::new();
    out.push_str(
        "================================================================================\n",
    );
    out.push_str(
        "                     QONPANEL STATE RECONCILER                                \n",
    );
    out.push_str(
        "================================================================================\n\n",
    );

    out.push_str(&format!(
        "Total sumber daya diperiksa: {}\n",
        report.total_resources_checked
    ));

    if report.is_clean {
        let clean_msg = if use_color {
            "\x1b[32m[SEMPURNA] Tidak ada penyimpangan (drift) yang ditemukan. Kondisi sistem sinkron 100%.\x1b[0m\n"
        } else {
            "[SEMPURNA] Tidak ada penyimpangan (drift) yang ditemukan. Kondisi sistem sinkron 100%.\n"
        };
        out.push_str(clean_msg);
    } else {
        out.push_str(&format!(
            "Ditemukan {} penyimpangan state (drift):\n\n",
            report.drifts.len()
        ));

        for (idx, drift) in report.drifts.iter().enumerate() {
            let warn_badge = if use_color {
                "\x1b[33m[DRIFT]\x1b[0m"
            } else {
                "[DRIFT]"
            };

            out.push_str(&format!(
                "{}. {} [{}] {} ({})\n",
                idx + 1,
                warn_badge,
                drift.drift_type.display_name(),
                drift.resource_id,
                drift.resource_type
            ));
            out.push_str(&format!("   Keterangan   : {}\n", drift.description));
            out.push_str(&format!("   Target (DB)  : {}\n", drift.desired_state));
            out.push_str(&format!("   Kondisi Host : {}\n", drift.actual_state));
            out.push_str(&format!(
                "   Tindakan     : {}\n\n",
                drift.remediation_action
            ));
        }
    }

    out.push_str(
        "--------------------------------------------------------------------------------\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reconcile_report_clean() {
        let report = ReconcileReport::new(1720000000, 24, vec![]);
        assert!(report.is_clean);
        assert_eq!(report.drifts.len(), 0);

        let cli_out = format_reconcile_report_cli(&report, false);
        assert!(cli_out.contains("[SEMPURNA]"));
    }

    #[test]
    fn test_reconcile_report_with_drifts() {
        let drifts = vec![
            DriftItem {
                id: "site_1_vhost".to_string(),
                drift_type: DriftType::MissingNginxConfig,
                resource_type: "nginx_vhost".to_string(),
                resource_id: "example.com".to_string(),
                description: "Berkas vhost Nginx tidak ditemukan di sites-available".to_string(),
                desired_state: "File /etc/panel/nginx/sites-available/1.conf aktif".to_string(),
                actual_state: "File tidak ada".to_string(),
                remediation_action: "Render ulang dan apply atomic vhost Nginx".to_string(),
            },
            DriftItem {
                id: "stuck_job_99".to_string(),
                drift_type: DriftType::InterruptedJobStuck,
                resource_type: "job".to_string(),
                resource_id: "job_99".to_string(),
                description: "Pekerjaan deployment terhenti saat restart".to_string(),
                desired_state: "Pekerjaan diselesaikan atau ditandai gagal".to_string(),
                actual_state: "Status 'running' tanpa proses aktif".to_string(),
                remediation_action: "Tandai gagal dan catat log pemulihan".to_string(),
            },
        ];

        let report = ReconcileReport::new(1720000000, 15, drifts);
        assert!(!report.is_clean);
        assert_eq!(report.drifts.len(), 2);

        let cli_out = format_reconcile_report_cli(&report, false);
        assert!(cli_out.contains("[DRIFT]"));
        assert!(cli_out.contains("Konfigurasi Nginx Hilang"));
        assert!(cli_out.contains("example.com"));
    }
}
