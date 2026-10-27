//! Typed email operations. No shell, free paths, or arbitrary mailbox queries.
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(transparent)]
pub struct EmailSecret(pub String);
impl std::fmt::Debug for EmailSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EmailServerConfig {
    pub hostname: String,
    pub certificate_id: i64,
    #[serde(default)]
    pub certificate_domain: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EmailMailbox {
    pub id: String,
    pub address: String,
    pub linux_username: String,
    pub uid: u32,
    pub password_hash: EmailSecret,
    pub quota_mb: u32,
    pub enabled: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum EmailFolder {
    Inbox,
    Sent,
    Trash,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum EmailRequest {
    Status,
    SecretSeal {
        value: EmailSecret,
    },
    SecretOpen {
        value: EmailSecret,
    },
    MailboxApply {
        mailbox: EmailMailbox,
    },
    MailboxRemove {
        id: String,
    },
    DnsRecords {
        domain: String,
    },
    Messages {
        id: String,
        folder: EmailFolder,
        before: Option<u32>,
    },
    Message {
        id: String,
        folder: EmailFolder,
        uid: u32,
        uid_validity: u32,
    },
    MessageTrash {
        id: String,
        folder: EmailFolder,
        uid: u32,
        uid_validity: u32,
    },
    Send {
        id: String,
        to: String,
        subject: String,
        body: String,
    },
    Service {
        operation: EmailServiceAction,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum EmailServiceAction {
    Start,
    Stop,
    Restart,
}
