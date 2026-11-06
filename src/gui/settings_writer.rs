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
        let (sender, receiver) = mpsc::sync_channel(1);
        let status = Arc::new(Mutex::new(WriteStatus::Idle));
        let state = status.clone();
        let worker = thread::Builder::new()
            .name("dasru-preferences".into())
            .spawn(move || {
                for preferences in receiver {
                    let result = match store.save(&preferences) {
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
