//! SSL/TLS and ACME Certificate Management for Qinpanel.
//!
//! Provides automated Let's Encrypt issuance (HTTP-01 challenge),
//! fallback self-signed certificate generation, custom certificate upload and validation,
//! strict 0600 private key deployment, and lifetime-aware automatic renewal.

use crate::auth::middleware::AuthenticatedUser;
use crate::routes::AppState;
use crate::sites::{fetch_and_authorize_site, recompile_and_apply_site_vhost};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use instant_acme::{
    Account, AccountCredentials, ChallengeType, Identifier, LetsEncrypt, NewAccount, NewOrder,
    OrderStatus, RetryPolicy,
};
use panel_db::models::{Certificate, CreateCertificate, Domain, Site, UpdateCertificate, User};
use panel_db::repositories::{
    SqliteCertificateRepository, domain_repo::SqliteDomainRepository,
    site_repo::SqliteSiteRepository, user_repo::SqliteUserRepository,
};
use panel_proto::{AcmeChallengeCleanupRequest, AcmeChallengeDeployRequest, SslCertDeployRequest};
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, SanType};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};
use utoipa::ToSchema;
use x509_parser::pem::parse_x509_pem;

/// Request payload to issue an ACME / Let's Encrypt certificate.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IssueSslRequest {
    /// Contact email for Let's Encrypt account. If omitted, uses tenant email.
    pub email: Option<String>,
    /// Whether to use Let's Encrypt staging environment (avoids rate limits). Default is false.
    pub staging: Option<bool>,
    /// If ACME verification fails, fallback to generating a self-signed certificate. Default is true.
    pub fallback_self_signed: Option<bool>,
    /// Enable OCSP stapling in Nginx. Default is true for ACME certs.
    pub ocsp_stapling: Option<bool>,
}

/// Request payload to upload a custom certificate.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CustomCertRequest {
    /// Full PEM certificate chain including leaf and intermediate certificates.
    pub fullchain_pem: String,
    /// PEM encoded private key matching the certificate.
    pub privkey_pem: String,
    /// Whether to mark for auto-renewal (for custom, default is false).
    pub auto_renew: Option<bool>,
}

/// Response payload containing SSL certificate details.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SslStatusResponse {
    pub has_ssl: bool,
    pub certificate: Option<CertificateDetailResponse>,
    pub force_https: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CertificateDetailResponse {
    pub id: i64,
    pub primary_domain: String,
    pub san_domains: Vec<String>,
    pub issuer: String,
    pub not_before: i64,
    pub not_after: i64,
    pub days_remaining: i64,
    pub auto_renew: bool,
    pub ocsp_stapling: bool,
    pub last_renew_attempt: Option<i64>,
    pub renew_error: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SslActionResponse {
    pub message: String,
    pub certificate: Option<CertificateDetailResponse>,
}

/// Generates a self-signed X.509 certificate using `rcgen`.
pub fn generate_self_signed_cert(
    primary_domain: &str,
    san_domains: &[String],
) -> Result<(String, String, i64, i64), String> {
    let mut params = CertificateParams::default();

    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, primary_domain);
    dn.push(DnType::OrganizationName, "Qinpanel Self-Signed");
    params.distinguished_name = dn;

    let mut sans = vec![SanType::DnsName(
        primary_domain
            .to_string()
            .try_into()
            .map_err(|e| format!("Invalid primary domain: {e}"))?,
    )];

    for san in san_domains {
        sans.push(SanType::DnsName(
            san.clone()
                .try_into()
                .map_err(|e| format!("Invalid SAN domain '{san}': {e}"))?,
        ));
    }
    params.subject_alt_names = sans;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let not_before = now;
    let not_after = now + (90 * 86400);

    if let Ok(nb) = time::OffsetDateTime::from_unix_timestamp(not_before) {
        params.not_before = nb;
    }
    if let Ok(na) = time::OffsetDateTime::from_unix_timestamp(not_after) {
        params.not_after = na;
    }

    let key_pair = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .map_err(|e| format!("Failed to generate key pair: {e}"))?;

    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| format!("Failed to self-sign certificate: {e}"))?;

    let fullchain_pem = cert.pem();
    let privkey_pem = key_pair.serialize_pem();

    let (parsed_nb, parsed_na, _, _) = inspect_x509_cert(&fullchain_pem)?;

    Ok((fullchain_pem, privkey_pem, parsed_nb, parsed_na))
}

/// Inspects and parses an X.509 certificate PEM string using `x509-parser`.
pub fn inspect_x509_cert(cert_pem: &str) -> Result<(i64, i64, String, Vec<String>), String> {
    let (_, pem) = parse_x509_pem(cert_pem.as_bytes())
        .map_err(|e| format!("Invalid PEM certificate format: {e}"))?;

    let (_, x509) = x509_parser::parse_x509_certificate(&pem.contents)
        .map_err(|e| format!("Failed to parse X.509 certificate: {e}"))?;

    let not_before = x509.validity().not_before.timestamp();
    let not_after = x509.validity().not_after.timestamp();
    let issuer = x509.issuer().to_string();

    let mut sans = Vec::new();
    if let Ok(Some(ext)) = x509.subject_alternative_name() {
        for name in &ext.value.general_names {
            if let x509_parser::extensions::GeneralName::DNSName(dns) = name {
                sans.push(dns.to_string());
            }
        }
    }

    Ok((not_before, not_after, issuer, sans))
}

/// Converts a DB `Certificate` model into `CertificateDetailResponse`.
pub fn cert_to_detail_response(cert: &Certificate) -> CertificateDetailResponse {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let diff = cert.not_after - now;
    let days_remaining = if diff > 0 { diff / 86400 } else { 0 };

    let san_domains: Vec<String> = serde_json::from_str(&cert.san_domains_json).unwrap_or_default();

    CertificateDetailResponse {
        id: cert.id,
        primary_domain: cert.primary_domain.clone(),
        san_domains,
        issuer: cert.issuer.clone(),
        not_before: cert.not_before,
        not_after: cert.not_after,
        days_remaining,
        auto_renew: cert.auto_renew == 1,
        ocsp_stapling: cert.ocsp_stapling == 1,
        last_renew_attempt: cert.last_renew_attempt,
        renew_error: cert.renew_error.clone(),
        status: cert.status.clone(),
    }
}

/// Helper to get or create an ACME account with instant-acme.
pub(crate) async fn get_acme_account(
    pool: &panel_db::DbPool,
    cert_repo: &SqliteCertificateRepository,
    email: &str,
    directory_url: &str,
) -> Result<Account, String> {
    let acme_account = cert_repo
        .get_or_create_acme_account(&panel_db::models::CreateAcmeAccount {
            email: email.to_string(),
            directory_url: directory_url.to_string(),
            account_uri: None,
            private_key_pem: String::new(),
        })
        .await
        .map_err(|e| format!("Failed to access ACME account in DB: {e}"))?;

    if !acme_account.private_key_pem.is_empty() {
        let creds: AccountCredentials = serde_json::from_str(&acme_account.private_key_pem)
            .map_err(|e| format!("Invalid stored ACME credentials: {e}"))?;
        Account::builder()
            .map_err(|e| format!("AccountBuilder error: {e}"))?
            .from_credentials(creds)
            .await
            .map_err(|e| format!("Failed to restore ACME account: {e}"))
    } else {
        let (acc, creds) = Account::builder()
            .map_err(|e| format!("AccountBuilder error: {e}"))?
            .create(
                &NewAccount {
                    contact: &[&format!("mailto:{email}")],
                    terms_of_service_agreed: true,
                    only_return_existing: false,
                },
                directory_url.to_string(),
                None,
            )
            .await
            .map_err(|e| format!("Failed to register ACME account: {e}"))?;

        let creds_json = serde_json::to_string(&creds)
            .map_err(|e| format!("Failed to serialize ACME credentials: {e}"))?;

        sqlx::query("UPDATE acme_accounts SET private_key_pem = ? WHERE id = ?")
            .bind(&creds_json)
            .bind(acme_account.id)
            .execute(pool)
            .await
            .map_err(|_| "Failed to persist ACME credentials".to_string())?;

        Ok(acc)
    }
}

/// Orders an ACME certificate via Let's Encrypt using HTTP-01 challenge.
pub async fn order_acme_certificate(
    state: &AppState,
    primary_domain: &str,
    san_domains: &[String],
    email: &str,
    staging: bool,
) -> Result<(String, String, i64, i64), String> {
    let directory_url = if staging {
        LetsEncrypt::Staging.url()
    } else {
        LetsEncrypt::Production.url()
    };

    let cert_repo = SqliteCertificateRepository::new(state.pool.clone());
    let account = get_acme_account(&state.pool, &cert_repo, email, directory_url).await?;

    let mut all_domains = vec![primary_domain.to_string()];
    for d in san_domains {
        if !all_domains.contains(d) {
            all_domains.push(d.clone());
        }
    }

    let identifiers: Vec<Identifier> = all_domains
        .iter()
        .map(|d| Identifier::Dns(d.clone()))
        .collect();

    let mut order = account
        .new_order(&NewOrder::new(&identifiers))
        .await
        .map_err(|e| format!("Failed to create ACME order: {e}"))?;

    let mut deployed_tokens = Vec::new();
    let mut authzs = order.authorizations();

    while let Some(authz_res) = authzs.next().await {
        let mut authz = authz_res.map_err(|e| format!("Authorization fetch failed: {e}"))?;
        if let Some(mut chal) = authz.challenge(ChallengeType::Http01) {
            let key_auth = chal.key_authorization();
            let deploy_req = AcmeChallengeDeployRequest {
                token: chal.token.clone(),
                key_authorization: key_auth.as_str().to_string(),
                acme_dir: None,
            };
            state
                .agent_client
                .deploy_acme_challenge(deploy_req)
                .await
                .map_err(|e| format!("Agent IPC challenge deploy error: {e}"))?;

            deployed_tokens.push(chal.token.clone());
            chal.set_ready()
                .await
                .map_err(|e| format!("Failed to set challenge ready: {e}"))?;
        }
    }

    let retries = RetryPolicy::default();
    let status = order
        .poll_ready(&retries)
        .await
        .map_err(|e| format!("Order poll_ready error: {e}"))?;

    // Cleanup challenge files via agent
    for token in deployed_tokens {
        let cleanup_req = AcmeChallengeCleanupRequest {
            token,
            acme_dir: None,
        };
        let _ = state.agent_client.cleanup_acme_challenge(cleanup_req).await;
    }

    if status != OrderStatus::Ready {
        return Err(format!("ACME order status is not ready: {status:?}"));
    }

    let privkey_pem = order
        .finalize()
        .await
        .map_err(|e| format!("Order finalize failed: {e}"))?;

    let fullchain_pem = order
        .poll_certificate(&retries)
        .await
        .map_err(|e| format!("Order poll_certificate failed: {e}"))?;

    let (not_before, not_after, _, _) = inspect_x509_cert(&fullchain_pem)?;

    Ok((fullchain_pem, privkey_pem, not_before, not_after))
}

/// Deploys certificate to agent and recompiles/applies site vhost.
pub async fn deploy_and_activate_site_ssl(
    state: &AppState,
    site: &Site,
    user: &User,
    domains: &[Domain],
    cert: &Certificate,
    ocsp_stapling: bool,
) -> Result<(), (StatusCode, String)> {
    let deploy_req = SslCertDeployRequest {
        domain: cert.primary_domain.clone(),
        fullchain_pem: cert.fullchain_pem.clone(),
        privkey_pem: cert.privkey_pem.clone(),
        ssl_base_dir: None,
    };

    let deployed = state
        .agent_client
        .deploy_ssl_cert(deploy_req)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to deploy certificate to webserver agent: {e}"),
            )
        })?;

    let mut settings: serde_json::Value =
        serde_json::from_str(&site.settings_json).unwrap_or(serde_json::json!({}));

    settings["has_ssl"] = serde_json::Value::Bool(true);
    settings["ssl_cert"] = serde_json::Value::String(deployed.cert_path);
    settings["ssl_key"] = serde_json::Value::String(deployed.key_path);
    settings["ocsp_stapling"] = serde_json::Value::Bool(ocsp_stapling);

    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let updated_site = site_repo
        .update(
            site.id,
            &panel_db::models::UpdateSite {
                settings_json: Some(settings.to_string()),
                ..Default::default()
            },
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to update site settings in DB: {e}"),
            )
        })?;

    recompile_and_apply_site_vhost(state, &updated_site, user, domains).await?;

    Ok(())
}

/// Disables SSL for a site and recompiles vhost to HTTP.
pub async fn deactivate_site_ssl(
    state: &AppState,
    site: &Site,
    user: &User,
    domains: &[Domain],
) -> Result<(), (StatusCode, String)> {
    let mut settings: serde_json::Value =
        serde_json::from_str(&site.settings_json).unwrap_or(serde_json::json!({}));

    settings["has_ssl"] = serde_json::Value::Bool(false);
    settings["ssl_cert"] = serde_json::Value::Null;
    settings["ssl_key"] = serde_json::Value::Null;
    settings["force_https"] = serde_json::Value::Bool(false);
    settings["ocsp_stapling"] = serde_json::Value::Bool(false);

    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let updated_site = site_repo
        .update(
            site.id,
            &panel_db::models::UpdateSite {
                settings_json: Some(settings.to_string()),
                ..Default::default()
            },
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to update site settings in DB: {e}"),
            )
        })?;

    let cert_repo = SqliteCertificateRepository::new(state.pool.clone());
    if let Ok(Some(cert)) = cert_repo.find_active_by_site_id(site.id).await {
        let _ = cert_repo
            .update(
                cert.id,
                &UpdateCertificate {
                    fullchain_pem: None,
                    privkey_pem: None,
                    cert_path: None,
                    key_path: None,
                    not_before: None,
                    not_after: None,
                    auto_renew: Some(false),
                    ocsp_stapling: Some(false),
                    last_renew_attempt: None,
                    renew_error: None,
                    status: Some("revoked".to_string()),
                },
            )
            .await;
    }

    recompile_and_apply_site_vhost(state, &updated_site, user, domains).await?;

    Ok(())
}

/// GET /api/v1/sites/{id}/ssl - Get SSL certificate status for a site.
#[utoipa::path(
    get,
    path = "/api/v1/sites/{id}/ssl",
    tag = "ssl",
    params(
        ("id" = i64, Path, description = "Site ID")
    ),
    responses(
        (status = 200, description = "SSL status retrieved", body = SslStatusResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Site not found")
    )
)]
pub async fn get_site_ssl_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let site = fetch_and_authorize_site(&site_repo, &auth_user, site_id).await?;

    let settings: serde_json::Value =
        serde_json::from_str(&site.settings_json).unwrap_or(serde_json::json!({}));
    let force_https = settings
        .get("force_https")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let cert_repo = SqliteCertificateRepository::new(state.pool.clone());
    let active_cert = cert_repo
        .find_active_by_site_id(site.id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let has_ssl = active_cert.is_some();
    let certificate = active_cert.as_ref().map(cert_to_detail_response);

    Ok((
        StatusCode::OK,
        Json(SslStatusResponse {
            has_ssl,
            certificate,
            force_https,
        }),
    ))
}

/// POST /api/v1/sites/{id}/ssl/issue - Issue Let's Encrypt certificate with fallback self-signed.
#[utoipa::path(
    post,
    path = "/api/v1/sites/{id}/ssl/issue",
    tag = "ssl",
    params(
        ("id" = i64, Path, description = "Site ID")
    ),
    request_body = IssueSslRequest,
    responses(
        (status = 200, description = "Certificate issued successfully", body = CertificateDetailResponse),
        (status = 400, description = "Invalid request or ACME failure"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Site not found")
    )
)]
pub async fn issue_site_ssl_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Json(req): Json<IssueSslRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    crate::dns01::guard_site(&state.pool, site_id).await?;
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let site = fetch_and_authorize_site(&site_repo, &auth_user, site_id)
        .await
        .map_err(|(sc, msg)| (sc, msg.to_string()))?;

    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let user = user_repo
        .find_by_id(site.user_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database error: {e}"),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            "Site tenant user not found".to_string(),
        ))?;

    let domain_repo = SqliteDomainRepository::new(state.pool.clone());
    let domains = domain_repo.find_by_site_id(site.id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Database error: {e}"),
        )
    })?;

    let primary_domain = domains
        .iter()
        .find(|d| d.is_primary)
        .map(|d| d.name.clone())
        .ok_or((
            StatusCode::BAD_REQUEST,
            "Site has no primary domain".to_string(),
        ))?;

    let san_domains: Vec<String> = domains
        .iter()
        .filter(|d| !d.is_primary)
        .map(|d| d.name.clone())
        .collect();

    let email = req
        .email
        .clone()
        .or_else(|| user.email.clone())
        .unwrap_or_else(|| "admin@example.com".to_string());

    let staging = req.staging.unwrap_or(false);
    let fallback = req.fallback_self_signed.unwrap_or(true);
    let ocsp = req.ocsp_stapling.unwrap_or(true);

    let (fullchain, privkey, not_before, not_after, issuer, is_fallback) =
        match order_acme_certificate(&state, &primary_domain, &san_domains, &email, staging).await {
            Ok((chain, key, nb, na)) => {
                let iss = if staging {
                    "letsencrypt_staging"
                } else {
                    "letsencrypt"
                };
                (chain, key, nb, na, iss.to_string(), false)
            }
            Err(err) => {
                warn!("ACME certificate issuance failed: {err}");
                if fallback {
                    info!("Falling back to self-signed certificate for {primary_domain}");
                    let (chain, key, nb, na) =
                        generate_self_signed_cert(&primary_domain, &san_domains)
                            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
                    (chain, key, nb, na, "self_signed".to_string(), true)
                } else {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        format!("ACME certificate order failed: {err}"),
                    ));
                }
            }
        };

    let cert_repo = SqliteCertificateRepository::new(state.pool.clone());
    let cert_path = "/etc/panel/certs/placeholder/fullchain.pem".to_string();
    let key_path = "/etc/panel/certs/placeholder/privkey.pem".to_string();

    let cert_create = CreateCertificate {
        site_id: site.id,
        primary_domain: primary_domain.clone(),
        san_domains_json: serde_json::to_string(&san_domains).unwrap_or_else(|_| "[]".to_string()),
        issuer: issuer.clone(),
        cert_path,
        key_path,
        fullchain_pem: fullchain.clone(),
        privkey_pem: privkey.clone(),
        not_before,
        not_after,
        auto_renew: !is_fallback,
        ocsp_stapling: ocsp && !is_fallback,
        status: "active".to_string(),
    };

    let mut cert = cert_repo.create(&cert_create).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to record cert: {e}"),
        )
    })?;

    // Update real cert and key paths with generated cert ID
    let real_cert_path = format!("/etc/panel/certs/{}/fullchain.pem", cert.id);
    let real_key_path = format!("/etc/panel/certs/{}/privkey.pem", cert.id);

    cert = cert_repo
        .update(
            cert.id,
            &UpdateCertificate {
                fullchain_pem: None,
                privkey_pem: None,
                cert_path: Some(real_cert_path),
                key_path: Some(real_key_path),
                not_before: None,
                not_after: None,
                auto_renew: None,
                ocsp_stapling: None,
                last_renew_attempt: None,
                renew_error: None,
                status: None,
            },
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to update cert path: {e}"),
            )
        })?;

    deploy_and_activate_site_ssl(&state, &site, &user, &domains, &cert, ocsp && !is_fallback)
        .await?;

    state.audit_writer.log_async(
        Some(auth_user.id),
        &auth_user.username,
        "site.ssl.issue",
        format!("site:{}", site.id),
        "success",
        None,
        Some(format!(
            "issuer:{issuer},domain:{primary_domain},fallback:{is_fallback}"
        )),
    );

    Ok((StatusCode::OK, Json(cert_to_detail_response(&cert))))
}

/// POST /api/v1/sites/{id}/ssl/self-signed - Issue a self-signed certificate directly.
#[utoipa::path(
    post,
    path = "/api/v1/sites/{id}/ssl/self-signed",
    tag = "ssl",
    params(
        ("id" = i64, Path, description = "Site ID")
    ),
    responses(
        (status = 200, description = "Self-signed certificate generated", body = CertificateDetailResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Site not found")
    )
)]
pub async fn self_signed_site_ssl_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    crate::dns01::guard_site(&state.pool, site_id).await?;
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let site = fetch_and_authorize_site(&site_repo, &auth_user, site_id)
        .await
        .map_err(|(sc, msg)| (sc, msg.to_string()))?;

    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let user = user_repo
        .find_by_id(site.user_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database error: {e}"),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            "Site tenant user not found".to_string(),
        ))?;

    let domain_repo = SqliteDomainRepository::new(state.pool.clone());
    let domains = domain_repo.find_by_site_id(site.id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Database error: {e}"),
        )
    })?;

    let primary_domain = domains
        .iter()
        .find(|d| d.is_primary)
        .map(|d| d.name.clone())
        .ok_or((
            StatusCode::BAD_REQUEST,
            "Site has no primary domain".to_string(),
        ))?;

    let san_domains: Vec<String> = domains
        .iter()
        .filter(|d| !d.is_primary)
        .map(|d| d.name.clone())
        .collect();

    let (fullchain, privkey, not_before, not_after) =
        generate_self_signed_cert(&primary_domain, &san_domains)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let cert_repo = SqliteCertificateRepository::new(state.pool.clone());
    let cert_create = CreateCertificate {
        site_id: site.id,
        primary_domain: primary_domain.clone(),
        san_domains_json: serde_json::to_string(&san_domains).unwrap_or_else(|_| "[]".to_string()),
        issuer: "self_signed".to_string(),
        cert_path: "/etc/panel/certs/placeholder/fullchain.pem".to_string(),
        key_path: "/etc/panel/certs/placeholder/privkey.pem".to_string(),
        fullchain_pem: fullchain.clone(),
        privkey_pem: privkey.clone(),
        not_before,
        not_after,
        auto_renew: true,
        ocsp_stapling: false,
        status: "active".to_string(),
    };

    let mut cert = cert_repo.create(&cert_create).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to record cert: {e}"),
        )
    })?;

    let real_cert_path = format!("/etc/panel/certs/{}/fullchain.pem", cert.id);
    let real_key_path = format!("/etc/panel/certs/{}/privkey.pem", cert.id);

    cert = cert_repo
        .update(
            cert.id,
            &UpdateCertificate {
                fullchain_pem: None,
                privkey_pem: None,
                cert_path: Some(real_cert_path),
                key_path: Some(real_key_path),
                not_before: None,
                not_after: None,
                auto_renew: None,
                ocsp_stapling: None,
                last_renew_attempt: None,
                renew_error: None,
                status: None,
            },
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to update cert path: {e}"),
            )
        })?;

    deploy_and_activate_site_ssl(&state, &site, &user, &domains, &cert, false).await?;

    state.audit_writer.log_async(
        Some(auth_user.id),
        &auth_user.username,
        "site.ssl.self_signed",
        format!("site:{}", site.id),
        "success",
        None,
        Some(format!("domain:{primary_domain}")),
    );

    Ok((StatusCode::OK, Json(cert_to_detail_response(&cert))))
}

/// POST /api/v1/sites/{id}/ssl/custom - Upload custom certificate and private key.
#[utoipa::path(
    post,
    path = "/api/v1/sites/{id}/ssl/custom",
    tag = "ssl",
    params(
        ("id" = i64, Path, description = "Site ID")
    ),
    request_body = CustomCertRequest,
    responses(
        (status = 200, description = "Custom certificate uploaded successfully", body = CertificateDetailResponse),
        (status = 400, description = "Invalid certificate or mismatched key"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Site not found")
    )
)]
pub async fn custom_site_ssl_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
    Json(req): Json<CustomCertRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    crate::dns01::guard_site(&state.pool, site_id).await?;
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let site = fetch_and_authorize_site(&site_repo, &auth_user, site_id)
        .await
        .map_err(|(sc, msg)| (sc, msg.to_string()))?;

    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let user = user_repo
        .find_by_id(site.user_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database error: {e}"),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            "Site tenant user not found".to_string(),
        ))?;

    let domain_repo = SqliteDomainRepository::new(state.pool.clone());
    let domains = domain_repo.find_by_site_id(site.id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Database error: {e}"),
        )
    })?;

    let primary_domain = domains
        .iter()
        .find(|d| d.is_primary)
        .map(|d| d.name.clone())
        .ok_or((
            StatusCode::BAD_REQUEST,
            "Site has no primary domain".to_string(),
        ))?;

    // Parse and validate custom certificate
    let (not_before, not_after, issuer, sans) = inspect_x509_cert(&req.fullchain_pem)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid certificate: {e}")))?;

    // Check validity period
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    if not_after <= now {
        return Err((
            StatusCode::BAD_REQUEST,
            "Certificate is already expired".to_string(),
        ));
    }

    let cert_repo = SqliteCertificateRepository::new(state.pool.clone());
    let cert_create = CreateCertificate {
        site_id: site.id,
        primary_domain: primary_domain.clone(),
        san_domains_json: serde_json::to_string(&sans).unwrap_or_else(|_| "[]".to_string()),
        issuer: "custom".to_string(),
        cert_path: "/etc/panel/certs/placeholder/fullchain.pem".to_string(),
        key_path: "/etc/panel/certs/placeholder/privkey.pem".to_string(),
        fullchain_pem: req.fullchain_pem.clone(),
        privkey_pem: req.privkey_pem.clone(),
        not_before,
        not_after,
        auto_renew: req.auto_renew.unwrap_or(false),
        ocsp_stapling: false,
        status: "active".to_string(),
    };

    let mut cert = cert_repo.create(&cert_create).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to record cert: {e}"),
        )
    })?;

    let real_cert_path = format!("/etc/panel/certs/{}/fullchain.pem", cert.id);
    let real_key_path = format!("/etc/panel/certs/{}/privkey.pem", cert.id);

    cert = cert_repo
        .update(
            cert.id,
            &UpdateCertificate {
                fullchain_pem: None,
                privkey_pem: None,
                cert_path: Some(real_cert_path),
                key_path: Some(real_key_path),
                not_before: None,
                not_after: None,
                auto_renew: None,
                ocsp_stapling: None,
                last_renew_attempt: None,
                renew_error: None,
                status: None,
            },
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to update cert path: {e}"),
            )
        })?;

    deploy_and_activate_site_ssl(&state, &site, &user, &domains, &cert, false).await?;

    state.audit_writer.log_async(
        Some(auth_user.id),
        &auth_user.username,
        "site.ssl.custom",
        format!("site:{}", site.id),
        "success",
        None,
        Some(format!("domain:{primary_domain},issuer:{issuer}")),
    );

    Ok((StatusCode::OK, Json(cert_to_detail_response(&cert))))
}

/// POST /api/v1/sites/{id}/ssl/renew - Manually trigger renewal of the active certificate.
#[utoipa::path(
    post,
    path = "/api/v1/sites/{id}/ssl/renew",
    tag = "ssl",
    params(
        ("id" = i64, Path, description = "Site ID")
    ),
    responses(
        (status = 200, description = "Certificate renewed successfully", body = CertificateDetailResponse),
        (status = 400, description = "Certificate cannot be renewed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Active certificate not found")
    )
)]
pub async fn renew_site_ssl_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    crate::dns01::guard_site(&state.pool, site_id).await?;
    let dns_managed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM dns01_bindings b JOIN certificates c ON c.id=b.certificate_id JOIN sites s ON s.id=b.site_id WHERE b.site_id=? AND c.status='active' AND c.id=(SELECT MAX(id) FROM certificates WHERE site_id=b.site_id AND status='active') AND json_extract(s.settings_json,'$.has_ssl')=1 AND json_extract(s.settings_json,'$.ssl_cert')=c.cert_path)").bind(site_id).fetch_one(&state.pool).await.map_err(|_|(StatusCode::INTERNAL_SERVER_ERROR,"Inventaris DNS-01 gagal.".into()))?;
    if dns_managed {
        return Err((
            StatusCode::CONFLICT,
            "Perbarui sertifikat DNS-01 melalui Pengaturan server > DNS dan Cloudflare.".into(),
        ));
    }
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let site = fetch_and_authorize_site(&site_repo, &auth_user, site_id)
        .await
        .map_err(|(sc, msg)| (sc, msg.to_string()))?;

    let cert_repo = SqliteCertificateRepository::new(state.pool.clone());
    let existing_cert = cert_repo
        .find_active_by_site_id(site.id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database error: {e}"),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            "No active SSL certificate found for site".to_string(),
        ))?;

    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let user = user_repo
        .find_by_id(site.user_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database error: {e}"),
            )
        })?
        .ok_or((StatusCode::NOT_FOUND, "Tenant user not found".to_string()))?;

    let domain_repo = SqliteDomainRepository::new(state.pool.clone());
    let domains = domain_repo.find_by_site_id(site.id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Database error: {e}"),
        )
    })?;

    let primary_domain = domains
        .iter()
        .find(|d| d.is_primary)
        .map(|d| d.name.clone())
        .ok_or((
            StatusCode::BAD_REQUEST,
            "Site has no primary domain".to_string(),
        ))?;

    let san_domains: Vec<String> = domains
        .iter()
        .filter(|d| !d.is_primary)
        .map(|d| d.name.clone())
        .collect();

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let (fullchain, privkey, not_before, not_after) = if existing_cert
        .issuer
        .starts_with("letsencrypt")
    {
        let staging = existing_cert.issuer == "letsencrypt_staging";
        let email = user.email.as_deref().unwrap_or("admin@example.com");
        match order_acme_certificate(&state, &primary_domain, &san_domains, email, staging).await {
            Ok(res) => res,
            Err(err) => {
                let _ = cert_repo
                    .update(
                        existing_cert.id,
                        &UpdateCertificate {
                            fullchain_pem: None,
                            privkey_pem: None,
                            cert_path: None,
                            key_path: None,
                            not_before: None,
                            not_after: None,
                            auto_renew: None,
                            ocsp_stapling: None,
                            last_renew_attempt: Some(Some(now)),
                            renew_error: Some(Some(err.clone())),
                            status: None,
                        },
                    )
                    .await;
                return Err((
                    StatusCode::BAD_GATEWAY,
                    format!("ACME renewal failed: {err}"),
                ));
            }
        }
    } else if existing_cert.issuer == "self_signed" {
        generate_self_signed_cert(&primary_domain, &san_domains)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
    } else {
        return Err((
            StatusCode::BAD_REQUEST,
            "Custom certificates cannot be auto-renewed. Upload a new certificate instead."
                .to_string(),
        ));
    };

    let updated_cert = cert_repo
        .update(
            existing_cert.id,
            &UpdateCertificate {
                fullchain_pem: Some(fullchain),
                privkey_pem: Some(privkey),
                cert_path: None,
                key_path: None,
                not_before: Some(not_before),
                not_after: Some(not_after),
                auto_renew: None,
                ocsp_stapling: None,
                last_renew_attempt: Some(Some(now)),
                renew_error: Some(None),
                status: Some("active".to_string()),
            },
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to update renewed cert: {e}"),
            )
        })?;

    deploy_and_activate_site_ssl(
        &state,
        &site,
        &user,
        &domains,
        &updated_cert,
        updated_cert.ocsp_stapling == 1,
    )
    .await?;

    state.audit_writer.log_async(
        Some(auth_user.id),
        &auth_user.username,
        "site.ssl.renew",
        format!("site:{}", site.id),
        "success",
        None,
        Some(format!(
            "domain:{primary_domain},cert_id:{}",
            updated_cert.id
        )),
    );

    Ok((StatusCode::OK, Json(cert_to_detail_response(&updated_cert))))
}

/// DELETE /api/v1/sites/{id}/ssl - Disable SSL on the site and revert to HTTP.
#[utoipa::path(
    delete,
    path = "/api/v1/sites/{id}/ssl",
    tag = "ssl",
    params(
        ("id" = i64, Path, description = "Site ID")
    ),
    responses(
        (status = 200, description = "SSL disabled successfully", body = SslActionResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Site not found")
    )
)]
pub async fn delete_site_ssl_handler(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(site_id): Path<i64>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    crate::dns01::guard_site(&state.pool, site_id).await?;
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let site = fetch_and_authorize_site(&site_repo, &auth_user, site_id)
        .await
        .map_err(|(sc, msg)| (sc, msg.to_string()))?;

    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let user = user_repo
        .find_by_id(site.user_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database error: {e}"),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            "Site tenant user not found".to_string(),
        ))?;

    let domain_repo = SqliteDomainRepository::new(state.pool.clone());
    let domains = domain_repo.find_by_site_id(site.id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Database error: {e}"),
        )
    })?;

    deactivate_site_ssl(&state, &site, &user, &domains).await?;

    state.audit_writer.log_async(
        Some(auth_user.id),
        &auth_user.username,
        "site.ssl.delete",
        format!("site:{}", site.id),
        "success",
        None,
        Some(format!("site_id:{}", site.id)),
    );

    Ok((
        StatusCode::OK,
        Json(SslActionResponse {
            message: "SSL disabled and vhost reverted to HTTP".to_string(),
            certificate: None,
        }),
    ))
}

fn should_skip_renewal(cert: &Certificate, now: i64) -> bool {
    let days_remaining = (cert.not_after - now) / 86400;
    if days_remaining < 7 {
        warn!(
            "URGENT: Certificate #{} for {} expires in {} days!",
            cert.id, cert.primary_domain, days_remaining
        );
    }
    matches!(cert.last_renew_attempt, Some(last) if now - last < 3600)
}

async fn fetch_expiring_certificates(state: &AppState, now: i64) -> Vec<Certificate> {
    let cert_repo = SqliteCertificateRepository::new(state.pool.clone());
    match cert_repo.find_expiring_certificates(now, 30 * 86400).await {
        Ok(list) => list
            .into_iter()
            .filter(|cert| {
                now >= panel_core::panel_tls::renewal_at(cert.not_before, cert.not_after, None)
            })
            .collect(),
        Err(e) => {
            error!("Failed to fetch expiring certificates: {e}");
            Vec::new()
        }
    }
}

/// Helper function to process expiring certificates in background renewal scheduler.
pub async fn renew_expiring_certificates(state: &AppState) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let expiring = fetch_expiring_certificates(state, now).await;
    if expiring.is_empty() {
        return;
    }

    info!("Found {} certificates needing auto-renewal", expiring.len());

    for cert in expiring {
        let dns_managed: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM dns01_bindings WHERE certificate_id=?)",
        )
        .bind(cert.id)
        .fetch_one(&state.pool)
        .await
        .unwrap_or(true);
        if dns_managed
            || crate::dns01::guard_site(&state.pool, cert.site_id)
                .await
                .is_err()
            || should_skip_renewal(&cert, now)
        {
            continue;
        }
        if let Err(e) = renew_single_certificate(state, &cert, now).await {
            error!("Renewal failed for certificate #{}: {e}", cert.id);
        }
    }
}

async fn renew_single_certificate(
    state: &AppState,
    cert: &Certificate,
    now: i64,
) -> Result<(), String> {
    let site_repo = SqliteSiteRepository::new(state.pool.clone());
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let domain_repo = SqliteDomainRepository::new(state.pool.clone());
    let cert_repo = SqliteCertificateRepository::new(state.pool.clone());

    let site = site_repo
        .find_by_id(cert.site_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Site {} not found", cert.site_id))?;

    let user = user_repo
        .find_by_id(site.user_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("User {} not found", site.user_id))?;

    let domains = domain_repo
        .find_by_site_id(site.id)
        .await
        .map_err(|e| e.to_string())?;

    let san_domains: Vec<String> = serde_json::from_str(&cert.san_domains_json).unwrap_or_default();
    let staging = cert.issuer == "letsencrypt_staging";
    let email = user.email.as_deref().unwrap_or("admin@example.com");

    match order_acme_certificate(state, &cert.primary_domain, &san_domains, email, staging).await {
        Ok((fullchain, privkey, not_before, not_after)) => {
            info!(
                "Successfully renewed certificate #{} for {}",
                cert.id, cert.primary_domain
            );
            let updated_cert = cert_repo
                .update(
                    cert.id,
                    &UpdateCertificate {
                        fullchain_pem: Some(fullchain),
                        privkey_pem: Some(privkey),
                        cert_path: None,
                        key_path: None,
                        not_before: Some(not_before),
                        not_after: Some(not_after),
                        auto_renew: None,
                        ocsp_stapling: None,
                        last_renew_attempt: Some(Some(now)),
                        renew_error: Some(None),
                        status: Some("active".to_string()),
                    },
                )
                .await
                .map_err(|e| e.to_string())?;

            deploy_and_activate_site_ssl(
                state,
                &site,
                &user,
                &domains,
                &updated_cert,
                updated_cert.ocsp_stapling == 1,
            )
            .await
            .map_err(|(_, msg)| msg)?;
            Ok(())
        }
        Err(e) => {
            let _ = cert_repo
                .update(
                    cert.id,
                    &UpdateCertificate {
                        fullchain_pem: None,
                        privkey_pem: None,
                        cert_path: None,
                        key_path: None,
                        not_before: None,
                        not_after: None,
                        auto_renew: None,
                        ocsp_stapling: None,
                        last_renew_attempt: Some(Some(now)),
                        renew_error: Some(Some(e.clone())),
                        status: None,
                    },
                )
                .await;
            Err(e)
        }
    }
}

/// Checks panel renewal every five minutes and site renewal every hour.
pub fn start_ssl_renewal_scheduler(state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Initial delay on startup
        tokio::time::sleep(Duration::from_secs(10)).await;

        let mut interval = tokio::time::interval(Duration::from_secs(300));
        let mut last_site_check = 0i64;
        loop {
            interval.tick().await;
            crate::panel_tls::schedule(&state).await;
            crate::dns01::schedule(&state).await;
            crate::storage::tls::schedule(&state).await;
            let current = chrono::Utc::now().timestamp();
            if current - last_site_check >= 3600 {
                renew_expiring_certificates(&state).await;
                last_site_check = current;
            }
        }
    })
}
