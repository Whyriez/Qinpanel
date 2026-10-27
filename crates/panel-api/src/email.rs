//! Email accounts, plain-text inbox and custom notification templates.
use crate::{
    agent_client::AgentClient,
    auth::{AuthenticatedUser, RequireAdmin, RequireAdminCsrf, RequireAuth, RequireCsrf},
    jobs::{
        handlers::JobItem,
        registry::{JobContext, JobFuture, JobHandler},
    },
    mail::{EmailMessage, SmtpClient, SmtpConfig},
    routes::AppState,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use panel_db::{pool::DbPool, repositories::SqliteNotificationRepository};
use panel_proto::{EmailFolder, EmailMailbox, EmailRequest, EmailSecret, EmailServerConfig};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, Row};
use std::sync::Arc;
use uuid::Uuid;
type Error = (StatusCode, Json<Value>);
fn fail(status: StatusCode, text: impl Into<String>) -> Error {
    (status, Json(json!({"error":text.into()})))
}
fn db_error(_: impl std::fmt::Display) -> Error {
    fail(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Gagal membaca atau menyimpan konfigurasi email",
    )
}
fn agent_error(error: crate::agent_client::AgentClientError) -> Error {
    let message = match error {
        crate::agent_client::AgentClientError::AgentError(value) if value.len() <= 600 => value,
        _ => "Layanan email belum dapat dihubungi. Periksa agent dan status layanan.".into(),
    };
    fail(StatusCode::BAD_GATEWAY, message)
}
#[derive(FromRow)]
struct Mailbox {
    id: String,
    user_id: i64,
    domain_id: i64,
    address: String,
    password_hash: String,
    quota_mb: i64,
    enabled: bool,
    state: String,
    operation: String,
    pending_job_id: Option<String>,
    error: Option<String>,
}
impl Mailbox {
    fn public(&self) -> Value {
        json!({"id":self.id,"domain_id":self.domain_id,"address":self.address,"quota_mb":self.quota_mb,"enabled":self.enabled,"state":self.state,"operation":self.operation,"job_id":self.pending_job_id,"error":self.error})
    }
}
async fn mailbox(pool: &DbPool, user: &AuthenticatedUser, id: &str) -> Result<Mailbox, Error> {
    let row: Mailbox = sqlx::query_as("SELECT * FROM email_mailboxes WHERE id=?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(db_error)?
        .ok_or_else(|| fail(StatusCode::NOT_FOUND, "Mailbox tidak ditemukan"))?;
    if !user.can_access_user(row.user_id) {
        return Err(fail(StatusCode::NOT_FOUND, "Mailbox tidak ditemukan"));
    }
    Ok(row)
}
pub async fn secret(agent: &AgentClient, value: String, seal: bool) -> Result<String, String> {
    let value = EmailSecret(value);
    let result = agent
        .email(if seal {
            EmailRequest::SecretSeal { value }
        } else {
            EmailRequest::SecretOpen { value }
        })
        .await
        .map_err(|_| "Agent tidak dapat memproses secret SMTP".to_owned())?;
    result["value"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "Respons secret SMTP tidak sesuai".into())
}
pub async fn smtp_password(
    pool: &DbPool,
    agent: &AgentClient,
    value: Option<String>,
) -> Result<Option<String>, String> {
    let Some(value) = value.filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    let cipher = if value.starts_with("enc:v1:") {
        value
    } else {
        let cipher = secret(agent, value.clone(), true).await?;
        // Compare-and-swap prevents a legacy migration overwriting a new password.
        sqlx::query("UPDATE notification_settings SET smtp_password_enc=? WHERE id=1 AND smtp_password_enc=?").bind(&cipher).bind(value).execute(pool).await.map_err(|_|"Migrasi password SMTP gagal".to_owned())?;
        cipher
    };
    secret(agent, cipher, false).await.map(Some)
}
#[derive(Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EmailTemplate {
    pub panel_name: String,
    pub subject: String,
    pub body: String,
}
impl Default for EmailTemplate {
    fn default() -> Self {
        Self {
            panel_name: "Qinpanel".into(),
            subject: "[{panel_name}] {title}".into(),
            body: "{message}\n\nBuka panel: {action_url}\n\n{panel_name}".into(),
        }
    }
}
fn interpolate(input: &str, values: &[(&str, &str)]) -> Result<String, String> {
    let mut output = String::new();
    let mut tail = input;
    while let Some((head, rest)) = tail.split_once('{') {
        output.push_str(head);
        let (key, next) = rest
            .split_once('}')
            .ok_or_else(|| "Placeholder belum ditutup".to_string())?;
        let value = values
            .iter()
            .find(|(k, _)| *k == key)
            .ok_or_else(|| format!("Placeholder {{{key}}} tidak dikenal"))?
            .1;
        output.push_str(value);
        tail = next;
    }
    output.push_str(tail);
    Ok(output)
}
impl EmailTemplate {
    fn validate(&self) -> Result<(), String> {
        if self.panel_name.trim().is_empty()
            || self.panel_name.len() > 100
            || self.panel_name.chars().any(char::is_control)
            || self.subject.trim().is_empty()
            || self.subject.len() > 300
            || self.subject.chars().any(char::is_control)
            || self.body.trim().is_empty()
            || self.body.len() > 16000
        {
            return Err("Nama panel, subjek atau isi template tidak valid".into());
        }
        let values = [
            ("panel_name", ""),
            ("title", ""),
            ("message", ""),
            ("category", ""),
            ("action_url", ""),
        ];
        interpolate(&self.subject, &values)?;
        interpolate(&self.body, &values)?;
        Ok(())
    }
    pub fn render(
        &self,
        title: &str,
        message: &str,
        category: &str,
        url: &str,
    ) -> Result<(String, String), String> {
        self.validate()?;
        let clean_title = title.replace(['\r', '\n'], " ");
        let values = [
            ("panel_name", self.panel_name.as_str()),
            ("title", clean_title.as_str()),
            ("message", message),
            ("category", category),
            ("action_url", url),
        ];
        let subject = interpolate(&self.subject, &values)?;
        if subject.chars().any(char::is_control) || subject.len() > 512 {
            return Err("Subjek hasil template tidak valid".into());
        }
        Ok((subject, interpolate(&self.body, &values)?))
    }
}
pub async fn load_template(pool: &DbPool) -> Result<EmailTemplate, String> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key='email.template'")
            .fetch_optional(pool)
            .await
            .map_err(|_| "Gagal membaca template email")?;
    value
        .map(|v| serde_json::from_str(&v).map_err(|_| "Template email tidak valid".into()))
        .unwrap_or_else(|| Ok(EmailTemplate::default()))
}
#[utoipa::path(get,path="/api/v1/email/templates",responses((status=200,description="Email response"),(status=202,description="Persisted email job"),(status=403,description="Access denied")),tag="email")]
pub async fn templates(
    State(state): State<AppState>,
    RequireAdmin(_user): RequireAdmin,
) -> Result<Json<Value>, Error> {
    let t = load_template(&state.pool).await.map_err(db_error)?;
    Ok(Json(json!(t)))
}
#[utoipa::path(put,path="/api/v1/email/templates",request_body=EmailTemplate,responses((status=200,description="Email response"),(status=202,description="Persisted email job"),(status=403,description="Access denied")),tag="email")]
pub async fn save_template(
    State(state): State<AppState>,
    RequireAdminCsrf(_user): RequireAdminCsrf,
    Json(t): Json<EmailTemplate>,
) -> Result<Json<Value>, Error> {
    t.validate().map_err(|e| fail(StatusCode::BAD_REQUEST, e))?;
    sqlx::query("INSERT INTO settings(key,value,updated_at) VALUES('email.template',?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at").bind(serde_json::to_string(&t).map_err(db_error)?).bind(chrono::Utc::now().timestamp()).execute(&state.pool).await.map_err(db_error)?;
    Ok(Json(json!(t)))
}
#[utoipa::path(get,path="/api/v1/email",responses((status=200,description="Email response"),(status=202,description="Persisted email job"),(status=403,description="Access denied")),tag="email")]
pub async fn overview(
    State(state): State<AppState>,
    RequireAuth(user): RequireAuth,
) -> Result<Json<Value>, Error> {
    // Reconcile interrupted/cancelled jobs so accounts never remain pending forever.
    let pending:Vec<(String,Option<String>,i64)>=sqlx::query_as("SELECT id,pending_job_id,updated_at FROM email_mailboxes WHERE state='pending' AND (? OR user_id=?)").bind(user.is_admin()).bind(user.id).fetch_all(&state.pool).await.map_err(db_error)?;
    for (id, job_id, updated) in pending {
        if let Some(job_id) = job_id {
            let job = state
                .job_runner
                .job_repo
                .find_by_id(&job_id)
                .await
                .map_err(db_error)?;
            let failed = job
                .as_ref()
                .is_some_and(|j| matches!(j.status.as_str(), "failed" | "cancelled" | "succeeded"))
                || (job.is_none() && chrono::Utc::now().timestamp() - updated > 60);
            if failed {
                sqlx::query("UPDATE email_mailboxes SET state='failed',error='Tugas terhenti atau dibatalkan. Terapkan ulang untuk menyelaraskan layanan.' WHERE id=? AND state='pending' AND pending_job_id=?").bind(id).bind(job_id).execute(&state.pool).await.map_err(db_error)?;
            }
        }
    }
    let boxes: Vec<Mailbox> =
        sqlx::query_as("SELECT * FROM email_mailboxes WHERE (? OR user_id=?) ORDER BY address")
            .bind(user.is_admin())
            .bind(user.id)
            .fetch_all(&state.pool)
            .await
            .map_err(db_error)?;
    let domains=sqlx::query("SELECT d.id,d.name,d.site_id FROM domains d JOIN sites s ON s.id=d.site_id JOIN users u ON u.id=s.user_id WHERE (? OR s.user_id=?) AND d.status!='disabled' AND s.status='active' AND u.role='user' AND u.status='active' AND u.linux_uid BETWEEN 10000 AND 59999 ORDER BY d.name").bind(user.is_admin()).bind(user.id).fetch_all(&state.pool).await.map_err(db_error)?.iter().map(|r|json!({"id":r.get::<i64,_>("id"),"name":r.get::<String,_>("name"),"site_id":r.get::<i64,_>("site_id")})).collect::<Vec<_>>();
    let server = state.agent_client.email(EmailRequest::Status).await;
    let (server, server_error) = match server {
        Ok(v) => (Some(v), None),
        Err(_) => (None, Some("Status layanan email belum tersedia")),
    };
    let certs = if user.is_admin() {
        sqlx::query("SELECT id,primary_domain,san_domains_json,not_after FROM certificates WHERE status='active' AND not_after>? AND not_before<=? AND issuer!='letsencrypt_staging'").bind(chrono::Utc::now().timestamp()).bind(chrono::Utc::now().timestamp()).fetch_all(&state.pool).await.map_err(db_error)?.iter().map(|r|json!({"id":r.get::<i64,_>("id"),"hostname":r.get::<String,_>("primary_domain"),"san_domains":serde_json::from_str::<Value>(&r.get::<String,_>("san_domains_json")).unwrap_or(json!([]))})).collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    Ok(Json(
        json!({"mailboxes":boxes.iter().map(Mailbox::public).collect::<Vec<_>>(),"domains":domains,"server":server,"server_error":server_error,"certificates":certs}),
    ))
}
pub async fn validate_server(
    pool: &DbPool,
    config: &EmailServerConfig,
) -> Result<EmailServerConfig, String> {
    let host =
        panel_core::DomainName::parse(&config.hostname).map_err(|_| "Hostname mail tidak valid")?;
    if host.as_ascii() != config.hostname {
        return Err("Gunakan hostname ASCII huruf kecil".into());
    }
    let cert:Option<(String,String)>=sqlx::query_as("SELECT primary_domain,san_domains_json FROM certificates WHERE id=? AND status='active' AND not_after>? AND not_before<=? AND issuer!='letsencrypt_staging'").bind(config.certificate_id).bind(chrono::Utc::now().timestamp()).bind(chrono::Utc::now().timestamp()).fetch_optional(pool).await.map_err(|_|"Gagal membaca sertifikat")?;
    let (primary, sans) = cert.ok_or("Sertifikat aktif yang belum kedaluwarsa diperlukan")?;
    let mut names: Vec<String> =
        serde_json::from_str(&sans).map_err(|_| "Daftar SAN sertifikat tidak valid")?;
    names.push(primary.clone());
    if !names.iter().any(|n| {
        n == &config.hostname
            || n.strip_prefix("*.").is_some_and(|base| {
                config
                    .hostname
                    .split_once('.')
                    .is_some_and(|(_, rest)| rest == base)
            })
    }) {
        return Err("Sertifikat tidak mencakup hostname mail".into());
    }
    Ok(EmailServerConfig {
        hostname: config.hostname.clone(),
        certificate_id: config.certificate_id,
        certificate_domain: primary,
    })
}
#[utoipa::path(post,path="/api/v1/email/install",request_body=EmailServerConfig,responses((status=200,description="Email response"),(status=202,description="Persisted email job"),(status=403,description="Access denied")),tag="email")]
pub async fn install(
    State(state): State<AppState>,
    RequireAdminCsrf(user): RequireAdminCsrf,
    Json(config): Json<EmailServerConfig>,
) -> Result<(StatusCode, Json<Value>), Error> {
    let config = validate_server(&state.pool, &config)
        .await
        .map_err(|e| fail(StatusCode::BAD_REQUEST, e))?;
    let (job, duplicate) = state
        .job_runner
        .enqueue_email(
            Some(user.id),
            "email.install",
            serde_json::to_string(&config).map_err(db_error)?,
        )
        .await
        .map_err(db_error)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"job":JobItem::from(job),"duplicate":duplicate})),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[derive(utoipa::ToSchema)]
pub struct CreateMailbox {
    domain_id: i64,
    local_part: String,
    password: Option<String>,
    quota_mb: u32,
}
fn password(value: Option<String>) -> Result<(String, String), Error> {
    let pass = value
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("Qp!{}", Uuid::new_v4().simple()));
    if !(12..=128).contains(&pass.len()) || pass.chars().any(char::is_control) {
        return Err(fail(
            StatusCode::BAD_REQUEST,
            "Password harus 12–128 karakter tanpa karakter kontrol",
        ));
    }
    let hash = panel_core::hash_password(&pass).map_err(db_error)?;
    Ok((pass, hash))
}
async fn queue_mailbox(
    state: &AppState,
    user_id: i64,
    id: &str,
    job_id: String,
) -> Result<Value, Error> {
    match state
        .job_runner
        .enqueue(
            Some(user_id),
            Some(job_id.clone()),
            "email.mailbox".into(),
            json!({"id":id}).to_string(),
            Some(0),
        )
        .await
    {
        Ok((job, _)) => Ok(json!(JobItem::from(job))),
        Err(e) => {
            let _=sqlx::query("UPDATE email_mailboxes SET state='failed',error='Gagal memasukkan tugas ke antrean' WHERE id=? AND pending_job_id=?").bind(id).bind(job_id).execute(&state.pool).await;
            Err(db_error(e))
        }
    }
}
#[utoipa::path(post,path="/api/v1/email/mailboxes",request_body=CreateMailbox,responses((status=200,description="Email response"),(status=202,description="Persisted email job"),(status=403,description="Access denied")),tag="email")]
pub async fn create_mailbox(
    State(state): State<AppState>,
    RequireCsrf(user): RequireCsrf,
    Json(p): Json<CreateMailbox>,
) -> Result<(StatusCode, Json<Value>), Error> {
    if !(10..=102400).contains(&p.quota_mb) {
        return Err(fail(StatusCode::BAD_REQUEST, "Kuota harus 10–102400 MiB"));
    }
    let row=sqlx::query("SELECT d.name,d.site_id,s.user_id FROM domains d JOIN sites s ON s.id=d.site_id WHERE d.id=? AND d.status!='disabled' AND s.status='active'").bind(p.domain_id).fetch_optional(&state.pool).await.map_err(db_error)?.ok_or_else(||fail(StatusCode::NOT_FOUND,"Domain aktif tidak ditemukan"))?;
    if !user.can_access_user(row.get("user_id")) {
        return Err(fail(StatusCode::NOT_FOUND, "Domain tidak ditemukan"));
    }
    crate::tenant_context::resolve_tenant_context(&state, &user, None, Some(row.get("site_id")))
        .await?;
    let address = format!(
        "{}@{}",
        p.local_part.trim().to_ascii_lowercase(),
        row.get::<String, _>("name")
    );
    // Same validation is repeated by the agent.
    if p.local_part.is_empty()
        || p.local_part.len() > 64
        || !p
            .local_part
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
        || p.local_part.starts_with('.')
        || p.local_part.ends_with('.')
        || p.local_part.contains("..")
    {
        return Err(fail(StatusCode::BAD_REQUEST, "Nama akun email tidak valid"));
    }
    let server = state
        .agent_client
        .email(EmailRequest::Status)
        .await
        .map_err(agent_error)?;
    if server["configured"].as_bool() != Some(true) {
        return Err(fail(
            StatusCode::CONFLICT,
            "Administrator perlu memasang server email dahulu",
        ));
    }
    let (pass, hash) = password(p.password)?;
    let id = Uuid::new_v4().to_string();
    let job_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    let result=sqlx::query("INSERT INTO email_mailboxes(id,user_id,domain_id,address,password_hash,quota_mb,pending_job_id,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)").bind(&id).bind(row.get::<i64,_>("user_id")).bind(p.domain_id).bind(&address).bind(hash).bind(p.quota_mb).bind(&job_id).bind(now).bind(now).execute(&state.pool).await;
    if let Err(e) = result {
        return Err(
            if e.as_database_error()
                .is_some_and(|e| e.is_unique_violation())
            {
                fail(
                    StatusCode::CONFLICT,
                    "Alamat email sudah ada; periksa daftar akun dan tugas",
                )
            } else {
                db_error(e)
            },
        );
    }
    let job = queue_mailbox(&state, user.id, &id, job_id).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"id":id,"address":address,"password":pass,"job":job})),
    ))
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
#[derive(utoipa::ToSchema)]
pub enum MailboxAction {
    ResetPassword { password: Option<String> },
    SetEnabled { enabled: bool },
    Delete,
    Retry,
}
#[utoipa::path(post,path="/api/v1/email/mailboxes/{id}",request_body=MailboxAction,params(("id"=String,Path,description="Mailbox UUID")),responses((status=200,description="Email response"),(status=202,description="Persisted email job"),(status=403,description="Access denied")),tag="email")]
pub async fn change_mailbox(
    State(state): State<AppState>,
    RequireCsrf(user): RequireCsrf,
    Path(id): Path<String>,
    Json(action): Json<MailboxAction>,
) -> Result<(StatusCode, Json<Value>), Error> {
    let m = mailbox(&state.pool, &user, &id).await?;
    if m.state == "pending" {
        return Err(fail(
            StatusCode::CONFLICT,
            "Perubahan mailbox masih berjalan. Lihat halaman Tugas",
        ));
    }
    let mut pass = None;
    let mut hash = m.password_hash;
    let mut enabled = m.enabled;
    let mut operation = m.operation;
    match action {
        MailboxAction::ResetPassword { password: p } => {
            let (p, h) = password(p)?;
            pass = Some(p);
            hash = h;
            operation = "apply".into();
        }
        MailboxAction::SetEnabled { enabled: e } => {
            enabled = e;
            operation = "apply".into();
        }
        MailboxAction::Delete => operation = "delete".into(),
        MailboxAction::Retry => {}
    }
    let job_id = Uuid::new_v4().to_string();
    let changed=sqlx::query("UPDATE email_mailboxes SET password_hash=?,enabled=?,operation=?,state='pending',pending_job_id=?,error=NULL,updated_at=? WHERE id=? AND state!='pending'").bind(hash).bind(enabled).bind(operation).bind(&job_id).bind(chrono::Utc::now().timestamp()).bind(&id).execute(&state.pool).await.map_err(db_error)?.rows_affected();
    if changed != 1 {
        return Err(fail(
            StatusCode::CONFLICT,
            "Mailbox sedang berubah; muat ulang daftar",
        ));
    }
    let job = queue_mailbox(&state, user.id, &id, job_id).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"job":job,"password":pass,"address":m.address})),
    ))
}
#[derive(Deserialize)]
pub struct FolderQuery {
    folder: EmailFolder,
    before: Option<u32>,
}
#[utoipa::path(get,path="/api/v1/email/mailboxes/{id}/messages",params(("id"=String,Path,description="Mailbox UUID")),responses((status=200,description="Email response"),(status=202,description="Persisted email job"),(status=403,description="Access denied")),tag="email")]
pub async fn messages(
    State(state): State<AppState>,
    RequireAuth(user): RequireAuth,
    Path(id): Path<String>,
    Query(q): Query<FolderQuery>,
) -> Result<Json<Value>, Error> {
    let m = mailbox(&state.pool, &user, &id).await?;
    if m.state != "active" {
        return Err(fail(StatusCode::CONFLICT, "Mailbox belum aktif"));
    }
    state
        .agent_client
        .email(EmailRequest::Messages {
            id,
            folder: q.folder,
            before: q.before,
        })
        .await
        .map(Json)
        .map_err(agent_error)
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
#[derive(utoipa::ToSchema)]
pub enum MessageAction {
    Read {
        folder: EmailFolder,
        uid: u32,
        uid_validity: u32,
    },
    Trash {
        folder: EmailFolder,
        uid: u32,
        uid_validity: u32,
    },
}
#[utoipa::path(post,path="/api/v1/email/mailboxes/{id}/messages",request_body=MessageAction,params(("id"=String,Path,description="Mailbox UUID")),responses((status=200,description="Email response"),(status=202,description="Persisted email job"),(status=403,description="Access denied")),tag="email")]
pub async fn message(
    State(state): State<AppState>,
    RequireCsrf(user): RequireCsrf,
    Path(id): Path<String>,
    Json(action): Json<MessageAction>,
) -> Result<Json<Value>, Error> {
    let m = mailbox(&state.pool, &user, &id).await?;
    if m.state != "active" {
        return Err(fail(StatusCode::CONFLICT, "Mailbox belum aktif"));
    }
    let cmd = match action {
        MessageAction::Read {
            folder,
            uid,
            uid_validity,
        } => EmailRequest::Message {
            id,
            folder,
            uid,
            uid_validity,
        },
        MessageAction::Trash {
            folder,
            uid,
            uid_validity,
        } => EmailRequest::MessageTrash {
            id,
            folder,
            uid,
            uid_validity,
        },
    };
    state
        .agent_client
        .email(cmd)
        .await
        .map(Json)
        .map_err(agent_error)
}
#[utoipa::path(get,path="/api/v1/email/mailboxes/{id}/dns",params(("id"=String,Path,description="Mailbox UUID")),responses((status=200,description="Email response"),(status=202,description="Persisted email job"),(status=403,description="Access denied")),tag="email")]
pub async fn dns(
    State(state): State<AppState>,
    RequireAuth(user): RequireAuth,
    Path(id): Path<String>,
) -> Result<Json<Value>, Error> {
    let m = mailbox(&state.pool, &user, &id).await?;
    let domain = m
        .address
        .split_once('@')
        .ok_or_else(|| db_error("address"))?
        .1
        .to_owned();
    let mut value = state
        .agent_client
        .email(EmailRequest::DnsRecords { domain })
        .await
        .map_err(agent_error)?;
    value["ipv4"] = sqlx::query_scalar::<_, String>(
        "SELECT value FROM settings WHERE key='network.public_ipv4'",
    )
    .fetch_optional(&state.pool)
    .await
    .map_err(db_error)?
    .map_or(Value::Null, Value::String);
    value["ipv6"] = sqlx::query_scalar::<_, String>(
        "SELECT value FROM settings WHERE key='network.public_ipv6'",
    )
    .fetch_optional(&state.pool)
    .await
    .map_err(db_error)?
    .map_or(Value::Null, Value::String);
    Ok(Json(value))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[derive(utoipa::ToSchema)]
pub struct Compose {
    #[schema(value_type=String)]
    request_id: Uuid,
    to: String,
    subject: String,
    body: String,
}
#[utoipa::path(post,path="/api/v1/email/mailboxes/{id}/send",request_body=Compose,params(("id"=String,Path,description="Mailbox UUID")),responses((status=200,description="Email response"),(status=202,description="Persisted email job"),(status=403,description="Access denied")),tag="email")]
pub async fn send(
    State(state): State<AppState>,
    RequireCsrf(user): RequireCsrf,
    Path(id): Path<String>,
    Json(p): Json<Compose>,
) -> Result<(StatusCode, Json<Value>), Error> {
    let m = mailbox(&state.pool, &user, &id).await?;
    if m.state != "active" {
        return Err(fail(StatusCode::CONFLICT, "Mailbox belum aktif"));
    }
    panel_core::email::build_message(
        &m.address,
        &m.address,
        &EmailMessage {
            to: p.to.clone(),
            subject: p.subject.clone(),
            body: p.body.clone(),
        },
    )
    .map_err(|e| fail(StatusCode::BAD_REQUEST, e))?;
    let delivery = p.request_id.to_string();
    if let Some(existing) = state
        .job_runner
        .job_repo
        .find_by_id(&delivery)
        .await
        .map_err(db_error)?
        && (existing.user_id != Some(user.id) || existing.name != "email.send")
    {
        return Err(fail(StatusCode::CONFLICT, "ID pengiriman sudah digunakan"));
    }
    let content_hash = panel_core::hash_token(
        &serde_json::to_string(&json!([&p.to, &p.subject, &p.body])).map_err(db_error)?,
    );
    let existing: Option<(i64, String, String)> =
        sqlx::query_as("SELECT user_id,mailbox_id,content_hash FROM email_deliveries WHERE id=?")
            .bind(&delivery)
            .fetch_optional(&state.pool)
            .await
            .map_err(db_error)?;
    if let Some((uid, mid, fingerprint)) = existing {
        if uid != user.id || mid != id || fingerprint != content_hash {
            return Err(fail(
                StatusCode::CONFLICT,
                "ID pengiriman sudah dipakai untuk pesan berbeda",
            ));
        }
    } else {
        sqlx::query("INSERT INTO email_deliveries(id,user_id,mailbox_id,to_address,subject,body,content_hash,created_at) VALUES(?,?,?,?,?,?,?,?)").bind(&delivery).bind(user.id).bind(&id).bind(p.to).bind(p.subject).bind(p.body).bind(content_hash).bind(chrono::Utc::now().timestamp()).execute(&state.pool).await.map_err(db_error)?;
    }
    let (job, _) = state
        .job_runner
        .enqueue(
            Some(user.id),
            Some(delivery.clone()),
            "email.send".into(),
            json!({"id":delivery}).to_string(),
            Some(0),
        )
        .await
        .map_err(db_error)?;
    if job.user_id != Some(user.id) || job.name != "email.send" {
        return Err(fail(StatusCode::CONFLICT, "ID pengiriman sudah digunakan"));
    }
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"job":JobItem::from(job)})),
    ))
}
pub struct EmailHandler(pub Arc<AgentClient>);
impl JobHandler for EmailHandler {
    fn run<'a>(&'a self, job: &'a panel_db::models::Job, ctx: &'a JobContext) -> JobFuture<'a> {
        Box::pin(async move {
            let actor: Option<(String, String)> =
                sqlx::query_as("SELECT role,status FROM users WHERE id=?")
                    .bind(job.user_id)
                    .fetch_optional(&ctx.pool)
                    .await
                    .map_err(|_| "Identitas tugas tidak tersedia")?;
            if job.name != "email.deliver"
                && !actor.as_ref().is_some_and(|(_, status)| status == "active")
            {
                return Err("Pengguna tugas tidak aktif".into());
            }
            let payload: Value =
                serde_json::from_str(&job.payload_json).map_err(|_| "Payload email tidak valid")?;
            ctx.report_progress(10, Some("Memeriksa konfigurasi dan izin email"))
                .await
                .map_err(|e| e.to_string())?;
            if job.name == "email.install" {
                if actor != Some(("admin".into(), "active".into())) {
                    return Err("Hanya administrator aktif yang dapat memasang email".into());
                }
                let config: EmailServerConfig = serde_json::from_value(payload)
                    .map_err(|_| "Konfigurasi server email tidak valid")?;
                let config = validate_server(&ctx.pool, &config).await?;
                return match ctx
                    .install(
                        &self.0,
                        panel_proto::InstallOperation::EmailServer(config),
                        60,
                    )
                    .await?
                {
                    panel_proto::ResponsePayload::Email(v) => Ok(Some(v.to_string())),
                    _ => Err("Respons instalasi email tidak sesuai".into()),
                };
            }
            let id = payload["id"].as_str().ok_or("ID operasi tidak ada")?;
            if job.name == "email.mailbox" {
                let m:Mailbox=sqlx::query_as("SELECT * FROM email_mailboxes WHERE id=? AND pending_job_id=? AND state='pending'").bind(id).bind(&job.id).fetch_optional(&ctx.pool).await.map_err(|_|"Mailbox gagal dibaca")?.ok_or("Operasi mailbox sudah digantikan atau dibatalkan")?;
                if job.user_id != Some(m.user_id)
                    && actor.as_ref().map(|a| a.0.as_str()) != Some("admin")
                {
                    return Err("Akses mailbox ditolak".into());
                }
                let result=async {
                let command=if m.operation=="delete" {EmailRequest::MailboxRemove{id:id.into()}}else{
                    let tenant=panel_db::repositories::SqliteUserRepository::new(ctx.pool.clone()).find_by_id(m.user_id).await.map_err(|_|"Tenant tidak tersedia")?.ok_or("Tenant tidak tersedia")?;
                    let(name,uid)=crate::tenant_context::execution_identity(&tenant).map_err(|e|e.1)?;
                    let domain:Option<String>=sqlx::query_scalar("SELECT d.name FROM domains d JOIN sites s ON s.id=d.site_id WHERE d.id=? AND s.user_id=? AND s.status='active' AND d.status!='disabled'").bind(m.domain_id).bind(m.user_id).fetch_optional(&ctx.pool).await.map_err(|_|"Domain tidak tersedia")?;
                    if domain.as_deref()!=m.address.split_once('@').map(|(_,d)|d) {return Err("Kepemilikan domain berubah; operasi dihentikan".into());}
                    EmailRequest::MailboxApply{mailbox:EmailMailbox{id:id.into(),address:m.address,password_hash:EmailSecret(m.password_hash),linux_username:name,uid,quota_mb:u32::try_from(m.quota_mb).map_err(|_|"Kuota tidak valid")?,enabled:m.enabled}}
                };
                ctx.report_progress(40,Some("Menerapkan akun, kuota dan konfigurasi layanan email")).await.map_err(|e|e.to_string())?;
                self.0.email(command).await.map_err(|e|e.to_string())
            }.await;
                match result {
                    Ok(v) => {
                        if m.operation == "delete" {
                            sqlx::query("DELETE FROM email_deliveries WHERE mailbox_id=?")
                                .bind(id)
                                .execute(&ctx.pool)
                                .await
                                .map_err(|_| "Riwayat email gagal dibersihkan")?;
                            sqlx::query(
                                "DELETE FROM email_mailboxes WHERE id=? AND pending_job_id=?",
                            )
                            .bind(id)
                            .bind(&job.id)
                            .execute(&ctx.pool)
                            .await
                            .map_err(|_| "Akun gagal dihapus")?;
                        } else {
                            sqlx::query("UPDATE email_mailboxes SET state=?,error=NULL WHERE id=? AND pending_job_id=?").bind(if m.enabled{"active"}else{"disabled"}).bind(id).bind(&job.id).execute(&ctx.pool).await.map_err(|_|"Status akun gagal disimpan")?;
                        }
                        return Ok(Some(v.to_string()));
                    }
                    Err(error) => {
                        let _=sqlx::query("UPDATE email_mailboxes SET state='failed',error=? WHERE id=? AND pending_job_id=?").bind(&error).bind(id).bind(&job.id).execute(&ctx.pool).await;
                        return Err(error);
                    }
                }
            }
            if !matches!(job.name.as_str(), "email.send" | "email.deliver") || id != job.id {
                return Err("Operasi pengiriman tidak valid".into());
            }
            let row = sqlx::query("SELECT * FROM email_deliveries WHERE id=? AND state='queued'")
                .bind(id)
                .fetch_optional(&ctx.pool)
                .await
                .map_err(|_| "Email gagal dibaca")?
                .ok_or("Pengiriman sudah diproses; periksa antrean sebelum mengirim ulang")?;
            let user_id: Option<i64> = row.get("user_id");
            if user_id != job.user_id {
                return Err("Pemilik pengiriman tidak sesuai".into());
            }
            let mailbox_id: Option<String> = row.get("mailbox_id");
            let msg = EmailMessage {
                to: row.get("to_address"),
                subject: row.get("subject"),
                body: row.get("body"),
            };
            let changed = sqlx::query(
                "UPDATE email_deliveries SET state='sending' WHERE id=? AND state='queued'",
            )
            .bind(id)
            .execute(&ctx.pool)
            .await
            .map_err(|_| "Status kirim gagal diperbarui")?
            .rows_affected();
            if changed != 1 {
                return Err("Pengiriman sedang diproses".into());
            }
            let result=async {
            if job.name=="email.send" {
                let mid=mailbox_id.ok_or("Mailbox pengirim tidak tersedia")?;
                let owner:Option<i64>=sqlx::query_scalar("SELECT m.user_id FROM email_mailboxes m JOIN users u ON u.id=m.user_id JOIN domains d ON d.id=m.domain_id JOIN sites s ON s.id=d.site_id WHERE m.id=? AND m.state='active' AND m.enabled=1 AND u.status='active' AND d.status!='disabled' AND s.status='active' AND s.user_id=m.user_id").bind(&mid).fetch_optional(&ctx.pool).await.map_err(|_|"Identitas pengirim gagal dibaca")?;
                if owner.is_none() || (owner!=job.user_id && actor.as_ref().map(|a|a.0.as_str())!=Some("admin")) {return Err("Mailbox pengirim tidak aktif atau akses ditolak".into());}
                self.0.email(EmailRequest::Send{id:mid,to:msg.to,subject:msg.subject,body:msg.body}).await.map_err(|e|e.to_string())
            }else{
                if mailbox_id.is_some(){return Err("Pengiriman notifikasi tidak sesuai".into());}
                let s=SqliteNotificationRepository::new(ctx.pool.clone()).get_settings().await.map_err(|_|"SMTP gagal dibaca")?;
                if !s.smtp_enabled{return Err("Notifikasi SMTP sedang dinonaktifkan".into());}
                let config=SmtpConfig{host:s.smtp_host.unwrap_or_default(),port:u16::try_from(s.smtp_port).map_err(|_|"Port SMTP tidak valid")?,username:s.smtp_username,password:smtp_password(&ctx.pool,&self.0,s.smtp_password_enc).await?,encryption:s.smtp_encryption,from_address:s.from_address,from_name:s.from_name};
                SmtpClient::send_mail(&config,&msg).await?;Ok(json!({"accepted":true,"message":"Pesan diterima server SMTP"}))
            }
        }.await;
            let accepted = result.is_ok();
            // Remove message contents after processing; job history contains no body or credentials.
            sqlx::query("UPDATE email_deliveries SET state=?,body='',subject='' WHERE id=?").bind(if accepted{"accepted"}else{"uncertain"}).bind(id).execute(&ctx.pool).await.map_err(|_|"Status penerimaan email gagal disimpan; periksa antrean sebelum mengirim ulang")?;
            result.map(|v|Some(v.to_string())).map_err(|e:String|format!("{e}. Penerimaan pesan belum dapat dipastikan; periksa antrean dan log SMTP sebelum mengirim ulang."))
        })
    }
}
