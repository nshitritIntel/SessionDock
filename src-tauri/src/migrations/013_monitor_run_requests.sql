ALTER TABLE monitor_definitions ADD COLUMN last_started_at TEXT;

CREATE TABLE IF NOT EXISTS monitor_run_requests (
    monitor_id TEXT PRIMARY KEY NOT NULL,
    requested_at TEXT NOT NULL,
    FOREIGN KEY (monitor_id) REFERENCES monitor_definitions(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_monitor_run_requests_requested_at
    ON monitor_run_requests(requested_at);