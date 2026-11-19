ALTER TABLE monitor_definitions ADD COLUMN interface_name TEXT;

UPDATE monitor_definitions
SET enabled = 0,
    updated_at = datetime('now')
WHERE interface_name IS NULL;
