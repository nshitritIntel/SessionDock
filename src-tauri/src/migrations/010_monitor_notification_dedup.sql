ALTER TABLE monitor_notification_state ADD COLUMN last_incident_at TEXT;

UPDATE monitor_notification_state
SET last_incident_at = last_notified_at
WHERE last_incident_at IS NULL;