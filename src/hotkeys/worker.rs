use std::{sync::mpsc::Receiver, thread::JoinHandle, time::Duration};

use super::HotkeyErrorCode;

pub(super) fn finish_shutdown(
    worker: &mut Option<JoinHandle<()>>,
    result: Result<Result<(), HotkeyErrorCode>, HotkeyErrorCode>,
) -> Result<(), HotkeyErrorCode> {
    if worker.take().is_some_and(|worker| worker.join().is_err()) {
        return Err(HotkeyErrorCode::ShutdownFailed);
    }
    result.and_then(|result| result)
}

pub(super) fn finish_startup_abort(
    worker: JoinHandle<()>,
    completed: Receiver<Result<(), HotkeyErrorCode>>,
    timeout: Duration,
) {
    let _ = completed.recv_timeout(timeout);
    // A notification deadline cannot release ownership of a live input thread.
    let _ = worker.join();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, thread};

    fn assert_abort_retains_owner(disconnect: bool) {
        let (release, gate) = mpsc::channel();
        let (released, resource_released) = mpsc::channel();
        let worker = thread::spawn(move || {
            gate.recv().unwrap();
            released.send(()).unwrap();
        });
        let (completion, completed) = mpsc::channel();
        let completion = if disconnect { None } else { Some(completion) };
        let (returned, result) = mpsc::channel();
        let (entered, entry) = mpsc::channel();
        let owner = thread::spawn(move || {
            entered.send(()).unwrap();
            finish_startup_abort(worker, completed, Duration::ZERO);
            returned.send(()).unwrap();
        });
        entry.recv().unwrap();
        let returned_before_release = result.recv_timeout(Duration::from_millis(100)).is_ok();
        release.send(()).unwrap();
        resource_released.recv().unwrap();
        owner.join().unwrap();
        drop(completion);
        assert!(
            !returned_before_release,
            "cleanup detached a live resource owner"
        );
    }

    #[test]
    fn startup_abort_joins_after_completion_disconnect() {
        assert_abort_retains_owner(true);
    }

    #[test]
    fn startup_abort_joins_after_completion_timeout() {
        assert_abort_retains_owner(false);
    }

    fn assert_shutdown_retains_owner(error: HotkeyErrorCode) {
        let (release, gate) = mpsc::channel();
        let worker = thread::spawn(move || gate.recv().unwrap());
        let (entered, entry) = mpsc::channel();
        let (returned, result) = mpsc::channel();
        let owner = thread::spawn(move || {
            let mut worker = Some(worker);
            entered.send(()).unwrap();
            let outcome = finish_shutdown(&mut worker, Err(error));
            returned.send((outcome, worker.is_none())).unwrap();
            if let Some(worker) = worker {
                worker.join().unwrap();
            }
        });
        entry.recv().unwrap();
        let early = result.recv_timeout(Duration::from_millis(100)).ok();
        release.send(()).unwrap();
        owner.join().unwrap();
        let early_return = early.is_some();
        let (outcome, joined) = early.unwrap_or_else(|| result.recv().unwrap());
        assert_eq!(outcome, Err(error), "cleanup error was lost");
        assert!(
            !early_return && joined,
            "shutdown returned with a live owner"
        );
    }

    #[test]
    fn shutdown_joins_owner_after_notification_failure() {
        assert_shutdown_retains_owner(HotkeyErrorCode::ShutdownFailed);
    }

    #[test]
    fn shutdown_joins_owner_after_notification_timeout() {
        assert_shutdown_retains_owner(HotkeyErrorCode::ShutdownTimeout);
    }

    #[test]
    fn shutdown_preserves_backend_cleanup_failure() {
        let mut worker = Some(thread::spawn(|| {}));
        assert_eq!(
            finish_shutdown(&mut worker, Ok(Err(HotkeyErrorCode::ShutdownFailed))),
            Err(HotkeyErrorCode::ShutdownFailed)
        );
        assert!(worker.is_none());
    }

    #[test]
    fn shutdown_reports_owner_panic() {
        let mut worker = Some(thread::spawn(|| panic!("controlled worker failure")));
        assert_eq!(
            finish_shutdown(&mut worker, Ok(Ok(()))),
            Err(HotkeyErrorCode::ShutdownFailed)
        );
        assert!(worker.is_none());
    }
}
