use chrono::{DateTime, Duration, Utc};
use rusqlite::params;

use crate::{AppError, Database};

const NOTIFICATION_COOLDOWN: Duration = Duration::hours(1);

#[derive(Debug)]
struct IncidentNotification {
    state_key: String,
    incident_id: String,
    title: String,
    body: String,
    incident_at: String,
    previous_notified_at: Option<String>,
}

pub(crate) fn dispatch_pending(database: &Database) -> Result<(), AppError> {
    for notification in pending_notifications(database)? {
        if notification_is_due(&notification)? {
            match show_native(&notification.title, &notification.body) {
                Ok(()) => record_notified(database, &notification)?,
                Err(error) => log::warn!("Could not display monitor notification: {error}"),
            }
        } else {
            record_suppressed(database, &notification)?;
        }
    }
    Ok(())
}

fn pending_notifications(database: &Database) -> Result<Vec<IncidentNotification>, AppError> {
    let conn = database.conn();
    let mut statement = conn.prepare(
        "SELECT i.id, i.monitor_id || ':' || i.fingerprint, i.title,
                m.display_name || CASE WHEN i.details IS NULL THEN '' ELSE ': ' || i.details END,
                i.last_seen_at, n.last_notified_at
         FROM monitor_incidents i
         JOIN monitor_definitions m ON m.id = i.monitor_id
         LEFT JOIN monitor_notification_state n
           ON n.fingerprint = i.monitor_id || ':' || i.fingerprint
         WHERE i.state != 'resolved'
           AND (n.last_incident_at IS NULL OR julianday(i.last_seen_at) > julianday(n.last_incident_at))
           AND NOT EXISTS (
               SELECT 1 FROM maintenance_markers maintenance
               WHERE (maintenance.session_id IS NULL OR maintenance.session_id = m.session_id)
                 AND julianday('now') BETWEEN julianday(maintenance.starts_at) AND julianday(maintenance.ends_at)
           )
         ORDER BY i.last_seen_at, i.id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(IncidentNotification {
            incident_id: row.get(0)?,
            state_key: row.get(1)?,
            title: row.get(2)?,
            body: row.get(3)?,
            incident_at: row.get(4)?,
            previous_notified_at: row.get(5)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(AppError::Database)
}

fn notification_is_due(notification: &IncidentNotification) -> Result<bool, AppError> {
    let Some(previous) = &notification.previous_notified_at else {
        return Ok(true);
    };
    let previous = DateTime::parse_from_rfc3339(previous)
        .map_err(|_| AppError::Validation("Stored notification timestamp is invalid".into()))?
        .with_timezone(&Utc);
    Ok(Utc::now() - previous >= NOTIFICATION_COOLDOWN)
}

fn record_notified(
    database: &Database,
    notification: &IncidentNotification,
) -> Result<(), AppError> {
    database.conn().execute(
        "INSERT INTO monitor_notification_state
         (fingerprint, incident_id, last_notified_at, notification_count, suppressed_count, last_incident_at)
         VALUES (?1, ?2, ?3, 1, 0, ?4)
         ON CONFLICT(fingerprint) DO UPDATE SET
           incident_id = excluded.incident_id,
           last_notified_at = excluded.last_notified_at,
           notification_count = monitor_notification_state.notification_count + 1,
           last_incident_at = excluded.last_incident_at",
        params![
            notification.state_key,
            notification.incident_id,
            Utc::now().to_rfc3339(),
            notification.incident_at
        ],
    )?;
    Ok(())
}

fn record_suppressed(
    database: &Database,
    notification: &IncidentNotification,
) -> Result<(), AppError> {
    database.conn().execute(
        "UPDATE monitor_notification_state
         SET incident_id = ?1, suppressed_count = suppressed_count + 1, last_incident_at = ?2
         WHERE fingerprint = ?3",
        params![
            notification.incident_id,
            notification.incident_at,
            notification.state_key
        ],
    )?;
    Ok(())
}

#[cfg(any(windows, target_os = "macos"))]
fn show_native(title: &str, body: &str) -> Result<(), AppError> {
    notify_rust::Notification::new()
        .appname("SessionDock")
        .summary(title)
        .body(body)
        .show()
        .map(|_| ())
        .map_err(|error| AppError::Generic(format!("Native notification failed: {error}")))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn show_native(_title: &str, _body: &str) -> Result<(), AppError> {
    Err(AppError::Generic(
        "Native monitor notifications are unsupported on this platform".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::{pending_notifications, record_notified, record_suppressed};
    use crate::Database;
    use chrono::Utc;
    use rusqlite::params;

    #[test]
    fn deduplicates_occurrences_and_suppresses_during_maintenance() {
        let path = std::env::temp_dir().join(format!(
            "sessiondock-monitor-alerts-{}.db",
            uuid::Uuid::new_v4()
        ));
        let db = Database::new(&path).unwrap();
        let session_id = uuid::Uuid::new_v4().to_string();
        let monitor_id = uuid::Uuid::new_v4().to_string();
        let incident_id = uuid::Uuid::new_v4().to_string();
        {
            let conn = db.conn();
            conn.execute(
                "INSERT INTO sessions (id, name, host, protocol, authentication_method, created_at, updated_at)
                 VALUES (?1, 'Leaf', '192.0.2.10', 'ssh', 'agent', datetime('now'), datetime('now'))",
                params![session_id],
            ).unwrap();
            conn.execute(
                "INSERT INTO monitor_definitions (id, session_id, provider_id, monitor_kind, display_name, interval_seconds)
                 VALUES (?1, ?2, 'arista-eos', 'interfaces', 'Leaf interfaces', 60)",
                params![monitor_id, session_id],
            ).unwrap();
            conn.execute(
                "INSERT INTO monitor_incidents
                 (id, monitor_id, fingerprint, severity, state, title, started_at, last_seen_at)
                 VALUES (?1, ?2, 'interfaces-down', 'critical', 'open', 'Interfaces down', ?3, ?3)",
                params![incident_id, monitor_id, Utc::now().to_rfc3339()],
            )
            .unwrap();
        }

        let pending = pending_notifications(&db).unwrap();
        assert_eq!(pending.len(), 1);
        record_notified(&db, &pending[0]).unwrap();
        assert!(pending_notifications(&db).unwrap().is_empty());

        db.conn()
            .execute(
                "UPDATE monitor_incidents SET last_seen_at = ?1 WHERE id = ?2",
                params![
                    (Utc::now() + chrono::Duration::minutes(1)).to_rfc3339(),
                    incident_id
                ],
            )
            .unwrap();
        let pending = pending_notifications(&db).unwrap();
        assert_eq!(pending.len(), 1);
        record_suppressed(&db, &pending[0]).unwrap();
        assert_eq!(
            db.conn()
                .query_row(
                    "SELECT suppressed_count FROM monitor_notification_state",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        assert!(pending_notifications(&db).unwrap().is_empty());

        db.conn()
            .execute(
                "INSERT INTO maintenance_markers (id, starts_at, ends_at, reason)
             VALUES (?1, ?2, ?3, 'Upgrade')",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    (Utc::now() - chrono::Duration::minutes(1)).to_rfc3339(),
                    (Utc::now() + chrono::Duration::minutes(10)).to_rfc3339()
                ],
            )
            .unwrap();
        db.conn()
            .execute(
                "UPDATE monitor_incidents SET last_seen_at = ?1 WHERE id = ?2",
                params![
                    (Utc::now() + chrono::Duration::minutes(2)).to_rfc3339(),
                    incident_id
                ],
            )
            .unwrap();
        assert!(pending_notifications(&db).unwrap().is_empty());

        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}
