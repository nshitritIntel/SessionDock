#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    env_logger::init();
    let mut arguments = std::env::args_os();
    let _executable = arguments.next();
    if arguments.next().as_deref() == Some(std::ffi::OsStr::new("--monitor-worker")) {
        let Some(app_data_dir) = arguments.next() else {
            log::error!("Monitor worker requires an app data directory");
            return;
        };
        let runtime =
            tokio::runtime::Runtime::new().expect("Failed to start monitor worker runtime");
        if let Err(error) =
            runtime.block_on(sessiondock_lib::run_monitor_worker(app_data_dir.into()))
        {
            log::error!("Monitor worker stopped: {error}");
        }
        return;
    }
    sessiondock_lib::run();
}
