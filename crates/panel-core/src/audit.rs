//! Read-time audit categories keep historical action identifiers intact.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum AuditCategory {
    Authentication,
    Users,
    Websites,
    Files,
    Databases,
    Runtimes,
    Access,
    Network,
    Email,
    Backups,
    System,
    Other,
}

impl AuditCategory {
    pub const ALL: [Self; 12] = [
        Self::Authentication,
        Self::Users,
        Self::Websites,
        Self::Files,
        Self::Databases,
        Self::Runtimes,
        Self::Access,
        Self::Network,
        Self::Email,
        Self::Backups,
        Self::System,
        Self::Other,
    ];

    /// Literal, case-sensitive prefixes including the namespace separator.
    pub const fn prefixes(self) -> &'static [&'static str] {
        match self {
            Self::Authentication => &["auth.", "session.", "token.", "api_token."],
            Self::Users => &["tenant.", "user.", "plan."],
            Self::Websites => &["site.", "domain.", "ssl.", "dns01."],
            Self::Files => &["file.", "files."],
            Self::Databases => &["database."],
            Self::Runtimes => &["php.", "node.", "python.", "runtime."],
            Self::Access => &["terminal.", "ssh.", "ssh_key."],
            Self::Network => &["network.", "cloudflare.", "connectivity.", "panel.tls."],
            Self::Email => &["email.", "mail."],
            Self::Backups => &["backup.", "server.backup."],
            Self::System => &[
                "system.",
                "setting.",
                "settings.",
                "job.",
                "optimizer.",
                "analytics.",
                "panel.update.",
                "notification.",
            ],
            Self::Other => &[],
        }
    }

    pub fn for_action(action: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|category| {
                category
                    .prefixes()
                    .iter()
                    .any(|prefix| action.starts_with(prefix))
            })
            .unwrap_or(Self::Other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_cover_namespaces_without_overlap() {
        for category in AuditCategory::ALL {
            for prefix in category.prefixes() {
                let action = format!("{prefix}existing_action");
                assert_eq!(AuditCategory::for_action(&action), category);
                assert_eq!(
                    AuditCategory::ALL
                        .into_iter()
                        .filter(|other| { other.prefixes().iter().any(|p| action.starts_with(p)) })
                        .count(),
                    1
                );
            }
        }
    }

    #[test]
    fn preserves_specific_site_namespaces_and_unknown_actions() {
        for action in [
            "site.php.settings_update",
            "site.ssl.issue",
            "ssl.dns01.issue",
        ] {
            assert_eq!(AuditCategory::for_action(action), AuditCategory::Websites);
        }
        assert_eq!(
            AuditCategory::for_action("panel.tls.issue"),
            AuditCategory::Network
        );
        assert_eq!(
            AuditCategory::for_action("server.backup.job"),
            AuditCategory::Backups
        );
        for action in [
            "",
            "authentic.login",
            "AUTH.login",
            "sshXkey.create",
            "future.action",
        ] {
            assert_eq!(AuditCategory::for_action(action), AuditCategory::Other);
        }
        assert!(serde_json::from_str::<AuditCategory>("\"unknown\"").is_err());
    }
}
