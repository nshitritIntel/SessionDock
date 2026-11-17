CREATE TABLE IF NOT EXISTS monitor_definitions (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    monitor_kind TEXT NOT NULL,
    display_name TEXT NOT NULL,
    interval_seconds INTEGER NOT NULL CHECK(interval_seconds BETWEEN 30 AND 86400),
    retention_hours INTEGER NOT NULL DEFAULT 168 CHECK(retention_hours BETWEEN 24 AND 8760),
    enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
    UNIQUE(session_id, provider_id, monitor_kind)
);

CREATE TABLE IF NOT EXISTS monitor_samples (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    monitor_id TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('healthy', 'warning', 'critical', 'unknown', 'failed')),
    duration_ms INTEGER NOT NULL CHECK(duration_ms >= 0),
    summary_json TEXT NOT NULL DEFAULT '{}',
    raw_output TEXT,
    error TEXT,
    FOREIGN KEY (monitor_id) REFERENCES monitor_definitions(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS interface_samples (
    sample_id INTEGER NOT NULL,
    interface_name TEXT NOT NULL,
    admin_status TEXT,
    oper_status TEXT,
    speed_bps INTEGER,
    input_rate_bps INTEGER,
    output_rate_bps INTEGER,
    input_errors INTEGER,
    output_errors INTEGER,
    input_discards INTEGER,
    output_discards INTEGER,
    neighbor TEXT,
    transceiver_status TEXT,
    PRIMARY KEY (sample_id, interface_name),
    FOREIGN KEY (sample_id) REFERENCES monitor_samples(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS monitor_incidents (
    id TEXT PRIMARY KEY NOT NULL,
    monitor_id TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    severity TEXT NOT NULL CHECK(severity IN ('info', 'warning', 'critical')),
    state TEXT NOT NULL CHECK(state IN ('open', 'acknowledged', 'resolved')),
    title TEXT NOT NULL,
    details TEXT,
    started_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    resolved_at TEXT,
    occurrence_count INTEGER NOT NULL DEFAULT 1 CHECK(occurrence_count > 0),
    FOREIGN KEY (monitor_id) REFERENCES monitor_definitions(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS monitor_notification_state (
    fingerprint TEXT PRIMARY KEY NOT NULL,
    incident_id TEXT NOT NULL,
    last_notified_at TEXT NOT NULL,
    notification_count INTEGER NOT NULL DEFAULT 1 CHECK(notification_count > 0),
    suppressed_count INTEGER NOT NULL DEFAULT 0 CHECK(suppressed_count >= 0),
    FOREIGN KEY (incident_id) REFERENCES monitor_incidents(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS monitor_baselines (
    id TEXT PRIMARY KEY NOT NULL,
    monitor_id TEXT NOT NULL,
    name TEXT NOT NULL,
    sample_id INTEGER NOT NULL,
    captured_at TEXT NOT NULL,
    FOREIGN KEY (monitor_id) REFERENCES monitor_definitions(id) ON DELETE CASCADE,
    FOREIGN KEY (sample_id) REFERENCES monitor_samples(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS maintenance_markers (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT,
    starts_at TEXT NOT NULL,
    ends_at TEXT NOT NULL,
    reason TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    CHECK(ends_at > starts_at),
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_monitor_definitions_enabled ON monitor_definitions(enabled, session_id);
CREATE INDEX IF NOT EXISTS idx_monitor_samples_monitor_time ON monitor_samples(monitor_id, observed_at DESC);
CREATE INDEX IF NOT EXISTS idx_interface_samples_name ON interface_samples(interface_name, sample_id DESC);
CREATE INDEX IF NOT EXISTS idx_monitor_incidents_state_time ON monitor_incidents(state, last_seen_at DESC);
CREATE UNIQUE INDEX IF NOT EXISTS idx_monitor_incidents_open_fingerprint ON monitor_incidents(monitor_id, fingerprint) WHERE state != 'resolved';
CREATE INDEX IF NOT EXISTS idx_monitor_baselines_monitor ON monitor_baselines(monitor_id, captured_at DESC);
CREATE INDEX IF NOT EXISTS idx_maintenance_markers_window ON maintenance_markers(starts_at, ends_at);