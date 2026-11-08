use std::{ffi::OsString, fs::File, io, path::Path, sync::Mutex};
use tracing_subscriber::{
    layer::{Layer, SubscriberExt},
    util::SubscriberInitExt,
};

fn subscriber(file: File) -> impl tracing::Subscriber + Send + Sync {
    tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .with_writer(Mutex::new(file))
            .with_ansi(false)
            .with_timer(tracing_subscriber::fmt::time::uptime())
            .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
                metadata.target() == "dasru::shutdown"
            })),
    )
}

pub fn configure(mut arguments: impl Iterator<Item = OsString>) -> io::Result<()> {
    let Some(option) = arguments.next() else {
        return Ok(());
    };
    let path = arguments
        .next()
        .ok_or_else(|| io::Error::other("SHUTDOWN_TRACE_PATH_REQUIRED"))?;
    if option != "--shutdown-trace" || arguments.next().is_some() {
        return Err(io::Error::other("GUI_ARGUMENT_INVALID"));
    }
    let file = File::create_new(Path::new(&path))?;
    subscriber(file)
        .try_init()
        .map_err(|_| io::Error::other("SHUTDOWN_TRACE_INIT_FAILED"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_contains_phases_without_unrelated_sensitive_events() {
        let path =
            std::env::temp_dir().join(format!("dasru-shutdown-trace-{}.log", std::process::id()));
        let file = File::create(&path).unwrap();
        tracing::subscriber::with_default(subscriber(file), || {
            tracing::info!(target: "other-service", private_path = "sensitive-fixture");
            tracing::info!(target: "dasru::shutdown", phase = "SHUTDOWN_REQUESTED");
        });
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(text.contains("SHUTDOWN_REQUESTED"));
        assert!(
            !text.contains("sensitive-fixture"),
            "unrelated sensitive event was logged"
        );
    }

    #[test]
    fn invalid_trace_arguments_are_rejected_without_files() {
        assert!(configure(std::iter::empty()).is_ok());
        for arguments in [
            vec!["--shutdown-trace"],
            vec!["--invalid", "ignored"],
            vec!["--shutdown-trace", "ignored", "extra"],
        ] {
            assert!(configure(arguments.into_iter().map(OsString::from)).is_err());
        }
    }

    #[test]
    fn trace_refuses_existing_destination() {
        let path = std::env::temp_dir().join(format!(
            "dasru-shutdown-no-overwrite-{}.log",
            std::process::id()
        ));
        std::fs::write(&path, b"historical evidence").unwrap();
        let result = configure(
            [
                OsString::from("--shutdown-trace"),
                path.clone().into_os_string(),
            ]
            .into_iter(),
        );
        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(result.is_err());
        assert_eq!(bytes, b"historical evidence");
    }

    #[test]
    fn app_shutdown_has_one_terminal_phase_on_repeated_close() {
        let path =
            std::env::temp_dir().join(format!("dasru-app-phases-{}.log", std::process::id()));
        let file = File::create(&path).unwrap();
        tracing::subscriber::with_default(subscriber(file), || {
            let mut app = super::super::DesktopApp::default();
            app.shutdown().unwrap();
            app.shutdown().unwrap();
        });
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(text.matches("SHUTDOWN_REQUESTED").count(), 1);
        assert_eq!(text.matches("APP_SHUTDOWN_COMPLETED").count(), 1);
    }

    #[test]
    fn controller_shutdown_phases_are_not_repeated_by_drop() {
        struct NoGame;
        impl super::super::controller::MonitorService for NoGame {
            fn observe(
                &mut self,
                _: &std::sync::atomic::AtomicBool,
            ) -> Result<super::super::model::Observation, &'static str> {
                Ok(super::super::model::Observation::new(
                    crate::probe::ProbeStatus::NoProcess,
                ))
            }
        }
        let path = std::env::temp_dir().join(format!(
            "dasru-controller-phases-{}.log",
            std::process::id()
        ));
        tracing::subscriber::with_default(subscriber(File::create(&path).unwrap()), || {
            let mut controller = super::super::controller::Controller::start(
                NoGame,
                std::time::Duration::from_secs(30),
            )
            .unwrap();
            controller.shutdown().unwrap();
            controller.shutdown().unwrap();
        });
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(text.matches("MONITOR_CANCEL_REQUESTED").count(), 1);
        assert_eq!(text.matches("HOTKEY_STOP_REQUESTED").count(), 1);
    }
}
