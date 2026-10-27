use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::fmt;

/// The execution runtime kind for a website.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SiteKind {
    Static,
    Php,
    Nodejs,
    Python,
    Proxy,
}

impl SiteKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::Php => "php",
            Self::Nodejs => "nodejs",
            Self::Python => "python",
            Self::Proxy => "proxy",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "static" => Some(Self::Static),
            "php" => Some(Self::Php),
            "nodejs" | "node" => Some(Self::Nodejs),
            "python" | "py" => Some(Self::Python),
            "proxy" => Some(Self::Proxy),
            _ => None,
        }
    }
}

impl fmt::Display for SiteKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Operational status of a site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SiteStatus {
    Active,
    Disabled,
    Suspended,
}

impl SiteStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled",
            Self::Suspended => "suspended",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "active" => Some(Self::Active),
            "disabled" => Some(Self::Disabled),
            "suspended" => Some(Self::Suspended),
            _ => None,
        }
    }
}

impl fmt::Display for SiteStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Site entity in database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct Site {
    pub id: i64,
    pub user_id: i64,
    pub kind: String,
    pub docroot: String,
    pub runtime: Option<String>,
    pub version: Option<String>,
    pub status: String,
    pub settings_json: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Site {
    pub fn parsed_kind(&self) -> Option<SiteKind> {
        SiteKind::parse(&self.kind)
    }

    pub fn parsed_status(&self) -> Option<SiteStatus> {
        SiteStatus::parse(&self.status)
    }
}

/// Parameters for creating a new website.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSite {
    pub user_id: i64,
    pub kind: String,
    pub docroot: String,
    pub runtime: Option<String>,
    pub version: Option<String>,
    pub status: Option<String>,
    pub settings_json: Option<String>,
}

/// Parameters for updating an existing website.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateSite {
    pub kind: Option<String>,
    pub docroot: Option<String>,
    pub runtime: Option<String>,
    pub version: Option<String>,
    pub status: Option<String>,
    pub settings_json: Option<String>,
}
