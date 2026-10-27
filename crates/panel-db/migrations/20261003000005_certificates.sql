-- Nusa Panel Database Migration: 20261003000005_certificates.sql

-- 1. Certificates Table
CREATE TABLE IF NOT EXISTS certificates (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    site_id INTEGER NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    primary_domain TEXT NOT NULL,
    san_domains_json TEXT NOT NULL DEFAULT '[]',
    issuer TEXT NOT NULL CHECK(issuer IN ('letsencrypt', 'letsencrypt_staging', 'self_signed', 'custom')),
    cert_path TEXT NOT NULL,
    key_path TEXT NOT NULL,
    fullchain_pem TEXT NOT NULL,
    privkey_pem TEXT NOT NULL,
    not_before INTEGER NOT NULL,
    not_after INTEGER NOT NULL,
    auto_renew INTEGER NOT NULL DEFAULT 1 CHECK(auto_renew IN (0, 1)),
    ocsp_stapling INTEGER NOT NULL DEFAULT 0 CHECK(ocsp_stapling IN (0, 1)),
    last_renew_attempt INTEGER,
    renew_error TEXT,
    status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active', 'renewing', 'expired', 'failed', 'revoked')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_certificates_site_id ON certificates(site_id);
CREATE INDEX IF NOT EXISTS idx_certificates_primary_domain ON certificates(primary_domain);
CREATE INDEX IF NOT EXISTS idx_certificates_not_after ON certificates(not_after);
CREATE INDEX IF NOT EXISTS idx_certificates_status ON certificates(status);

-- 2. ACME Accounts Table
CREATE TABLE IF NOT EXISTS acme_accounts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    email TEXT NOT NULL,
    directory_url TEXT NOT NULL,
    account_uri TEXT,
    private_key_pem TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_acme_accounts_email ON acme_accounts(email);
