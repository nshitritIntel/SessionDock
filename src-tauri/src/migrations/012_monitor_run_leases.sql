CREATE TABLE IF NOT EXISTS monitor_run_leases (
    session_id TEXT PRIMARY KEY NOT NULL,
    monitor_id TEXT NOT NULL,
    started_at TEXT NOT NULL,
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
    FOREIGN KEY (monitor_id) REFERENCES monitor_definitions(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_monitor_run_leases_started_at
    ON monitor_run_leases(started_at);