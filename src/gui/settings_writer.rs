use super::settings::{Preferences, SettingsStore};
use std::{
    sync::{
        Arc, Mutex,
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WriteStatus {
    #[default]
    Idle,
    Pending,
    Saved,
    Failed(&'static str),
}
pub struct SettingsWriter {
    sender: Option<SyncSender<Preferences>>,
    worker: Option<JoinHandle<()>>,
    status: Arc<Mutex<WriteStatus>>,
}
impl SettingsWriter {
    pub fn start(mut store: SettingsStore) -> std::io::Result<Self> {
        Self::spawn(move |preferences| store.save(preferences))
    }
    fn spawn(
        mut save: impl FnMut(&Preferences) -> Result<(), &'static str> + Send + 'static,
    ) -> std::io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let status = Arc::new(Mutex::new(WriteStatus::Idle));
        let state = status.clone();
        let worker = thread::Builder::new()
            .name("dasru-preferences".into())
            .spawn(move || {
                for preferences in receiver {
                    let result = match save(&preferences) {
                        Ok(()) => WriteStatus::Saved,
                        Err(error) => WriteStatus::Failed(error),
                    };
                    *state.lock().unwrap_or_else(|e| e.into_inner()) = result;
                }
            })?;
        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
            status,
        })
    }
    pub fn status(&self) -> WriteStatus {
        *self.status.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub fn save(&self, preferences: Preferences) -> Result<(), &'static str> {
        preferences.validate()?;
        let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
        if *status == WriteStatus::Pending {
            return Err("SETTINGS_SAVE_PENDING");
        }
        self.sender
            .as_ref()
            .ok_or("SETTINGS_WRITER_STOPPED")?
            .try_send(preferences)
            .map_err(|_| "SETTINGS_SAVE_PENDING")?;
        *status = WriteStatus::Pending;
        Ok(())
    }
    pub fn finish(&mut self, preferences: Preferences) -> Result<(), &'static str> {
        preferences.validate()?;
        if let Some(sender) = self.sender.take() {
            sender
                .send(preferences)
                .map_err(|_| "SETTINGS_WRITER_STOPPED")?;
        }
        self.join()?;
        match self.status() {
            WriteStatus::Failed(error) => Err(error),
            _ => Ok(()),
        }
    }
    fn join(&mut self) -> Result<(), &'static str> {
        if let Some(worker) = self.worker.take() {
            worker.join().map_err(|_| "SETTINGS_WORKER_FAILED")?;
        }
        Ok(())
    }
}
impl Drop for SettingsWriter {
    fn drop(&mut self) {
        self.sender.take();
        let _ = self.join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn finish_drains_full_queue_and_persists_final_preferences() {
        let (entered, entry) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let (saved, writes) = mpsc::channel();
        let mut first = true;
        let mut writer = SettingsWriter::spawn(move |preferences| {
            if first {
                first = false;
                entered.send(()).unwrap();
                gate.recv_timeout(Duration::from_secs(2)).unwrap();
            }
            saved.send(preferences.clone()).unwrap();
            Ok(())
        })
        .unwrap();
        writer.save(Preferences::default()).unwrap();
        entry.recv_timeout(Duration::from_secs(2)).unwrap();
        writer
            .sender
            .as_ref()
            .unwrap()
            .send(Preferences::default())
            .unwrap();
        let final_preferences = Preferences {
            refresh_seconds: 7,
            ..Default::default()
        };
        let expected = final_preferences.clone();
        let (done, completion) = mpsc::channel();
        let owner = thread::spawn(move || {
            let result = writer.finish(final_preferences);
            done.send(result).unwrap();
        });
        let early = completion.recv_timeout(Duration::from_millis(100)).ok();
        release.send(()).unwrap();
        owner.join().unwrap();
        assert!(early.is_none(), "settings owner escaped before flush");
        assert_eq!(completion.recv().unwrap(), Ok(()));
        assert_eq!(writes.try_iter().last(), Some(expected));
    }

    #[test]
    fn finish_reports_failed_write_and_worker_panic() {
        let mut writer = SettingsWriter::spawn(|_| Err("SETTINGS_WRITE_FAILED")).unwrap();
        assert_eq!(
            writer.finish(Preferences::default()),
            Err("SETTINGS_WRITE_FAILED")
        );
        let mut writer = SettingsWriter::spawn(|_| panic!("controlled settings fault")).unwrap();
        assert_eq!(
            writer.finish(Preferences::default()),
            Err("SETTINGS_WORKER_FAILED")
        );
    }
}
