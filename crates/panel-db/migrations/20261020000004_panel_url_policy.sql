-- Explicit activation; metadata edits do not redirect or lock out existing users.
CREATE TABLE panel_url_policy (
    id INTEGER PRIMARY KEY CHECK(id=1),
    revision INTEGER NOT NULL CHECK(revision>0),
    canonical_url TEXT NOT NULL,
    endpoint_id TEXT NOT NULL,
    endpoint_revision INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE panel_url_aliases (
    origin TEXT PRIMARY KEY NOT NULL
);
