-- Compatibility schema for single-pool/in-memory clients; production writes use operations.db.
CREATE TABLE job_events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    status TEXT NOT NULL,
    progress INTEGER NOT NULL,
    message TEXT,
    timestamp INTEGER NOT NULL
);
CREATE INDEX idx_job_events_job_sequence ON job_events(job_id, sequence);
