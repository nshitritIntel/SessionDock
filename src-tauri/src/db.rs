use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use crate::error::AppError;

pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    pub fn new(path: &Path) -> Result<Self, AppError> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.run_migrations()?;
        Ok(db)
    }

    pub fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().expect("Database mutex poisoned")
    }

    fn run_migrations(&self) -> Result<(), AppError> {
        let conn = self.conn();

        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            ",
        )?;

        let current_version: i32 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 1 {
            conn.execute_batch(include_str!("migrations/001_initial.sql"))?;
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                params![1],
            )?;
        }

        if current_version < 2 {
            conn.execute_batch(include_str!("migrations/002_snippets.sql"))?;
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                params![2],
            )?;
        }

        if current_version < 3 {
            add_bmc_columns(&conn)?;
            conn.execute_batch(include_str!("migrations/003_bmc_sessions.sql"))?;
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                params![3],
            )?;
        }

        if current_version < 4 {
            add_serial_columns(&conn)?;
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                params![4],
            )?;
        }

        if current_version < 5 {
            conn.execute_batch(include_str!("migrations/005_jump_hosts.sql"))?;
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                params![5],
            )?;
        }

        if current_version < 6 {
            add_reconnect_columns(&conn)?;
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                params![6],
            )?;
        }

        if current_version < 7 {
            conn.execute_batch(include_str!("migrations/007_session_groups.sql"))?;
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                params![7],
            )?;
        }

        if current_version < 8 {
            conn.execute_batch(include_str!("migrations/008_monitoring.sql"))?;
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                params![8],
            )?;
        }

        if current_version < 9 {
            conn.execute_batch(include_str!(
                "migrations/009_monitor_baseline_snapshots.sql"
            ))?;
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                params![9],
            )?;
        }

        if current_version < 10 {
            conn.execute_batch(include_str!(
                "migrations/010_monitor_notification_dedup.sql"
            ))?;
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                params![10],
            )?;
        }

        if current_version < 11 {
            conn.execute_batch(include_str!("migrations/011_monitor_interface_scope.sql"))?;
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                params![11],
            )?;
        }

        if current_version < 12 {
            conn.execute_batch(include_str!("migrations/012_monitor_run_leases.sql"))?;
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                params![12],
            )?;
        }

        if current_version < 13 {
            conn.execute_batch(include_str!("migrations/013_monitor_run_requests.sql"))?;
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                params![13],
            )?;
        }

        log::info!(
            "Database migrations complete. Version: {}",
            std::cmp::max(current_version, 13)
        );
        Ok(())
    }
}

fn add_reconnect_columns(conn: &Connection) -> Result<(), AppError> {
    let exists = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('sessions') WHERE name = 'reconnect_mode')",
        [],
        |row| row.get::<_, bool>(0),
    )?;
    if !exists {
        conn.execute_batch(
            "ALTER TABLE sessions ADD COLUMN reconnect_mode TEXT NOT NULL DEFAULT 'disabled';",
        )?;
    }
    Ok(())
}

fn add_serial_columns(conn: &Connection) -> Result<(), AppError> {
    let exists = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('sessions') WHERE name = 'serial_baud_rate')",
        [],
        |row| row.get::<_, bool>(0),
    )?;
    if !exists {
        conn.execute_batch(
            "ALTER TABLE sessions ADD COLUMN serial_baud_rate INTEGER NOT NULL DEFAULT 9600;",
        )?;
    }
    Ok(())
}

fn add_bmc_columns(conn: &Connection) -> Result<(), AppError> {
    let columns = [
        ("bmc_use_https", "INTEGER NOT NULL DEFAULT 1"),
        ("bmc_web_path", "TEXT"),
        ("bmc_console_url", "TEXT"),
        ("bmc_viewer_mode", "TEXT NOT NULL DEFAULT 'web'"),
        ("bmc_ignore_tls_errors", "INTEGER NOT NULL DEFAULT 0"),
        (
            "bmc_open_console_automatically",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("bmc_open_fullscreen", "INTEGER NOT NULL DEFAULT 0"),
        ("bmc_timeout_seconds", "INTEGER NOT NULL DEFAULT 30"),
        ("bmc_server_hostname", "TEXT"),
        ("bmc_server_serial_number", "TEXT"),
        ("bmc_rack", "TEXT"),
        ("bmc_rack_unit", "TEXT"),
        ("bmc_site", "TEXT"),
        ("bmc_redfish_enabled", "INTEGER NOT NULL DEFAULT 1"),
        (
            "bmc_cookie_persistence",
            "TEXT NOT NULL DEFAULT 'application'",
        ),
    ];

    for (name, definition) in columns {
        let exists = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('sessions') WHERE name = ?1)",
            params![name],
            |row| row.get::<_, bool>(0),
        )?;
        if !exists {
            conn.execute_batch(&format!(
                "ALTER TABLE sessions ADD COLUMN {name} {definition};"
            ))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Database;
    use rusqlite::Connection;

    #[test]
    fn migrates_v2_database_without_changing_existing_session() {
        let path = std::env::temp_dir().join(format!(
            "sessiondock-bmc-migration-{}.db",
            uuid::Uuid::new_v4()
        ));
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(include_str!("migrations/001_initial.sql"))
            .unwrap();
        conn.execute_batch(include_str!("migrations/002_snippets.sql"))
            .unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT (datetime('now')));
             INSERT INTO schema_version(version) VALUES (1), (2);
             INSERT INTO sessions (id, name, host, port, protocol, authentication_method, created_at, updated_at)
             VALUES ('legacy-session', 'Legacy SSH', '192.0.2.10', 22, 'ssh', 'password', datetime('now'), datetime('now'));",
        ).unwrap();
        drop(conn);

        let db = Database::new(&path).unwrap();
        let conn = db.conn();
        let row: (String, String, i64, String) = conn.query_row(
            "SELECT name, protocol, bmc_use_https, bmc_viewer_mode FROM sessions WHERE id = 'legacy-session'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).unwrap();
        assert_eq!(row, ("Legacy SSH".into(), "ssh".into(), 1, "web".into()));
        assert_eq!(
            conn.query_row("SELECT MAX(version) FROM schema_version", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            13
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'monitor_run_requests'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            1
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM session_jump_hosts", [], |row| row
                .get::<_, i64>(0),)
                .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row(
                "SELECT serial_baud_rate FROM sessions WHERE id = 'legacy-session'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            9600
        );
        assert_eq!(
            conn.query_row(
                "SELECT reconnect_mode FROM sessions WHERE id = 'legacy-session'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
            "disabled"
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM session_groups", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM session_group_members", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM monitor_definitions", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM monitor_samples", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        conn.execute(
            "UPDATE sessions SET protocol = 'bmc', bmc_site = 'North Campus', bmc_server_serial_number = 'SN-700' WHERE id = 'legacy-session'",
            [],
        ).unwrap();
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM sessions_fts WHERE sessions_fts MATCH 'North'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            1
        );
        conn.execute(
            "INSERT INTO session_groups (id, name) VALUES ('legacy-group', 'Legacy Group')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_group_members (group_id, session_id) VALUES ('legacy-group', 'legacy-session')",
            [],
        )
        .unwrap();
        conn.execute("DELETE FROM sessions WHERE id = 'legacy-session'", [])
            .unwrap();
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM session_group_members", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            0
        );
        drop(conn);
        drop(db);
        let reopened = Database::new(&path).unwrap();
        assert_eq!(
            reopened
                .conn()
                .query_row(
                    "SELECT COUNT(*) FROM schema_version WHERE version = 7",
                    [],
                    |row| { row.get::<_, i64>(0) }
                )
                .unwrap(),
            1
        );
        drop(reopened);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn migrates_v8_baseline_to_snapshot_that_survives_sample_retention() {
        let path = std::env::temp_dir().join(format!(
            "sessiondock-baseline-migration-{}.db",
            uuid::Uuid::new_v4()
        ));
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(include_str!("migrations/001_initial.sql"))
            .unwrap();
        conn.execute_batch(include_str!("migrations/008_monitoring.sql"))
            .unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT (datetime('now')));
             INSERT INTO schema_version(version) VALUES (8);
             INSERT INTO sessions (id, name, host, protocol, authentication_method, created_at, updated_at)
             VALUES ('session-1', 'Leaf', '192.0.2.10', 'ssh', 'agent', datetime('now'), datetime('now'));
             INSERT INTO monitor_definitions (id, session_id, provider_id, monitor_kind, display_name, interval_seconds)
             VALUES ('monitor-1', 'session-1', 'arista-eos', 'interfaces', 'Interfaces', 60);
             INSERT INTO monitor_samples (id, monitor_id, observed_at, status, duration_ms, summary_json)
             VALUES (42, 'monitor-1', '2026-01-01T00:00:00Z', 'healthy', 10, '{\"interfaceCount\":2}');
             INSERT INTO monitor_baselines (id, monitor_id, name, sample_id, captured_at)
             VALUES ('baseline-1', 'monitor-1', 'Known good', 42, '2026-01-01T00:01:00Z');",
        )
        .unwrap();
        drop(conn);

        let db = Database::new(&path).unwrap();
        let conn = db.conn();
        conn.execute("DELETE FROM monitor_samples WHERE id = 42", [])
            .unwrap();
        let baseline: (String, String) = conn
            .query_row(
                "SELECT snapshot_json, source_observed_at FROM monitor_baselines WHERE id = 'baseline-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            baseline,
            (
                "{\"interfaceCount\":2}".into(),
                "2026-01-01T00:00:00Z".into()
            )
        );
        assert_eq!(
            conn.query_row("SELECT MAX(version) FROM schema_version", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            13
        );
        assert_eq!(
            conn.query_row(
                "SELECT enabled FROM monitor_definitions WHERE id = 'monitor-1'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            0
        );

        drop(conn);
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}
