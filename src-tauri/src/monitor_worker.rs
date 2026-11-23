use std::fs::OpenOptions;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use fs2::FileExt;

use crate::{commands::monitoring::run_monitor_by_id, AppError, Database};

const WORKER_POLL_INTERVAL: Duration = Duration::from_secs(1);
const MAX_CONCURRENT_MONITORS: usize = 4;
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

    let database = Arc::new(Database::new(&app_data_dir.join("sessiondock.db"))?);
    let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_MONITORS));
    let mut jobs = tokio::task::JoinSet::new();
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
        reap_finished_jobs(&mut jobs);
        let run_database = database.clone();
        schedule_monitor_jobs(
            &mut jobs,
            semaphore.clone(),
            due_monitor_ids,
            move |monitor_id| {
                let database = run_database.clone();
                async move {
                    if let Err(error) = run_monitor_by_id(&database, &monitor_id).await {
                        log::warn!("Monitor {monitor_id} failed: {error}");
                    }
                }
            },
        );
        if let Err(error) = crate::monitor_alerts::dispatch_pending(&database) {
            log::error!("Could not process monitor notifications: {error}");
        }
        if wait_for_poll_or_shutdown(&app_data_dir).await? {
            log::info!("SessionDock monitor worker stopped for application update");
            return Ok(());
        }
    }
}

fn schedule_monitor_jobs<F, Fut>(
    jobs: &mut tokio::task::JoinSet<()>,
    semaphore: Arc<tokio::sync::Semaphore>,
    monitor_ids: Vec<String>,
    run: F,
) -> usize
where
    F: Fn(String) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut scheduled = 0;
    for monitor_id in monitor_ids {
        let permit = match semaphore.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => break,
        };
        let run = run.clone();
        jobs.spawn(async move {
            let _permit = permit;
            run(monitor_id).await;
        });
        scheduled += 1;
    }
    scheduled
}

fn reap_finished_jobs(jobs: &mut tokio::task::JoinSet<()>) {
    while let Some(result) = jobs.try_join_next() {
        if let Err(error) = result {
            log::error!("Monitor task failed unexpectedly: {error}");
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
    let stale_before = (chrono::Utc::now() - chrono::Duration::minutes(5)).to_rfc3339();
    let mut statement = conn.prepare(
        "SELECT m.id
         FROM monitor_definitions m
         LEFT JOIN monitor_run_requests request ON request.monitor_id = m.id
         LEFT JOIN monitor_samples latest ON latest.id = (
             SELECT id FROM monitor_samples WHERE monitor_id = m.id ORDER BY observed_at DESC, id DESC LIMIT 1
         )
         WHERE m.enabled = 1
                     AND (request.monitor_id IS NOT NULL
                                OR (m.last_started_at IS NULL AND latest.id IS NULL)
                                OR julianday(COALESCE(m.last_started_at, latest.observed_at)) <= julianday('now', '-' || m.interval_seconds || ' seconds'))
                     AND NOT EXISTS (
                             SELECT 1 FROM monitor_run_leases lease
                             WHERE lease.session_id = m.session_id AND lease.started_at > ?1
                     )
         ORDER BY request.requested_at IS NULL, request.requested_at, m.updated_at, m.id
         LIMIT 100",
    )?;
    let ids = statement.query_map([stale_before], |row| row.get(0))?;
    ids.collect::<Result<Vec<_>, _>>()
        .map_err(AppError::Database)
}

#[cfg(test)]
mod tests {
    use super::{
        due_monitors, request_shutdown_inner, schedule_monitor_jobs, validate_app_data_dir,
    };
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

    #[test]
    fn interval_changes_take_effect_and_active_sessions_are_not_scheduled() {
        let path = std::env::temp_dir().join(format!("sessiondock-due-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        let db = Database::new(&path.join("sessiondock.db")).unwrap();
        let session_id = uuid::Uuid::new_v4().to_string();
        let monitor_id = uuid::Uuid::new_v4().to_string();
        {
            let conn = db.conn();
            conn.execute(
                "INSERT INTO sessions (id, name, host, protocol, authentication_method, created_at, updated_at)
                 VALUES (?1, 'Router', '192.0.2.20', 'ssh', 'agent', datetime('now'), datetime('now'))",
                rusqlite::params![session_id],
            ).unwrap();
            conn.execute(
                "INSERT INTO monitor_definitions
                 (id, session_id, provider_id, monitor_kind, interface_name, display_name, interval_seconds, last_started_at)
                 VALUES (?1, ?2, 'cisco-ios', 'interfaces', 'GigabitEthernet1/0/1', 'WAN', 60, datetime('now', '-40 seconds'))",
                rusqlite::params![monitor_id, session_id],
            ).unwrap();
            conn.execute(
                "INSERT INTO monitor_samples (monitor_id, observed_at, status, duration_ms)
                 VALUES (?1, datetime('now'), 'healthy', 40000)",
                rusqlite::params![monitor_id],
            )
            .unwrap();
        }
        assert!(due_monitors(&db).unwrap().is_empty());

        db.conn()
            .execute(
                "DELETE FROM monitor_run_leases WHERE session_id = ?1",
                rusqlite::params![session_id],
            )
            .unwrap();
        db.conn()
            .execute(
                "UPDATE monitor_definitions SET interval_seconds = 86400 WHERE id = ?1",
                rusqlite::params![monitor_id],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO monitor_run_requests (monitor_id, requested_at) VALUES (?1, ?2)",
                rusqlite::params![monitor_id, chrono::Utc::now().to_rfc3339()],
            )
            .unwrap();
        assert_eq!(due_monitors(&db).unwrap(), vec![monitor_id.clone()]);

        db.conn()
            .execute(
                "UPDATE monitor_definitions SET interval_seconds = 30 WHERE id = ?1",
                rusqlite::params![monitor_id],
            )
            .unwrap();
        assert_eq!(due_monitors(&db).unwrap(), vec![monitor_id.clone()]);

        db.conn()
            .execute(
                "INSERT INTO monitor_run_leases (session_id, monitor_id, started_at)
             VALUES (?1, ?2, ?3)",
                rusqlite::params![session_id, monitor_id, chrono::Utc::now().to_rfc3339()],
            )
            .unwrap();
        assert!(due_monitors(&db).unwrap().is_empty());

        drop(db);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn scheduler_does_not_wait_for_running_monitor_jobs() {
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(1));
        let release = std::sync::Arc::new(tokio::sync::Notify::new());
        let started = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut jobs = tokio::task::JoinSet::new();
        let run_release = release.clone();
        let run_started = started.clone();

        let scheduled = schedule_monitor_jobs(
            &mut jobs,
            semaphore,
            vec!["first".into(), "second".into()],
            move |_| {
                let release = run_release.clone();
                let started = run_started.clone();
                async move {
                    started.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    release.notified().await;
                }
            },
        );

        assert_eq!(scheduled, 1);
        tokio::task::yield_now().await;
        assert_eq!(started.load(std::sync::atomic::Ordering::SeqCst), 1);
        release.notify_waiters();
        assert!(jobs.join_next().await.unwrap().is_ok());
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
