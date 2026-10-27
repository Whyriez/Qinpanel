use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::fmt;

/// Redirection policy for a domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RedirectMode {
    None,
    ToPrimary,
    ToWww,
    ToNonWww,
    Custom,
}

impl RedirectMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ToPrimary => "to_primary",
            Self::ToWww => "to_www",
            Self::ToNonWww => "to_non_www",
            Self::Custom => "custom",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "none" => Some(Self::None),
            "to_primary" | "toprimary" => Some(Self::ToPrimary),
            "to_www" | "towww" => Some(Self::ToWww),
            "to_non_www" | "tononwww" => Some(Self::ToNonWww),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }
}

impl fmt::Display for RedirectMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Verification / operational status of a domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DomainStatus {
    Active,
    PendingSsl,
    Disabled,
}

impl DomainStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::PendingSsl => "pending_ssl",
            Self::Disabled => "disabled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "active" => Some(Self::Active),
            "pending_ssl" | "pendingssl" => Some(Self::PendingSsl),
            "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }
}

impl fmt::Display for DomainStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Domain database entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct Domain {
    pub id: i64,
    pub site_id: i64,
    pub name: String,
    pub unicode_name: String,
    pub is_primary: bool,
    pub redirect_mode: String,
    pub redirect_target: Option<String>,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Domain {
    pub fn parsed_redirect_mode(&self) -> Option<RedirectMode> {
        RedirectMode::parse(&self.redirect_mode)
    }

    pub fn parsed_status(&self) -> Option<DomainStatus> {
        DomainStatus::parse(&self.status)
    }
}

/// Parameters for adding a new domain to a site.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDomain {
    pub site_id: i64,
    pub name: String,
    pub unicode_name: String,
    pub is_primary: bool,
    pub redirect_mode: Option<String>,
    pub redirect_target: Option<String>,
    pub status: Option<String>,
}

/// Parameters for updating domain settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateDomain {
    pub name: Option<String>,
    pub unicode_name: Option<String>,
    pub is_primary: Option<bool>,
    pub redirect_mode: Option<String>,
    pub redirect_target: Option<String>,
    pub status: Option<String>,
}
