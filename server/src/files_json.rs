use chrono::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::state::AppState;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ReceivedFile {
    pub file_name: String,
    pub file_size: u64,
    pub file_extension: String,
    pub download_url: String,
    pub download_time: DateTime<Local>,
    pub connection_type: String,
    pub peer_address: SocketAddr,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SentFile {
    pub file_name: String,
    pub file_size: u64,
    pub file_extension: String,
    pub file_paths: Vec<String>,
    pub send_time: DateTime<Local>,
    pub connection_code: String,
}

fn received_files_path(data_dir: &Path) -> PathBuf {
    data_dir.join("received_files.json")
}

fn sent_files_path(data_dir: &Path) -> PathBuf {
    data_dir.join("sent_files.json")
}

pub fn load_received_files(data_dir: &Path) -> Vec<ReceivedFile> {
    let path = received_files_path(data_dir);
    if path.exists()
        && let Ok(content) = fs::read_to_string(&path)
        && let Ok(files) = serde_json::from_str::<Vec<ReceivedFile>>(&content)
    {
        return files;
    }
    let _ = fs::write(&path, "[]");
    vec![]
}

pub fn load_sent_files(data_dir: &Path) -> Vec<SentFile> {
    let path = sent_files_path(data_dir);
    if path.exists()
        && let Ok(content) = fs::read_to_string(&path)
        && let Ok(files) = serde_json::from_str::<Vec<SentFile>>(&content)
    {
        return files;
    }
    let _ = fs::write(&path, "[]");
    vec![]
}

pub fn add_received_file(state: &Arc<AppState>, new_file: ReceivedFile) -> Result<(), String> {
    let path = received_files_path(&state.data_dir);
    let mut files = load_received_files(&state.data_dir);
    files.push(new_file);
    let json = serde_json::to_string_pretty(&files).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())?;
    state.emit("received-file-added", serde_json::json!({}));
    Ok(())
}

pub fn add_sent_file(state: &Arc<AppState>, new_file: SentFile) -> Result<(), String> {
    let path = sent_files_path(&state.data_dir);
    let mut files = load_sent_files(&state.data_dir);
    files.push(new_file);
    let json = serde_json::to_string_pretty(&files).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())?;
    state.emit("sent-file-added", serde_json::json!({}));
    Ok(())
}

pub fn get_received_files_json(data_dir: &Path) -> Vec<serde_json::Value> {
    let path = received_files_path(data_dir);
    if let Ok(content) = fs::read_to_string(&path)
        && let Ok(files) = serde_json::from_str::<Vec<serde_json::Value>>(&content)
    {
        return files;
    }
    vec![]
}

pub fn get_sent_files_json(data_dir: &Path) -> Vec<serde_json::Value> {
    let path = sent_files_path(data_dir);
    if let Ok(content) = fs::read_to_string(&path)
        && let Ok(files) = serde_json::from_str::<Vec<serde_json::Value>>(&content)
    {
        return files;
    }
    vec![]
}
