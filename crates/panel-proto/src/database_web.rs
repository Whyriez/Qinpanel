//! Internal database administration. No client-controlled URL, command or filesystem path.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DatabaseWebStatus {
    pub installed: bool,
    pub ready: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DatabaseWebCredentialsRequest {
    pub engine: String,
    pub name: String,
    pub db_user: String,
    pub tenant_username: String,
    pub tenant_uid: u32,
    /// Only a CSRF-validated launch may provision the dedicated local account.
    pub provision: bool,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct DatabaseWebCredentials {
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for DatabaseWebCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DatabaseWebCredentials")
            .field("username", &self.username)
            .field("password", &"[redacted]")
            .finish()
    }
}
