use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};
use tauri::State;

use crate::db::Database;
use crate::error::AppError;
use crate::providers::network_monitor::{self, InterfaceObservation};

const MIN_INTERVAL_SECONDS: u32 = 30;
const MAX_INTERVAL_SECONDS: u32 = 86_400;
const MIN_RETENTION_HOURS: u32 = 24;
const MAX_RETENTION_HOURS: u32 = 8_760;
const MAX_DISPLAY_NAME_LENGTH: usize = 100;
const MONITOR_LEASE_TIMEOUT_MINUTES: i64 = 5;
const MAX_SSH_ATTEMPTS: usize = 3;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateMonitorRequest {
    session_id: String,
    provider_id: String,
    monitor_kind: String,
    interface_name: String,
    display_name: String,
    interval_seconds: u32,
    retention_hours: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorDefinition {
    id: String,
    session_id: String,
    session_name: String,
    host: String,
    provider_id: String,
    monitor_kind: String,
    interface_name: Option<String>,
    display_name: String,
    interval_seconds: u32,
    retention_hours: u32,
    enabled: bool,
    latest_status: Option<String>,
    latest_observed_at: Option<String>,
    open_incident_count: u32,
    created_at: String,
    updated_at: String,
}

struct MonitorRunConfig {
    monitor_id: String,
    session_id: String,
    provider_id: String,
    host: String,
    port: u16,
    username: String,
    authentication_method: String,
    credential_profile_id: Option<String>,
    private_key_reference: Option<String>,
    jump_host_ids: Vec<String>,
    interface_name: Option<String>,
    retention_hours: u32,
}

struct MonitorRunLease<'a> {
    db: &'a Database,
    session_id: String,
    monitor_id: String,
}

impl Drop for MonitorRunLease<'_> {
    fn drop(&mut self) {
        if let Err(error) = self.db.conn().execute(
            "DELETE FROM monitor_run_leases WHERE session_id = ?1 AND monitor_id = ?2",
            params![self.session_id, self.monitor_id],
        ) {
            log::warn!("Could not release monitor execution lease: {error}");
        }
    }
}

fn monitor_commands(
    provider_id: &str,
    interface_name: Option<&str>,
) -> Result<network_monitor::MonitorCommands, AppError> {
    let interface_name = interface_name.ok_or_else(|| {
        AppError::Validation(
            "This legacy monitor has no interface scope; delete it and create a scoped monitor"
                .into(),
        )
    })?;
    network_monitor::commands(provider_id, interface_name)
}

fn validate_monitor_request(request: &CreateMonitorRequest) -> Result<(&str, &str), AppError> {
    uuid::Uuid::parse_str(&request.session_id)
        .map_err(|_| AppError::Validation("Invalid monitoring session identifier".into()))?;
    if !network_monitor::is_supported(&request.provider_id) || request.monitor_kind != "interfaces"
    {
        return Err(AppError::Validation(
            "Unsupported provider monitor; select Arista EOS or Cisco IOS/IOS-XE".into(),
        ));
    }
    let display_name = request.display_name.trim();
    if display_name.is_empty() || display_name.chars().count() > MAX_DISPLAY_NAME_LENGTH {
        return Err(AppError::Validation(format!(
            "Monitor name must contain 1 to {MAX_DISPLAY_NAME_LENGTH} characters"
        )));
    }
    if !(MIN_INTERVAL_SECONDS..=MAX_INTERVAL_SECONDS).contains(&request.interval_seconds) {
        return Err(AppError::Validation(format!(
            "Monitor interval must be between {MIN_INTERVAL_SECONDS} and {MAX_INTERVAL_SECONDS} seconds"
        )));
    }
    if !(MIN_RETENTION_HOURS..=MAX_RETENTION_HOURS).contains(&request.retention_hours) {
        return Err(AppError::Validation(format!(
            "Monitor retention must be between {MIN_RETENTION_HOURS} and {MAX_RETENTION_HOURS} hours"
        )));
    }
    Ok((
        display_name,
        network_monitor::validate_interface_name(&request.interface_name)?,
    ))
}

fn monitor_by_id(conn: &Connection, id: &str) -> Result<MonitorDefinition, AppError> {
    conn.query_row(
        "SELECT m.id, m.session_id, s.name, s.host, m.provider_id, m.monitor_kind,
            m.interface_name, m.display_name, m.interval_seconds, m.retention_hours, m.enabled,
                latest.status, latest.observed_at,
                (SELECT COUNT(*) FROM monitor_incidents i WHERE i.monitor_id = m.id AND i.state != 'resolved'),
                m.created_at, m.updated_at
         FROM monitor_definitions m
         JOIN sessions s ON s.id = m.session_id
         LEFT JOIN monitor_samples latest ON latest.id = (
             SELECT id FROM monitor_samples WHERE monitor_id = m.id ORDER BY observed_at DESC, id DESC LIMIT 1
         )
         WHERE m.id = ?1",
        params![id],
        |row| {
            Ok(MonitorDefinition {
                id: row.get(0)?,
                session_id: row.get(1)?,
                session_name: row.get(2)?,
                host: row.get(3)?,
                provider_id: row.get(4)?,
                monitor_kind: row.get(5)?,
                interface_name: row.get(6)?,
                display_name: row.get(7)?,
                interval_seconds: row.get(8)?,
                retention_hours: row.get(9)?,
                enabled: row.get::<_, i64>(10)? != 0,
                latest_status: row.get(11)?,
                latest_observed_at: row.get(12)?,
                open_incident_count: row.get(13)?,
                created_at: row.get(14)?,
                updated_at: row.get(15)?,
            })
        },
    )
    .map_err(|error| match error {
        rusqlite::Error::QueryReturnedNoRows => {
            AppError::NotFound(format!("Monitor definition not found: {id}"))
        }
        other => AppError::Database(other),
    })
}

fn create_monitor(
    conn: &Connection,
    request: &CreateMonitorRequest,
) -> Result<MonitorDefinition, AppError> {
    let (display_name, interface_name) = validate_monitor_request(request)?;
    let protocol: String = conn
        .query_row(
            "SELECT protocol FROM sessions WHERE id = ?1",
            params![request.session_id],
            |row| row.get(0),
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                AppError::NotFound("Monitoring session not found".into())
            }
            other => AppError::Database(other),
        })?;
    if protocol != "ssh" {
        return Err(AppError::Validation(
            "Monitoring requires a saved SSH session".into(),
        ));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO monitor_definitions
         (id, session_id, provider_id, monitor_kind, interface_name, display_name, interval_seconds, retention_hours, enabled, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9, ?9)",
        params![
            id,
            request.session_id,
            request.provider_id,
            request.monitor_kind,
            interface_name,
            display_name,
            request.interval_seconds,
            request.retention_hours,
            now,
        ],
    )
    .map_err(|error| match error {
        rusqlite::Error::SqliteFailure(ref failure, _)
            if failure.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            AppError::Validation("This session already has that monitor".into())
        }
        other => AppError::Database(other),
    })?;
    monitor_by_id(conn, &id)
}

#[tauri::command]
pub fn create_monitor_definition(
    db: State<'_, Database>,
    request: CreateMonitorRequest,
) -> Result<MonitorDefinition, AppError> {
    create_monitor(&db.conn(), &request)
}

#[tauri::command]
pub fn get_monitor_definitions(
    db: State<'_, Database>,
) -> Result<Vec<MonitorDefinition>, AppError> {
    let conn = db.conn();
    let mut statement =
        conn.prepare("SELECT id FROM monitor_definitions ORDER BY display_name COLLATE NOCASE")?;
    let ids = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    ids.iter().map(|id| monitor_by_id(&conn, id)).collect()
}

#[tauri::command]
pub fn set_monitor_enabled(
    db: State<'_, Database>,
    id: String,
    enabled: bool,
) -> Result<MonitorDefinition, AppError> {
    uuid::Uuid::parse_str(&id)
        .map_err(|_| AppError::Validation("Invalid monitor identifier".into()))?;
    let conn = db.conn();
    let changed = conn.execute(
        "UPDATE monitor_definitions SET enabled = ?1, updated_at = ?2 WHERE id = ?3",
        params![enabled as i32, chrono::Utc::now().to_rfc3339(), id],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound(format!(
            "Monitor definition not found: {id}"
        )));
    }
    monitor_by_id(&conn, &id)
}

#[tauri::command]
pub fn set_monitor_interval(
    db: State<'_, Database>,
    id: String,
    interval_seconds: u32,
) -> Result<MonitorDefinition, AppError> {
    uuid::Uuid::parse_str(&id)
        .map_err(|_| AppError::Validation("Invalid monitor identifier".into()))?;
    if !(MIN_INTERVAL_SECONDS..=MAX_INTERVAL_SECONDS).contains(&interval_seconds) {
        return Err(AppError::Validation(format!(
            "Monitor interval must be between {MIN_INTERVAL_SECONDS} and {MAX_INTERVAL_SECONDS} seconds"
        )));
    }
    let conn = db.conn();
    let changed = conn.execute(
        "UPDATE monitor_definitions SET interval_seconds = ?1, updated_at = ?2 WHERE id = ?3",
        params![interval_seconds, chrono::Utc::now().to_rfc3339(), id],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound(format!(
            "Monitor definition not found: {id}"
        )));
    }
    monitor_by_id(&conn, &id)
}

#[tauri::command]
pub fn delete_monitor_definition(db: State<'_, Database>, id: String) -> Result<(), AppError> {
    uuid::Uuid::parse_str(&id)
        .map_err(|_| AppError::Validation("Invalid monitor identifier".into()))?;
    let deleted = db
        .conn()
        .execute("DELETE FROM monitor_definitions WHERE id = ?1", params![id])?;
    if deleted == 0 {
        return Err(AppError::NotFound(format!(
            "Monitor definition not found: {id}"
        )));
    }
    Ok(())
}

fn load_monitor_run_config(conn: &Connection, id: &str) -> Result<MonitorRunConfig, AppError> {
    let mut config = conn
        .query_row(
                    "SELECT m.id, m.session_id, m.provider_id, s.host, s.port, COALESCE(s.username, ''), s.authentication_method,
                    s.credential_profile_id, s.private_key_reference, m.interface_name, m.retention_hours
             FROM monitor_definitions m JOIN sessions s ON s.id = m.session_id
             WHERE m.id = ?1 AND m.enabled = 1",
            params![id],
            |row| {
                Ok(MonitorRunConfig {
                    monitor_id: row.get(0)?,
                    session_id: row.get(1)?,
                    provider_id: row.get(2)?,
                    host: row.get(3)?,
                    port: row.get(4)?,
                    username: row.get(5)?,
                    authentication_method: row.get(6)?,
                    credential_profile_id: row.get(7)?,
                    private_key_reference: row.get(8)?,
                    jump_host_ids: Vec::new(),
                    interface_name: row.get(9)?,
                    retention_hours: row.get(10)?,
                })
            },
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                AppError::NotFound("Enabled monitor definition not found".into())
            }
            other => AppError::Database(other),
        })?;
    let mut statement = conn.prepare(
        "SELECT jump_session_id FROM session_jump_hosts
         WHERE session_id = (SELECT session_id FROM monitor_definitions WHERE id = ?1)
         ORDER BY position",
    )?;
    config.jump_host_ids = statement
        .query_map(params![id], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(config)
}

fn acquire_monitor_run_lease<'a>(
    db: &'a Database,
    config: &MonitorRunConfig,
) -> Result<MonitorRunLease<'a>, AppError> {
    let now = chrono::Utc::now();
    let stale_before =
        (now - chrono::Duration::minutes(MONITOR_LEASE_TIMEOUT_MINUTES)).to_rfc3339();
    let changed = db.conn().execute(
        "INSERT INTO monitor_run_leases (session_id, monitor_id, started_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(session_id) DO UPDATE SET monitor_id = excluded.monitor_id, started_at = excluded.started_at
         WHERE monitor_run_leases.started_at <= ?4",
        params![config.session_id, config.monitor_id, now.to_rfc3339(), stale_before],
    )?;
    if changed == 0 {
        return Err(AppError::Generic(
            "A Network Health check is already running for this saved session".into(),
        ));
    }
    Ok(MonitorRunLease {
        db,
        session_id: config.session_id.clone(),
        monitor_id: config.monitor_id.clone(),
    })
}

fn is_transient_ssh_error(error: &AppError) -> bool {
    let AppError::Ssh(message) = error else {
        return false;
    };
    let message = message.to_ascii_lowercase();
    [
        "unable to exchange encryption keys",
        "session(-8)",
        "timed out",
        "timeout",
        "connection reset",
        "connection aborted",
        "broken pipe",
        "temporarily unavailable",
        "tcp connection",
    ]
    .iter()
    .any(|needle| message.contains(needle))
        && !message.contains("authentication")
        && !message.contains("host key")
}

fn retry_delay(monitor_id: &str, attempt: usize) -> Duration {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    monitor_id.hash(&mut hasher);
    attempt.hash(&mut hasher);
    let jitter_ms = hasher.finish() % 200;
    Duration::from_millis(250 * (1_u64 << attempt) + jitter_ms)
}

async fn open_monitor_connection(
    db: &Database,
    config: &MonitorRunConfig,
    host: &str,
) -> Result<crate::terminal::StandaloneSshConnection, AppError> {
    for attempt in 0..MAX_SSH_ATTEMPTS {
        let authentication = super::terminal::resolve_ssh_authentication(
            db,
            &config.authentication_method,
            config.credential_profile_id.as_deref(),
            config.private_key_reference.as_deref(),
            "",
        )?;
        let jump_hosts = super::terminal::resolve_jump_hosts(db, &config.jump_host_ids)?;
        match crate::terminal::open_trusted_ssh_connection(
            db,
            host.to_string(),
            config.port,
            config.username.clone(),
            authentication,
            jump_hosts,
        )
        .await
        {
            Ok(connection) => return Ok(connection),
            Err(error) if is_transient_ssh_error(&error) && attempt + 1 < MAX_SSH_ATTEMPTS => {
                log::warn!(
                    "Transient monitor SSH transport failure for monitor {}; retrying attempt {} of {}",
                    config.monitor_id,
                    attempt + 2,
                    MAX_SSH_ATTEMPTS
                );
                tokio::time::sleep(retry_delay(&config.monitor_id, attempt)).await;
            }
            Err(AppError::Ssh(message))
                if is_transient_ssh_error(&AppError::Ssh(message.clone())) =>
            {
                return Err(AppError::Ssh(format!(
                    "Monitor transport failed after {MAX_SSH_ATTEMPTS} attempts: {message}. Check device SSH connection limits, reachability, and supported key-exchange algorithms"
                )));
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("the bounded SSH retry loop always returns")
}

fn persist_sample(
    conn: &Connection,
    config: &MonitorRunConfig,
    observed_at: &str,
    duration_ms: i64,
    raw_output: &str,
    observations: &[InterfaceObservation],
    log_count: usize,
) -> Result<i64, AppError> {
    let down_count = observations
        .iter()
        .filter(|interface| {
            interface.admin_status.as_deref() != Some("disabled")
                && matches!(
                    interface.oper_status.as_deref(),
                    Some("down" | "notconnect" | "errdisabled")
                )
        })
        .count();
    let error_count = observations
        .iter()
        .filter(|interface| {
            interface.input_errors.unwrap_or(0) > 0
                || interface.output_errors.unwrap_or(0) > 0
                || interface.input_discards.unwrap_or(0) > 0
                || interface.output_discards.unwrap_or(0) > 0
        })
        .count();
    let status = if observations.is_empty() {
        "unknown"
    } else if down_count > 0 {
        "critical"
    } else if error_count > 0 {
        "warning"
    } else {
        "healthy"
    };
    let summary = serde_json::json!({
        "interfaceCount": observations.len(),
        "downCount": down_count,
        "errorCount": error_count,
        "logCount": log_count,
    });
    conn.execute(
        "INSERT INTO monitor_samples (monitor_id, observed_at, status, duration_ms, summary_json, raw_output)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![config.monitor_id, observed_at, status, duration_ms, summary.to_string(), raw_output],
    )?;
    let sample_id = conn.last_insert_rowid();
    for interface in observations {
        conn.execute(
            "INSERT INTO interface_samples
             (sample_id, interface_name, admin_status, oper_status, speed_bps, input_rate_bps,
              output_rate_bps, input_errors, output_errors, input_discards, output_discards, neighbor, transceiver_status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![sample_id, interface.name, interface.admin_status, interface.oper_status,
                interface.speed_bps, interface.input_rate_bps, interface.output_rate_bps,
                interface.input_errors, interface.output_errors, interface.input_discards,
                interface.output_discards, interface.neighbor, interface.transceiver_status],
        )?;
    }
    update_interface_incident(conn, config, observed_at, down_count)?;
    update_interface_error_incident(conn, config, observed_at, error_count)?;
    resolve_execution_incident(conn, config, observed_at)?;
    conn.execute(
        "DELETE FROM monitor_samples WHERE monitor_id = ?1
         AND julianday(observed_at) < julianday(?2, '-' || ?3 || ' hours')",
        params![config.monitor_id, observed_at, config.retention_hours],
    )?;
    Ok(sample_id)
}

fn persist_failed_sample(
    conn: &Connection,
    config: &MonitorRunConfig,
    started: Instant,
    error: &AppError,
) -> Result<(), AppError> {
    let observed_at = chrono::Utc::now().to_rfc3339();
    let duration_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
    let bounded_error = error.to_string().chars().take(2_048).collect::<String>();
    conn.execute(
        "INSERT INTO monitor_samples (monitor_id, observed_at, status, duration_ms, summary_json, error)
         VALUES (?1, ?2, 'failed', ?3, '{}', ?4)",
        params![config.monitor_id, observed_at, duration_ms, bounded_error],
    )?;
    update_execution_incident(conn, config, &observed_at)?;
    conn.execute(
        "DELETE FROM monitor_samples WHERE monitor_id = ?1
         AND julianday(observed_at) < julianday(?2, '-' || ?3 || ' hours')",
        params![config.monitor_id, observed_at, config.retention_hours],
    )?;
    Ok(())
}

fn update_execution_incident(
    conn: &Connection,
    config: &MonitorRunConfig,
    observed_at: &str,
) -> Result<(), AppError> {
    let fingerprint = format!("{}:monitor-execution-failed", config.provider_id);
    let existing = conn.query_row(
        "SELECT id FROM monitor_incidents
         WHERE monitor_id = ?1 AND fingerprint = ?2 AND state != 'resolved'",
        params![config.monitor_id, fingerprint],
        |row| row.get::<_, String>(0),
    );
    match existing {
        Ok(id) => {
            conn.execute(
                "UPDATE monitor_incidents SET last_seen_at = ?1,
                        occurrence_count = occurrence_count + 1,
                        details = 'The monitor could not collect a sample' WHERE id = ?2",
                params![observed_at, id],
            )?;
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            conn.execute(
                "INSERT INTO monitor_incidents
                 (id, monitor_id, fingerprint, severity, state, title, details, started_at, last_seen_at)
                 VALUES (?1, ?2, ?3, 'critical', 'open', 'Monitor collection failed',
                         'The monitor could not collect a sample', ?4, ?4)",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    config.monitor_id,
                    fingerprint,
                    observed_at
                ],
            )?;
        }
        Err(error) => return Err(AppError::Database(error)),
    }
    Ok(())
}

fn resolve_execution_incident(
    conn: &Connection,
    config: &MonitorRunConfig,
    observed_at: &str,
) -> Result<(), AppError> {
    let fingerprint = format!("{}:monitor-execution-failed", config.provider_id);
    conn.execute(
        "UPDATE monitor_incidents SET state = 'resolved', resolved_at = ?1, last_seen_at = ?1
         WHERE monitor_id = ?2 AND fingerprint = ?3
           AND state != 'resolved'",
        params![observed_at, config.monitor_id, fingerprint],
    )?;
    Ok(())
}

fn update_interface_incident(
    conn: &Connection,
    config: &MonitorRunConfig,
    observed_at: &str,
    down_count: usize,
) -> Result<(), AppError> {
    let fingerprint = format!("{}:interfaces-down", config.provider_id);
    let existing = conn.query_row(
        "SELECT id FROM monitor_incidents WHERE monitor_id = ?1 AND fingerprint = ?2 AND state != 'resolved'",
        params![config.monitor_id, fingerprint],
        |row| row.get::<_, String>(0),
    );
    if down_count > 0 {
        match existing {
            Ok(id) => {
                conn.execute(
                    "UPDATE monitor_incidents SET last_seen_at = ?1, occurrence_count = occurrence_count + 1,
                            details = ?2 WHERE id = ?3",
                    params![observed_at, format!("{down_count} monitored interfaces are down"), id],
                )?;
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                conn.execute(
                    "INSERT INTO monitor_incidents
                     (id, monitor_id, fingerprint, severity, state, title, details, started_at, last_seen_at)
                     VALUES (?1, ?2, ?3, 'critical', 'open', 'Interfaces down', ?4, ?5, ?5)",
                    params![uuid::Uuid::new_v4().to_string(), config.monitor_id, fingerprint,
                        format!("{down_count} monitored interfaces are down"), observed_at],
                )?;
            }
            Err(error) => return Err(AppError::Database(error)),
        }
    } else if let Ok(id) = existing {
        conn.execute(
            "UPDATE monitor_incidents SET state = 'resolved', resolved_at = ?1, last_seen_at = ?1 WHERE id = ?2",
            params![observed_at, id],
        )?;
    }
    Ok(())
}

fn update_interface_error_incident(
    conn: &Connection,
    config: &MonitorRunConfig,
    observed_at: &str,
    error_count: usize,
) -> Result<(), AppError> {
    let fingerprint = format!("{}:interface-errors", config.provider_id);
    let existing = conn.query_row(
        "SELECT id FROM monitor_incidents WHERE monitor_id = ?1 AND fingerprint = ?2 AND state != 'resolved'",
        params![config.monitor_id, fingerprint],
        |row| row.get::<_, String>(0),
    );
    if error_count > 0 {
        let interface_name = config
            .interface_name
            .as_deref()
            .unwrap_or("monitored interface");
        let details = format!("{interface_name} reports nonzero error or discard counters");
        match existing {
            Ok(id) => {
                conn.execute(
                    "UPDATE monitor_incidents SET last_seen_at = ?1, occurrence_count = occurrence_count + 1,
                            details = ?2 WHERE id = ?3",
                    params![observed_at, details, id],
                )?;
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                conn.execute(
                    "INSERT INTO monitor_incidents
                     (id, monitor_id, fingerprint, severity, state, title, details, started_at, last_seen_at)
                     VALUES (?1, ?2, ?3, 'warning', 'open', 'Interface errors detected', ?4, ?5, ?5)",
                    params![uuid::Uuid::new_v4().to_string(), config.monitor_id, fingerprint, details, observed_at],
                )?;
            }
            Err(error) => return Err(AppError::Database(error)),
        }
    } else if let Ok(id) = existing {
        conn.execute(
            "UPDATE monitor_incidents SET state = 'resolved', resolved_at = ?1, last_seen_at = ?1 WHERE id = ?2",
            params![observed_at, id],
        )?;
    }
    Ok(())
}

pub(crate) async fn run_monitor_by_id(
    db: &Database,
    id: &str,
) -> Result<MonitorDefinition, AppError> {
    uuid::Uuid::parse_str(id)
        .map_err(|_| AppError::Validation("Invalid monitor identifier".into()))?;
    let config = load_monitor_run_config(&db.conn(), id)?;
    let _lease = acquire_monitor_run_lease(db, &config)?;
    {
        let mut conn = db.conn();
        mark_monitor_run_started(&mut conn, id)?;
    }
    let started = Instant::now();
    let result = execute_monitor(db, &config, started).await;
    if let Err(error) = &result {
        if let Err(persist_error) = persist_failed_sample(&db.conn(), &config, started, error) {
            log::error!("Could not persist failed monitor sample: {persist_error}");
        }
    }
    result?;
    monitor_by_id(&db.conn(), id)
}

fn mark_monitor_run_started(conn: &mut Connection, id: &str) -> Result<(), AppError> {
    let transaction = conn.transaction()?;
    transaction.execute(
        "UPDATE monitor_definitions SET last_started_at = ?1 WHERE id = ?2",
        params![chrono::Utc::now().to_rfc3339(), id],
    )?;
    transaction.execute(
        "DELETE FROM monitor_run_requests WHERE monitor_id = ?1",
        params![id],
    )?;
    transaction.commit()?;
    Ok(())
}

fn queue_monitor_run(conn: &Connection, id: &str) -> Result<MonitorDefinition, AppError> {
    uuid::Uuid::parse_str(id)
        .map_err(|_| AppError::Validation("Invalid monitor identifier".into()))?;
    let monitor = monitor_by_id(conn, id)?;
    if !monitor.enabled {
        return Err(AppError::Validation(
            "Enable the monitor before running it".into(),
        ));
    }
    conn.execute(
        "INSERT INTO monitor_run_requests (monitor_id, requested_at)
         VALUES (?1, ?2)
         ON CONFLICT(monitor_id) DO UPDATE SET requested_at = excluded.requested_at",
        params![id, chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(monitor)
}

async fn execute_monitor(
    db: &Database,
    config: &MonitorRunConfig,
    started: Instant,
) -> Result<(), AppError> {
    if config.username.trim().is_empty() {
        return Err(AppError::Validation(
            "The monitored SSH session requires a username".into(),
        ));
    }
    let host = super::terminal::validate_ssh_endpoint(&config.host, config.port)?;
    let connection = open_monitor_connection(db, config, &host).await?;
    let commands = monitor_commands(&config.provider_id, config.interface_name.as_deref())?;
    let output = super::diagnostics::execute_bounded_ssh_command(
        &connection.session,
        &commands.interface,
        &AtomicBool::new(false),
    )
    .await;
    if output.status != "completed" {
        return Err(AppError::Ssh(
            output
                .error
                .unwrap_or_else(|| "Arista EOS monitor command failed".into()),
        ));
    }
    let interface_name = config.interface_name.as_deref().ok_or_else(|| {
        AppError::Validation("The monitor does not have an interface scope".into())
    })?;
    let observations =
        network_monitor::parse_interfaces(&config.provider_id, interface_name, &output.stdout)?;
    let log_output = super::diagnostics::execute_bounded_ssh_command(
        &connection.session,
        &commands.logs,
        &AtomicBool::new(false),
    )
    .await;
    let (relevant_logs, log_count) = normalize_log_output(log_output);
    let raw_output = format!(
        "Interface state:\n{}\n\nRelevant logs:\n{}",
        output.stdout, relevant_logs
    );
    let observed_at = chrono::Utc::now().to_rfc3339();
    let duration_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
    persist_sample(
        &db.conn(),
        config,
        &observed_at,
        duration_ms,
        &raw_output,
        &observations,
        log_count,
    )?;
    Ok(())
}

fn normalize_log_output(output: super::diagnostics::BoundedSshOutput) -> (String, usize) {
    if output.status == "completed" {
        let count = output
            .stdout
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count();
        return (output.stdout, count);
    }
    (
        format!(
            "Log collection unavailable: {}",
            output
                .error
                .unwrap_or_else(|| "Provider logging command failed".into())
        ),
        0,
    )
}

#[tauri::command]
pub fn run_monitor_now(db: State<'_, Database>, id: String) -> Result<MonitorDefinition, AppError> {
    queue_monitor_run(&db.conn(), &id)
}

#[cfg(test)]
mod tests {
    use rusqlite::params;
    use std::time::Instant;

    use super::{
        acquire_monitor_run_lease, create_monitor, is_transient_ssh_error, load_monitor_run_config,
        mark_monitor_run_started, monitor_commands, normalize_log_output, persist_failed_sample,
        persist_sample, queue_monitor_run, run_monitor_by_id, CreateMonitorRequest,
    };
    use crate::{providers::arista_eos::parse_interfaces, AppError, Database};

    #[test]
    fn creates_only_allowlisted_bounded_ssh_monitors() {
        let path = std::env::temp_dir().join(format!(
            "sessiondock-monitoring-{}.db",
            uuid::Uuid::new_v4()
        ));
        let db = Database::new(&path).unwrap();
        let session_id = uuid::Uuid::new_v4().to_string();
        let conn = db.conn();
        conn.execute(
            "INSERT INTO sessions (id, name, host, port, protocol, authentication_method, created_at, updated_at)
             VALUES (?1, 'Leaf 1', '192.0.2.10', 22, 'ssh', 'password', datetime('now'), datetime('now'))",
            rusqlite::params![session_id],
        )
        .unwrap();
        let request = CreateMonitorRequest {
            session_id,
            provider_id: "arista-eos".into(),
            monitor_kind: "interfaces".into(),
            interface_name: "Ethernet15".into(),
            display_name: "Leaf interfaces".into(),
            interval_seconds: 60,
            retention_hours: 168,
        };
        let monitor = create_monitor(&conn, &request).unwrap();
        assert_eq!(monitor.session_name, "Leaf 1");
        assert_eq!(monitor.interface_name.as_deref(), Some("Ethernet15"));
        assert!(monitor.enabled);
        assert!(create_monitor(&conn, &request).is_err());

        assert_eq!(
            monitor_commands("arista-eos", Some("Ethernet15")).unwrap(),
            crate::providers::network_monitor::MonitorCommands {
                interface: "show interfaces Ethernet15 | json".into(),
                logs: "show logging | grep Ethernet15".into(),
            }
        );
        assert!(monitor_commands("arista-eos", Some("Ethernet15 | reload")).is_err());
        assert!(monitor_commands("arista-eos", Some("Ethernet15; reload")).is_err());
        assert!(monitor_commands("arista-eos", Some("../../../etc/passwd")).is_err());
        assert!(monitor_commands("arista-eos", Some("-all")).is_err());
        assert!(monitor_commands("arista-eos", Some(&"a".repeat(65))).is_err());
        assert!(monitor_commands("arista-eos", None).is_err());

        drop(conn);
        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn serializes_monitor_runs_per_saved_session_and_recovers_stale_leases() {
        let path = std::env::temp_dir().join(format!(
            "sessiondock-monitor-lease-{}.db",
            uuid::Uuid::new_v4()
        ));
        let first_db = Database::new(&path).unwrap();
        let session_id = uuid::Uuid::new_v4().to_string();
        let monitor_id = uuid::Uuid::new_v4().to_string();
        {
            let conn = first_db.conn();
            conn.execute(
                "INSERT INTO sessions (id, name, host, port, protocol, username, authentication_method, created_at, updated_at)
                 VALUES (?1, 'Leaf', '192.0.2.10', 22, 'ssh', 'admin', 'agent', datetime('now'), datetime('now'))",
                params![session_id],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO monitor_definitions
                 (id, session_id, provider_id, monitor_kind, interface_name, display_name, interval_seconds)
                 VALUES (?1, ?2, 'arista-eos', 'interfaces', 'Ethernet1', 'Port', 30)",
                params![monitor_id, session_id],
            )
            .unwrap();
        }
        let second_db = Database::new(&path).unwrap();
        let config = load_monitor_run_config(&first_db.conn(), &monitor_id).unwrap();
        let lease = acquire_monitor_run_lease(&first_db, &config).unwrap();
        assert!(acquire_monitor_run_lease(&second_db, &config).is_err());
        drop(lease);
        let replacement = acquire_monitor_run_lease(&second_db, &config).unwrap();
        drop(replacement);

        first_db
            .conn()
            .execute(
                "INSERT INTO monitor_run_leases (session_id, monitor_id, started_at)
             VALUES (?1, ?2, '2000-01-01T00:00:00Z')",
                params![session_id, monitor_id],
            )
            .unwrap();
        let recovered = acquire_monitor_run_lease(&second_db, &config).unwrap();
        drop(recovered);

        drop(second_db);
        drop(first_db);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn retries_only_transient_ssh_transport_failures() {
        assert!(is_transient_ssh_error(&AppError::Ssh(
            "SSH handshake failed: [Session(-8)] Unable to exchange encryption keys".into()
        )));
        assert!(is_transient_ssh_error(&AppError::Ssh(
            "TCP connection timed out".into()
        )));
        assert!(!is_transient_ssh_error(&AppError::Ssh(
            "Authentication stage failed: password rejected".into()
        )));
        assert!(!is_transient_ssh_error(&AppError::Ssh(
            "Host key changed".into()
        )));
        assert!(!is_transient_ssh_error(&AppError::Validation(
            "Invalid host".into()
        )));
    }

    #[tokio::test]
    async fn queues_manual_runs_until_the_session_lease_is_acquired() {
        let path = std::env::temp_dir().join(format!(
            "sessiondock-monitor-queue-{}.db",
            uuid::Uuid::new_v4()
        ));
        let db = Database::new(&path).unwrap();
        let session_id = uuid::Uuid::new_v4().to_string();
        let monitor_id = uuid::Uuid::new_v4().to_string();
        {
            let conn = db.conn();
            conn.execute(
                "INSERT INTO sessions (id, name, host, protocol, authentication_method, created_at, updated_at)
                 VALUES (?1, 'Router', '192.0.2.10', 'ssh', 'agent', datetime('now'), datetime('now'))",
                params![session_id],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO monitor_definitions
                 (id, session_id, provider_id, monitor_kind, interface_name, display_name, interval_seconds)
                 VALUES (?1, ?2, 'cisco-ios', 'interfaces', 'GigabitEthernet1/0/1', 'WAN', 3600)",
                params![monitor_id, session_id],
            )
            .unwrap();

            let queued = queue_monitor_run(&conn, &monitor_id).unwrap();
            assert_eq!(queued.id, monitor_id);
            queue_monitor_run(&conn, &monitor_id).unwrap();
        }
        let config = load_monitor_run_config(&db.conn(), &monitor_id).unwrap();
        let lease = acquire_monitor_run_lease(&db, &config).unwrap();
        assert!(run_monitor_by_id(&db, &monitor_id).await.is_err());
        assert_eq!(
            db.conn()
                .query_row(
                    "SELECT COUNT(*) FROM monitor_run_requests WHERE monitor_id = ?1",
                    params![monitor_id],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        drop(lease);
        mark_monitor_run_started(&mut db.conn(), &monitor_id).unwrap();
        let (last_started_at, request_count): (Option<String>, i64) = db
            .conn()
            .query_row(
                "SELECT last_started_at,
                        (SELECT COUNT(*) FROM monitor_run_requests WHERE monitor_id = ?1)
                 FROM monitor_definitions WHERE id = ?1",
                params![monitor_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert!(last_started_at.is_some());
        assert_eq!(request_count, 0);

        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn accepts_empty_matching_logs_without_failing_the_sample() {
        let (logs, count) = normalize_log_output(super::super::diagnostics::BoundedSshOutput {
            status: "completed",
            stdout: String::new(),
            stderr: String::new(),
            exit_code: Some(0),
            truncated: false,
            error: None,
        });
        assert!(logs.is_empty());
        assert_eq!(count, 0);
    }

    #[test]
    fn persists_normalized_samples_incident_lifecycle_and_retention() {
        let path = std::env::temp_dir().join(format!(
            "sessiondock-monitor-samples-{}.db",
            uuid::Uuid::new_v4()
        ));
        let db = Database::new(&path).unwrap();
        let session_id = uuid::Uuid::new_v4().to_string();
        let monitor_id = {
            let conn = db.conn();
            conn.execute(
                "INSERT INTO sessions (id, name, host, port, protocol, username, authentication_method, created_at, updated_at)
                 VALUES (?1, 'Leaf 1', '192.0.2.10', 22, 'ssh', 'admin', 'agent', datetime('now'), datetime('now'))",
                rusqlite::params![session_id],
            ).unwrap();
            create_monitor(
                &conn,
                &CreateMonitorRequest {
                    session_id,
                    provider_id: "arista-eos".into(),
                    monitor_kind: "interfaces".into(),
                    interface_name: "Ethernet1".into(),
                    display_name: "Leaf interfaces".into(),
                    interval_seconds: 60,
                    retention_hours: 24,
                },
            )
            .unwrap()
            .id
        };
        let config = load_monitor_run_config(&db.conn(), &monitor_id).unwrap();
        let observed_at = chrono::Utc::now().to_rfc3339();
        let old_observed_at = (chrono::Utc::now() - chrono::Duration::hours(25)).to_rfc3339();
        db.conn()
            .execute(
                "INSERT INTO monitor_samples (monitor_id, observed_at, status, duration_ms)
             VALUES (?1, ?2, 'healthy', 1)",
                rusqlite::params![monitor_id, old_observed_at],
            )
            .unwrap();
        let down = parse_interfaces(
            r#"{"interfaces":{"Ethernet1":{"adminStatus":"enabled","lineProtocolStatus":"down","interfaceCounters":{"inErrors":2}}}}"#,
        ).unwrap();
        persist_sample(&db.conn(), &config, &observed_at, 12, "{}", &down, 2).unwrap();
        persist_sample(&db.conn(), &config, &observed_at, 13, "{}", &down, 2).unwrap();
        let healthy = parse_interfaces(
            r#"{"interfaces":{"Ethernet1":{"adminStatus":"enabled","lineProtocolStatus":"up","interfaceCounters":{}}}}"#,
        ).unwrap();
        persist_sample(&db.conn(), &config, &observed_at, 14, "{}", &healthy, 0).unwrap();

        let conn = db.conn();
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM monitor_samples WHERE monitor_id = ?1",
                rusqlite::params![monitor_id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            3
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM interface_samples WHERE interface_name = 'Ethernet1'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            3
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM monitor_incidents
                 WHERE monitor_id = ?1 AND state = 'resolved' AND occurrence_count = 2",
                rusqlite::params![monitor_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            2
        );
        assert_eq!(
            conn.query_row(
                "SELECT json_extract(summary_json, '$.logCount') FROM monitor_samples
                 WHERE monitor_id = ?1 ORDER BY id LIMIT 1",
                rusqlite::params![monitor_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            2
        );
        drop(conn);

        let long_error = AppError::Validation("x".repeat(3_000));
        persist_failed_sample(&db.conn(), &config, Instant::now(), &long_error).unwrap();
        let failure: (String, i64) = db.conn().query_row(
            "SELECT status, length(error) FROM monitor_samples WHERE monitor_id = ?1 ORDER BY id DESC LIMIT 1",
            rusqlite::params![monitor_id], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
        assert_eq!(failure, ("failed".into(), 2_048));

        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn failed_run_persists_one_failed_sample() {
        let path = std::env::temp_dir().join(format!(
            "sessiondock-monitor-failure-{}.db",
            uuid::Uuid::new_v4()
        ));
        let db = Database::new(&path).unwrap();
        let session_id = uuid::Uuid::new_v4().to_string();
        let monitor_id = {
            let conn = db.conn();
            conn.execute(
                "INSERT INTO sessions (id, name, host, port, protocol, authentication_method, created_at, updated_at)
                 VALUES (?1, 'Incomplete', '192.0.2.10', 22, 'ssh', 'agent', datetime('now'), datetime('now'))",
                rusqlite::params![session_id],
            ).unwrap();
            create_monitor(
                &conn,
                &CreateMonitorRequest {
                    session_id,
                    provider_id: "arista-eos".into(),
                    monitor_kind: "interfaces".into(),
                    interface_name: "Ethernet1".into(),
                    display_name: "Incomplete monitor".into(),
                    interval_seconds: 60,
                    retention_hours: 24,
                },
            )
            .unwrap()
            .id
        };

        assert!(run_monitor_by_id(&db, &monitor_id).await.is_err());
        assert_eq!(
            db.conn().query_row(
                "SELECT COUNT(*) FROM monitor_samples WHERE monitor_id = ?1 AND status = 'failed'",
                rusqlite::params![monitor_id],
                |row| row.get::<_, i64>(0),
            ).unwrap(),
            1
        );

        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}
