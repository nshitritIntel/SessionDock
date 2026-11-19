use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{AppError, Database};

const MAX_HISTORY_LIMIT: u32 = 500;
const MAX_BASELINE_NAME_LENGTH: usize = 100;
const MAX_MAINTENANCE_REASON_LENGTH: usize = 500;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorSample {
    id: i64,
    monitor_id: String,
    observed_at: String,
    status: String,
    duration_ms: i64,
    summary: serde_json::Value,
    error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InterfaceSnapshot {
    name: String,
    admin_status: Option<String>,
    oper_status: Option<String>,
    speed_bps: Option<i64>,
    input_rate_bps: Option<i64>,
    output_rate_bps: Option<i64>,
    input_errors: Option<i64>,
    output_errors: Option<i64>,
    input_discards: Option<i64>,
    output_discards: Option<i64>,
    neighbor: Option<String>,
    transceiver_status: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorSampleDetail {
    sample: MonitorSample,
    interfaces: Vec<InterfaceSnapshot>,
    raw_output: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorIncident {
    id: String,
    monitor_id: String,
    fingerprint: String,
    severity: String,
    state: String,
    title: String,
    details: Option<String>,
    started_at: String,
    last_seen_at: String,
    resolved_at: Option<String>,
    occurrence_count: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorBaseline {
    id: String,
    monitor_id: String,
    name: String,
    source_observed_at: String,
    captured_at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BaselineComparison {
    baseline: MonitorBaseline,
    current_observed_at: String,
    added_interfaces: Vec<String>,
    removed_interfaces: Vec<String>,
    changed_interfaces: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateMaintenanceMarkerRequest {
    session_id: Option<String>,
    starts_at: String,
    ends_at: String,
    reason: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceMarker {
    id: String,
    session_id: Option<String>,
    starts_at: String,
    ends_at: String,
    reason: String,
    created_at: String,
}

fn validate_uuid(value: &str, label: &str) -> Result<(), AppError> {
    uuid::Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| AppError::Validation(format!("Invalid {label} identifier")))
}

fn sample_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MonitorSample> {
    let summary_json: String = row.get(4)?;
    Ok(MonitorSample {
        id: row.get(0)?,
        monitor_id: row.get(1)?,
        observed_at: row.get(2)?,
        status: row.get(3)?,
        duration_ms: row.get(5)?,
        summary: serde_json::from_str(&summary_json).unwrap_or(serde_json::Value::Null),
        error: row.get(6)?,
    })
}

fn interfaces_for_sample(
    conn: &Connection,
    sample_id: i64,
) -> Result<Vec<InterfaceSnapshot>, AppError> {
    let mut statement = conn.prepare(
        "SELECT interface_name, admin_status, oper_status, speed_bps, input_rate_bps,
                output_rate_bps, input_errors, output_errors, input_discards, output_discards,
                neighbor, transceiver_status
         FROM interface_samples WHERE sample_id = ?1 ORDER BY interface_name",
    )?;
    let rows = statement.query_map(params![sample_id], |row| {
        Ok(InterfaceSnapshot {
            name: row.get(0)?,
            admin_status: row.get(1)?,
            oper_status: row.get(2)?,
            speed_bps: row.get(3)?,
            input_rate_bps: row.get(4)?,
            output_rate_bps: row.get(5)?,
            input_errors: row.get(6)?,
            output_errors: row.get(7)?,
            input_discards: row.get(8)?,
            output_discards: row.get(9)?,
            neighbor: row.get(10)?,
            transceiver_status: row.get(11)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(AppError::Database)
}

fn baseline_by_id(conn: &Connection, id: &str) -> Result<MonitorBaseline, AppError> {
    conn.query_row(
        "SELECT id, monitor_id, name, source_observed_at, captured_at
         FROM monitor_baselines WHERE id = ?1",
        params![id],
        |row| {
            Ok(MonitorBaseline {
                id: row.get(0)?,
                monitor_id: row.get(1)?,
                name: row.get(2)?,
                source_observed_at: row.get(3)?,
                captured_at: row.get(4)?,
            })
        },
    )
    .map_err(|error| match error {
        rusqlite::Error::QueryReturnedNoRows => {
            AppError::NotFound(format!("Monitor baseline not found: {id}"))
        }
        other => AppError::Database(other),
    })
}

fn capture_baseline(
    conn: &Connection,
    monitor_id: &str,
    name: &str,
) -> Result<MonitorBaseline, AppError> {
    validate_uuid(monitor_id, "monitor")?;
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_BASELINE_NAME_LENGTH {
        return Err(AppError::Validation(format!(
            "Baseline name must contain 1 to {MAX_BASELINE_NAME_LENGTH} characters"
        )));
    }
    let (sample_id, observed_at) = conn
        .query_row(
            "SELECT id, observed_at FROM monitor_samples
             WHERE monitor_id = ?1 AND status != 'failed'
             ORDER BY observed_at DESC, id DESC LIMIT 1",
            params![monitor_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                AppError::NotFound("No successful monitor sample is available".into())
            }
            other => AppError::Database(other),
        })?;
    let snapshot =
        serde_json::to_string(&interfaces_for_sample(conn, sample_id)?).map_err(|error| {
            AppError::Generic(format!("Could not encode monitor baseline: {error}"))
        })?;
    let id = uuid::Uuid::new_v4().to_string();
    let captured_at = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO monitor_baselines
         (id, monitor_id, name, snapshot_json, source_observed_at, captured_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, monitor_id, name, snapshot, observed_at, captured_at],
    )?;
    baseline_by_id(conn, &id)
}

#[tauri::command]
pub fn get_monitor_samples(
    db: State<'_, Database>,
    monitor_id: String,
    limit: Option<u32>,
) -> Result<Vec<MonitorSample>, AppError> {
    validate_uuid(&monitor_id, "monitor")?;
    let limit = limit.unwrap_or(100).clamp(1, MAX_HISTORY_LIMIT);
    let conn = db.conn();
    let mut statement = conn.prepare(
        "SELECT id, monitor_id, observed_at, status, summary_json, duration_ms, error
         FROM monitor_samples WHERE monitor_id = ?1
         ORDER BY observed_at DESC, id DESC LIMIT ?2",
    )?;
    let rows = statement.query_map(params![monitor_id, limit], sample_from_row)?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(AppError::Database)
}

#[tauri::command]
pub fn get_monitor_sample_detail(
    db: State<'_, Database>,
    sample_id: i64,
) -> Result<MonitorSampleDetail, AppError> {
    if sample_id <= 0 {
        return Err(AppError::Validation(
            "Invalid monitor sample identifier".into(),
        ));
    }
    let conn = db.conn();
    let sample = conn
        .query_row(
            "SELECT id, monitor_id, observed_at, status, summary_json, duration_ms, error
             FROM monitor_samples WHERE id = ?1",
            params![sample_id],
            sample_from_row,
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                AppError::NotFound(format!("Monitor sample not found: {sample_id}"))
            }
            other => AppError::Database(other),
        })?;
    let interfaces = interfaces_for_sample(&conn, sample_id)?;
    let raw_output = conn.query_row(
        "SELECT raw_output FROM monitor_samples WHERE id = ?1",
        params![sample_id],
        |row| row.get(0),
    )?;
    Ok(MonitorSampleDetail {
        sample,
        interfaces,
        raw_output,
    })
}

#[tauri::command]
pub fn get_monitor_incidents(
    db: State<'_, Database>,
    include_resolved: Option<bool>,
) -> Result<Vec<MonitorIncident>, AppError> {
    let conn = db.conn();
    let mut statement = conn.prepare(
        "SELECT id, monitor_id, fingerprint, severity, state, title, details,
                started_at, last_seen_at, resolved_at, occurrence_count
         FROM monitor_incidents
         WHERE ?1 = 1 OR state != 'resolved'
         ORDER BY last_seen_at DESC LIMIT 500",
    )?;
    let rows = statement.query_map(params![include_resolved.unwrap_or(false)], |row| {
        Ok(MonitorIncident {
            id: row.get(0)?,
            monitor_id: row.get(1)?,
            fingerprint: row.get(2)?,
            severity: row.get(3)?,
            state: row.get(4)?,
            title: row.get(5)?,
            details: row.get(6)?,
            started_at: row.get(7)?,
            last_seen_at: row.get(8)?,
            resolved_at: row.get(9)?,
            occurrence_count: row.get(10)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(AppError::Database)
}

#[tauri::command]
pub fn acknowledge_monitor_incident(db: State<'_, Database>, id: String) -> Result<(), AppError> {
    validate_uuid(&id, "incident")?;
    let changed = db.conn().execute(
        "UPDATE monitor_incidents SET state = 'acknowledged'
         WHERE id = ?1 AND state = 'open'",
        params![id],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound(
            "Open monitor incident was not found".into(),
        ));
    }
    Ok(())
}

#[tauri::command]
pub fn capture_monitor_baseline(
    db: State<'_, Database>,
    monitor_id: String,
    name: String,
) -> Result<MonitorBaseline, AppError> {
    capture_baseline(&db.conn(), &monitor_id, &name)
}

#[tauri::command]
pub fn get_monitor_baselines(
    db: State<'_, Database>,
    monitor_id: String,
) -> Result<Vec<MonitorBaseline>, AppError> {
    validate_uuid(&monitor_id, "monitor")?;
    let conn = db.conn();
    let mut statement = conn.prepare(
        "SELECT id FROM monitor_baselines WHERE monitor_id = ?1 ORDER BY captured_at DESC",
    )?;
    let ids = statement
        .query_map(params![monitor_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    ids.iter().map(|id| baseline_by_id(&conn, id)).collect()
}

#[tauri::command]
pub fn compare_monitor_baseline(
    db: State<'_, Database>,
    id: String,
) -> Result<BaselineComparison, AppError> {
    validate_uuid(&id, "baseline")?;
    let conn = db.conn();
    let baseline = baseline_by_id(&conn, &id)?;
    let snapshot_json: String = conn.query_row(
        "SELECT snapshot_json FROM monitor_baselines WHERE id = ?1",
        params![id],
        |row| row.get(0),
    )?;
    let baseline_interfaces: Vec<InterfaceSnapshot> = serde_json::from_str(&snapshot_json)
        .map_err(|error| {
            AppError::Generic(format!("Could not decode monitor baseline: {error}"))
        })?;
    let (sample_id, current_observed_at) = conn
        .query_row(
            "SELECT id, observed_at FROM monitor_samples
             WHERE monitor_id = ?1 AND status != 'failed'
             ORDER BY observed_at DESC, id DESC LIMIT 1",
            params![baseline.monitor_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                AppError::NotFound("No current successful monitor sample is available".into())
            }
            other => AppError::Database(other),
        })?;
    let current_interfaces = interfaces_for_sample(&conn, sample_id)?;
    let (added_interfaces, removed_interfaces, changed_interfaces) =
        compare_snapshots(baseline_interfaces, current_interfaces);
    Ok(BaselineComparison {
        baseline,
        current_observed_at,
        added_interfaces,
        removed_interfaces,
        changed_interfaces,
    })
}

#[tauri::command]
pub fn delete_monitor_baseline(db: State<'_, Database>, id: String) -> Result<(), AppError> {
    validate_uuid(&id, "baseline")?;
    let changed = db
        .conn()
        .execute("DELETE FROM monitor_baselines WHERE id = ?1", params![id])?;
    if changed == 0 {
        return Err(AppError::NotFound(format!(
            "Monitor baseline not found: {id}"
        )));
    }
    Ok(())
}

fn create_maintenance_marker(
    conn: &Connection,
    request: &CreateMaintenanceMarkerRequest,
) -> Result<MaintenanceMarker, AppError> {
    if let Some(session_id) = &request.session_id {
        validate_uuid(session_id, "session")?;
        let exists = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ?1)",
            params![session_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !exists {
            return Err(AppError::NotFound("Maintenance session not found".into()));
        }
    }
    let starts_at = DateTime::parse_from_rfc3339(&request.starts_at)
        .map_err(|_| AppError::Validation("Maintenance start must be RFC 3339".into()))?
        .with_timezone(&Utc);
    let ends_at = DateTime::parse_from_rfc3339(&request.ends_at)
        .map_err(|_| AppError::Validation("Maintenance end must be RFC 3339".into()))?
        .with_timezone(&Utc);
    if ends_at <= starts_at {
        return Err(AppError::Validation(
            "Maintenance end must be after its start".into(),
        ));
    }
    let reason = request.reason.trim();
    if reason.is_empty() || reason.chars().count() > MAX_MAINTENANCE_REASON_LENGTH {
        return Err(AppError::Validation(format!(
            "Maintenance reason must contain 1 to {MAX_MAINTENANCE_REASON_LENGTH} characters"
        )));
    }
    let marker = MaintenanceMarker {
        id: uuid::Uuid::new_v4().to_string(),
        session_id: request.session_id.clone(),
        starts_at: starts_at.to_rfc3339(),
        ends_at: ends_at.to_rfc3339(),
        reason: reason.to_string(),
        created_at: Utc::now().to_rfc3339(),
    };
    conn.execute(
        "INSERT INTO maintenance_markers (id, session_id, starts_at, ends_at, reason, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            marker.id,
            marker.session_id,
            marker.starts_at,
            marker.ends_at,
            marker.reason,
            marker.created_at
        ],
    )?;
    Ok(marker)
}

#[tauri::command]
pub fn create_monitor_maintenance(
    db: State<'_, Database>,
    request: CreateMaintenanceMarkerRequest,
) -> Result<MaintenanceMarker, AppError> {
    create_maintenance_marker(&db.conn(), &request)
}

#[tauri::command]
pub fn get_monitor_maintenance(
    db: State<'_, Database>,
) -> Result<Vec<MaintenanceMarker>, AppError> {
    let conn = db.conn();
    let mut statement = conn.prepare(
        "SELECT id, session_id, starts_at, ends_at, reason, created_at
         FROM maintenance_markers ORDER BY starts_at DESC LIMIT 500",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(MaintenanceMarker {
            id: row.get(0)?,
            session_id: row.get(1)?,
            starts_at: row.get(2)?,
            ends_at: row.get(3)?,
            reason: row.get(4)?,
            created_at: row.get(5)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(AppError::Database)
}

#[tauri::command]
pub fn delete_monitor_maintenance(db: State<'_, Database>, id: String) -> Result<(), AppError> {
    validate_uuid(&id, "maintenance")?;
    let changed = db
        .conn()
        .execute("DELETE FROM maintenance_markers WHERE id = ?1", params![id])?;
    if changed == 0 {
        return Err(AppError::NotFound(format!(
            "Maintenance marker not found: {id}"
        )));
    }
    Ok(())
}

fn compare_snapshots(
    baseline_interfaces: Vec<InterfaceSnapshot>,
    current_interfaces: Vec<InterfaceSnapshot>,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let before = baseline_interfaces
        .into_iter()
        .map(|interface| (interface.name.clone(), interface))
        .collect::<BTreeMap<_, _>>();
    let current = current_interfaces
        .into_iter()
        .map(|interface| (interface.name.clone(), interface))
        .collect::<BTreeMap<_, _>>();
    let added = current
        .keys()
        .filter(|name| !before.contains_key(*name))
        .cloned()
        .collect();
    let removed = before
        .keys()
        .filter(|name| !current.contains_key(*name))
        .cloned()
        .collect();
    let changed = current
        .iter()
        .filter(|(name, value)| before.get(*name).is_some_and(|prior| prior != *value))
        .map(|(name, _)| name.clone())
        .collect();
    (added, removed, changed)
}

#[cfg(test)]
mod tests {
    use super::{
        capture_baseline, compare_snapshots, create_maintenance_marker,
        CreateMaintenanceMarkerRequest, InterfaceSnapshot,
    };
    use crate::Database;

    fn interface(name: &str, state: &str) -> InterfaceSnapshot {
        InterfaceSnapshot {
            name: name.into(),
            admin_status: Some("enabled".into()),
            oper_status: Some(state.into()),
            speed_bps: Some(1_000_000_000),
            input_rate_bps: None,
            output_rate_bps: None,
            input_errors: Some(0),
            output_errors: Some(0),
            input_discards: Some(0),
            output_discards: Some(0),
            neighbor: None,
            transceiver_status: None,
        }
    }

    #[test]
    fn compares_interface_snapshots_deterministically() {
        let before = vec![interface("Ethernet1", "up"), interface("Ethernet2", "up")];
        let current = vec![interface("Ethernet1", "down"), interface("Ethernet3", "up")];
        let (added, removed, changed) = compare_snapshots(before, current);
        assert_eq!(added, vec!["Ethernet3"]);
        assert_eq!(removed, vec!["Ethernet2"]);
        assert_eq!(changed, vec!["Ethernet1"]);
    }

    #[test]
    fn captures_copied_baseline_and_validates_maintenance_window() {
        let path = std::env::temp_dir().join(format!(
            "sessiondock-monitor-data-{}.db",
            uuid::Uuid::new_v4()
        ));
        let db = Database::new(&path).unwrap();
        let session_id = uuid::Uuid::new_v4().to_string();
        let monitor_id = uuid::Uuid::new_v4().to_string();
        let sample_id;
        {
            let conn = db.conn();
            conn.execute(
                "INSERT INTO sessions (id, name, host, protocol, authentication_method, created_at, updated_at)
                 VALUES (?1, 'Leaf', '192.0.2.10', 'ssh', 'agent', datetime('now'), datetime('now'))",
                rusqlite::params![session_id],
            ).unwrap();
            conn.execute(
                "INSERT INTO monitor_definitions (id, session_id, provider_id, monitor_kind, display_name, interval_seconds)
                 VALUES (?1, ?2, 'arista-eos', 'interfaces', 'Interfaces', 60)",
                rusqlite::params![monitor_id, session_id],
            ).unwrap();
            conn.execute(
                "INSERT INTO monitor_samples (monitor_id, observed_at, status, duration_ms)
                 VALUES (?1, '2026-01-01T00:00:00Z', 'healthy', 1)",
                rusqlite::params![monitor_id],
            )
            .unwrap();
            sample_id = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO interface_samples (sample_id, interface_name, oper_status)
                 VALUES (?1, 'Ethernet1', 'up')",
                rusqlite::params![sample_id],
            )
            .unwrap();
            let baseline = capture_baseline(&conn, &monitor_id, "Known good").unwrap();
            conn.execute(
                "DELETE FROM monitor_samples WHERE id = ?1",
                rusqlite::params![sample_id],
            )
            .unwrap();
            assert_eq!(
                conn.query_row(
                    "SELECT COUNT(*) FROM monitor_baselines WHERE id = ?1",
                    rusqlite::params![baseline.id],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
                1
            );
            assert!(create_maintenance_marker(
                &conn,
                &CreateMaintenanceMarkerRequest {
                    session_id: Some(session_id.clone()),
                    starts_at: "2026-01-02T02:00:00Z".into(),
                    ends_at: "2026-01-02T01:00:00Z".into(),
                    reason: "Upgrade".into(),
                },
            )
            .is_err());
        }
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}
