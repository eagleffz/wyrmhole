use std::path::PathBuf;
use tokio::sync::{broadcast, Mutex};
use crate::settings::AppSettings;

pub struct AppState {
    pub settings: Mutex<AppSettings>,
    pub event_tx: broadcast::Sender<String>,
    pub data_dir: PathBuf,
}

impl AppState {
    pub fn new(settings: AppSettings, data_dir: PathBuf) -> Self {
        let (tx, _) = broadcast::channel(512);
        Self {
            settings: Mutex::new(settings),
            event_tx: tx,
            data_dir,
        }
    }

    pub fn emit(&self, event_type: &str, payload: serde_json::Value) {
        let msg = serde_json::json!({ "type": event_type, "payload": payload });
        let _ = self.event_tx.send(msg.to_string());
    }
}
