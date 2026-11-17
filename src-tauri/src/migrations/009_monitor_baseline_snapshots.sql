ALTER TABLE monitor_baselines RENAME TO monitor_baselines_legacy;

CREATE TABLE monitor_baselines (
    id TEXT PRIMARY KEY NOT NULL,
    monitor_id TEXT NOT NULL,
    name TEXT NOT NULL,
    snapshot_json TEXT NOT NULL,
    source_observed_at TEXT NOT NULL,
    captured_at TEXT NOT NULL,
    FOREIGN KEY (monitor_id) REFERENCES monitor_definitions(id) ON DELETE CASCADE
);

INSERT INTO monitor_baselines (id, monitor_id, name, snapshot_json, source_observed_at, captured_at)
SELECT b.id, b.monitor_id, b.name, s.summary_json, s.observed_at, b.captured_at
FROM monitor_baselines_legacy b
JOIN monitor_samples s ON s.id = b.sample_id;

DROP TABLE monitor_baselines_legacy;

CREATE INDEX idx_monitor_baselines_monitor
ON monitor_baselines(monitor_id, captured_at DESC);