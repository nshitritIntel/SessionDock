use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::time::Duration;

use fs2::FileExt;

use crate::{commands::monitoring::run_monitor_by_id, AppError, Database};

const WORKER_POLL_INTERVAL: Duration = Duration::from_secs(15);
const WORKER_SHUTDOWN_FILE: &str = "monitor-worker.shutdown";

pub async fn run(app_data_dir: PathBuf) -> Result<(), AppError> {
    validate_app_data_dir(&app_data_dir)?;
    std::fs::create_dir_all(&app_data_dir)?;
    let lock_path = app_data_dir.join("monitor-worker.lock");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    if lock.try_lock_exclusive().is_err() {
        log::info!("A SessionDock monitor worker is already running");
        return Ok(());
    }

    let database = Database::new(&app_data_dir.join("sessiondock.db"))?;
    log::info!("SessionDock monitor worker started");
    loop {
        if take_shutdown_request(&app_data_dir)? {
            log::info!("SessionDock monitor worker stopped for application update");
            return Ok(());
        }
        let due_monitor_ids = match due_monitors(&database) {
            Ok(ids) => ids,
            Err(error) => {
                log::error!("Could not load due monitors: {error}");
                tokio::time::sleep(WORKER_POLL_INTERVAL).await;
                continue;
            }
        };
        for monitor_id in due_monitor_ids {
            if take_shutdown_request(&app_data_dir)? {
                log::info!("SessionDock monitor worker stopped for application update");
                return Ok(());
            }
            if let Err(error) = run_monitor_by_id(&database, &monitor_id).await {
                log::warn!("Monitor {monitor_id} failed: {error}");
            }
        }
        if let Err(error) = crate::monitor_alerts::dispatch_pending(&database) {
            log::error!("Could not process monitor notifications: {error}");
        }
        if wait_for_poll_or_shutdown(&app_data_dir).await? {
            log::info!("SessionDock monitor worker stopped for application update");
            return Ok(());
        }
    }
}

pub fn clear_shutdown_request(app_data_dir: &Path) -> Result<(), AppError> {
    match std::fs::remove_file(app_data_dir.join(WORKER_SHUTDOWN_FILE)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub async fn request_shutdown(app_data_dir: &Path) -> Result<(), AppError> {
    validate_app_data_dir(app_data_dir)?;
    request_shutdown_inner(app_data_dir).await
}

async fn request_shutdown_inner(app_data_dir: &Path) -> Result<(), AppError> {
    std::fs::write(app_data_dir.join(WORKER_SHUTDOWN_FILE), [])?;
    let lock_path = app_data_dir.join("monitor-worker.lock");
    for _ in 0..300 {
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)?;
        if lock.try_lock_exclusive().is_ok() {
            tokio::time::sleep(Duration::from_millis(500)).await;
            clear_shutdown_request(app_data_dir)?;
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err(AppError::Generic(
        "Timed out waiting for the monitor worker to stop".into(),
    ))
}

fn take_shutdown_request(app_data_dir: &Path) -> Result<bool, AppError> {
    let path = app_data_dir.join(WORKER_SHUTDOWN_FILE);
    if !path.exists() {
        return Ok(false);
    }
    clear_shutdown_request(app_data_dir)?;
    Ok(true)
}

async fn wait_for_poll_or_shutdown(app_data_dir: &Path) -> Result<bool, AppError> {
    let checks = WORKER_POLL_INTERVAL.as_secs();
    for _ in 0..checks {
        if take_shutdown_request(app_data_dir)? {
            return Ok(true);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Ok(false)
}

fn validate_app_data_dir(path: &Path) -> Result<(), AppError> {
    let expected = dirs::data_dir()
        .ok_or_else(|| AppError::Validation("User data directory is unavailable".into()))?
        .join("com.sessiondock.app");
    if path != expected {
        return Err(AppError::Validation(
            "Monitor worker data directory is not the SessionDock app directory".into(),
        ));
    }
    Ok(())
}

fn due_monitors(database: &Database) -> Result<Vec<String>, AppError> {
    let conn = database.conn();
    let mut statement = conn.prepare(
        "SELECT m.id
         FROM monitor_definitions m
         LEFT JOIN monitor_samples latest ON latest.id = (
             SELECT id FROM monitor_samples WHERE monitor_id = m.id ORDER BY observed_at DESC, id DESC LIMIT 1
         )
         WHERE m.enabled = 1
           AND (latest.id IS NULL OR julianday(latest.observed_at) <= julianday('now', '-' || m.interval_seconds || ' seconds'))
         ORDER BY m.updated_at, m.id
         LIMIT 100",
    )?;
    let ids = statement.query_map([], |row| row.get(0))?;
    ids.collect::<Result<Vec<_>, _>>()
        .map_err(AppError::Database)
}

#[cfg(test)]
mod tests {
    use super::{due_monitors, request_shutdown_inner, validate_app_data_dir};
    use crate::Database;

    #[test]
    fn validates_worker_paths_and_selects_only_due_enabled_monitors() {
        assert!(validate_app_data_dir(std::path::Path::new("relative")).is_err());
        let path =
            std::env::temp_dir().join(format!("sessiondock-worker-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        let db = Database::new(&path.join("sessiondock.db")).unwrap();
        let session_id = uuid::Uuid::new_v4().to_string();
        let disabled_session_id = uuid::Uuid::new_v4().to_string();
        let due_id = uuid::Uuid::new_v4().to_string();
        let disabled_id = uuid::Uuid::new_v4().to_string();
        {
            let conn = db.conn();
            conn.execute(
                "INSERT INTO sessions (id, name, host, protocol, authentication_method, created_at, updated_at)
                 VALUES (?1, 'Leaf', '192.0.2.10', 'ssh', 'password', datetime('now'), datetime('now'))",
                rusqlite::params![session_id],
            ).unwrap();
            conn.execute(
                "INSERT INTO sessions (id, name, host, protocol, authentication_method, created_at, updated_at)
                 VALUES (?1, 'Spine', '192.0.2.11', 'ssh', 'password', datetime('now'), datetime('now'))",
                rusqlite::params![disabled_session_id],
            ).unwrap();
            for (id, monitor_session_id, enabled) in [
                (&due_id, &session_id, 1),
                (&disabled_id, &disabled_session_id, 0),
            ] {
                conn.execute(
                    "INSERT INTO monitor_definitions
                     (id, session_id, provider_id, monitor_kind, display_name, interval_seconds, enabled)
                     VALUES (?1, ?2, 'arista-eos', 'interfaces', ?1, 60, ?3)",
                    rusqlite::params![id, monitor_session_id, enabled],
                ).unwrap();
            }
        }
        assert_eq!(due_monitors(&db).unwrap(), vec![due_id]);
        drop(db);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn shutdown_request_completes_when_no_worker_holds_the_lock() {
        let path =
            std::env::temp_dir().join(format!("sessiondock-worker-stop-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();

        request_shutdown_inner(&path).await.unwrap();

        assert!(!path.join("monitor-worker.shutdown").exists());
        std::fs::remove_dir_all(path).unwrap();
    }
}
