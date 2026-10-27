-- Additive, disabled by default. Existing plans, settings and secrets are unchanged.
CREATE TABLE ai_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    enabled INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
    provider TEXT NOT NULL DEFAULT 'gemini' CHECK (provider IN ('gemini', 'openai')),
    model TEXT NOT NULL DEFAULT 'gemini-3.8-flash',
    daily_request_limit INTEGER NOT NULL DEFAULT 20 CHECK (daily_request_limit BETWEEN 1 AND 1000),
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0)
);
INSERT INTO ai_settings (id) VALUES (1);

CREATE TABLE ai_credentials (
    provider TEXT PRIMARY KEY CHECK (provider IN ('gemini', 'openai')),
    key_enc TEXT NOT NULL CHECK (key_enc LIKE 'enc:v1:%'),
    revision INTEGER NOT NULL CHECK (revision > 0)
);

CREATE TABLE ai_requests (
    id TEXT PRIMARY KEY,
    actor_id INTEGER NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    settings_revision INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    completed_at INTEGER,
    state TEXT NOT NULL CHECK (state IN ('running', 'completed', 'failed')),
    total_tokens INTEGER CHECK (total_tokens IS NULL OR total_tokens >= 0)
);
CREATE INDEX ai_requests_created_at ON ai_requests (created_at);
