use std::{sync::mpsc::Receiver, thread::JoinHandle, time::Duration};

use super::HotkeyErrorCode;

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
}
