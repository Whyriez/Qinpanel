//! Certificate repository for SSL/TLS and ACME certificate management.

use crate::error::DbError;
use crate::models::{
    AcmeAccount, Certificate, CreateAcmeAccount, CreateCertificate, UpdateCertificate,
};
use crate::pool::DbPool;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct SqliteCertificateRepository {
    pool: DbPool,
}

impl SqliteCertificateRepository {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Stores a new SSL certificate record.
    pub async fn create(&self, input: &CreateCertificate) -> Result<Certificate, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let auto_renew_val = if input.auto_renew { 1i64 } else { 0i64 };
        let ocsp_stapling_val = if input.ocsp_stapling { 1i64 } else { 0i64 };

        let res = sqlx::query(
            r#"
            INSERT INTO certificates (
                site_id, primary_domain, san_domains_json, issuer,
                cert_path, key_path, fullchain_pem, privkey_pem,
                not_before, not_after, auto_renew, ocsp_stapling,
                status, created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(input.site_id)
        .bind(&input.primary_domain)
        .bind(&input.san_domains_json)
        .bind(&input.issuer)
        .bind(&input.cert_path)
        .bind(&input.key_path)
        .bind(&input.fullchain_pem)
        .bind(&input.privkey_pem)
        .bind(input.not_before)
        .bind(input.not_after)
        .bind(auto_renew_val)
        .bind(ocsp_stapling_val)
        .bind(&input.status)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        let id = res.last_insert_rowid();

        self.find_by_id(id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("Certificate {id} not found after insert")))
    }

    /// Finds a certificate by ID.
    pub async fn find_by_id(&self, id: i64) -> Result<Option<Certificate>, DbError> {
        sqlx::query_as::<_, Certificate>(
            r#"
            SELECT * FROM certificates WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Sqlx)
    }

    /// Finds the currently active certificate for a site.
    pub async fn find_active_by_site_id(
        &self,
        site_id: i64,
    ) -> Result<Option<Certificate>, DbError> {
        sqlx::query_as::<_, Certificate>(
            r#"
            SELECT * FROM certificates
            WHERE site_id = ? AND status = 'active'
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .bind(site_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Sqlx)
    }

    /// Finds all certificates expiring within threshold seconds from `now_timestamp`.
    pub async fn find_expiring_certificates(
        &self,
        now_timestamp: i64,
        threshold_seconds: i64,
    ) -> Result<Vec<Certificate>, DbError> {
        let deadline = now_timestamp + threshold_seconds;
        sqlx::query_as::<_, Certificate>(
            r#"
            SELECT * FROM certificates
            WHERE auto_renew = 1
              AND status = 'active'
              AND issuer IN ('letsencrypt', 'letsencrypt_staging')
              AND not_after <= ?
            ORDER BY not_after ASC
            "#,
        )
        .bind(deadline)
        .fetch_all(&self.pool)
        .await
        .map_err(DbError::Sqlx)
    }

    /// Updates certificate fields (e.g. renewal, renewal error, status).
    pub async fn update(&self, id: i64, input: &UpdateCertificate) -> Result<Certificate, DbError> {
        let existing = self
            .find_by_id(id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("Certificate {id} not found")))?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let fullchain = input
            .fullchain_pem
            .as_deref()
            .unwrap_or(&existing.fullchain_pem);
        let privkey = input
            .privkey_pem
            .as_deref()
            .unwrap_or(&existing.privkey_pem);
        let cert_path = input.cert_path.as_deref().unwrap_or(&existing.cert_path);
        let key_path = input.key_path.as_deref().unwrap_or(&existing.key_path);
        let not_before = input.not_before.unwrap_or(existing.not_before);
        let not_after = input.not_after.unwrap_or(existing.not_after);
        let auto_renew = input
            .auto_renew
            .map(|b| if b { 1i64 } else { 0i64 })
            .unwrap_or(existing.auto_renew);
        let ocsp_stapling = input
            .ocsp_stapling
            .map(|b| if b { 1i64 } else { 0i64 })
            .unwrap_or(existing.ocsp_stapling);
        let status = input.status.as_deref().unwrap_or(&existing.status);

        let last_renew_attempt = match input.last_renew_attempt {
            Some(opt) => opt,
            None => existing.last_renew_attempt,
        };

        let renew_error = match &input.renew_error {
            Some(opt) => opt.clone(),
            None => existing.renew_error,
        };

        sqlx::query(
            r#"
            UPDATE certificates
            SET fullchain_pem = ?, privkey_pem = ?, cert_path = ?, key_path = ?,
                not_before = ?, not_after = ?, auto_renew = ?, ocsp_stapling = ?,
                last_renew_attempt = ?, renew_error = ?, status = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(fullchain)
        .bind(privkey)
        .bind(cert_path)
        .bind(key_path)
        .bind(not_before)
        .bind(not_after)
        .bind(auto_renew)
        .bind(ocsp_stapling)
        .bind(last_renew_attempt)
        .bind(renew_error)
        .bind(status)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        self.find_by_id(id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("Certificate {id} not found after update")))
    }

    /// Deletes a certificate record by ID.
    pub async fn delete(&self, id: i64) -> Result<(), DbError> {
        let res = sqlx::query("DELETE FROM certificates WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(DbError::Sqlx)?;

        if res.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("Certificate {id} not found")));
        }
        Ok(())
    }

    /// Deletes all certificates associated with a site.
    pub async fn delete_by_site_id(&self, site_id: i64) -> Result<(), DbError> {
        sqlx::query("DELETE FROM certificates WHERE site_id = ?")
            .bind(site_id)
            .execute(&self.pool)
            .await
            .map_err(DbError::Sqlx)?;
        Ok(())
    }

    /// Finds or creates an ACME account by email and directory URL.
    pub async fn get_or_create_acme_account(
        &self,
        input: &CreateAcmeAccount,
    ) -> Result<AcmeAccount, DbError> {
        let existing = sqlx::query_as::<_, AcmeAccount>(
            r#"
            SELECT * FROM acme_accounts
            WHERE email = ? AND directory_url = ?
            LIMIT 1
            "#,
        )
        .bind(&input.email)
        .bind(&input.directory_url)
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        if let Some(account) = existing {
            return Ok(account);
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let res = sqlx::query(
            r#"
            INSERT INTO acme_accounts (
                email, directory_url, account_uri, private_key_pem,
                created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&input.email)
        .bind(&input.directory_url)
        .bind(&input.account_uri)
        .bind(&input.private_key_pem)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        let id = res.last_insert_rowid();

        sqlx::query_as::<_, AcmeAccount>("SELECT * FROM acme_accounts WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(DbError::Sqlx)
    }
}
