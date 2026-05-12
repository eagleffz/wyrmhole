use chrono::prelude::*;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use futures::FutureExt;
use magic_wormhole::{transfer, transit, Code, MailboxConnection, Wormhole, WormholeError};
use once_cell::sync::Lazy;
use std::{collections::HashMap, net::SocketAddr, path::Path, path::PathBuf, time::Instant};
use tar::{Archive, Builder};
use tokio::fs::File;
use tokio::sync::{oneshot, Mutex};
use tokio_util::compat::TokioAsyncReadCompatExt;
use tokio_util::compat::TokioAsyncWriteCompatExt;
use uuid::Uuid;
use std::sync::Arc;

use crate::files_json;
use crate::state::AppState;

struct OpenRequests {
    request: transfer::ReceiveRequest,
}

struct ActiveSend {
    code: String,
    cancel_tx: Option<oneshot::Sender<()>>,
}

#[allow(dead_code)]
struct ActiveDownload {
    cancel_tx: oneshot::Sender<()>,
    file_name: String,
}

struct ActiveConnection {
    cancel_tx: oneshot::Sender<()>,
}

static REQUESTS_HASHMAP: Lazy<Mutex<HashMap<String, OpenRequests>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

static ACTIVE_SENDS: Lazy<Mutex<HashMap<String, ActiveSend>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

static ACTIVE_DOWNLOADS: Lazy<Mutex<HashMap<String, ActiveDownload>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

static ACTIVE_CONNECTIONS: Lazy<Mutex<HashMap<String, ActiveConnection>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub async fn send_file_call(
    state: Arc<AppState>,
    file_path: &str,
    file_name: String,
    send_id: String,
) -> Result<String, String> {
    let overall_start = Instant::now();
    let config = transfer::APP_CONFIG.clone();

    state.emit(
        "send-progress",
        serde_json::json!({
            "id": send_id,
            "file_name": file_name,
            "sent": 0,
            "total": 0,
            "percentage": 0,
            "code": "",
            "status": "preparing"
        }),
    );

    let (cancel_tx, cancel_rx) = oneshot::channel::<()>();

    let mailbox_connection = match MailboxConnection::create(config, 2).await {
        Ok(conn) => {
            let code_string = conn.code().to_string();
            ACTIVE_SENDS.lock().await.insert(
                send_id.clone(),
                ActiveSend {
                    code: code_string.clone(),
                    cancel_tx: Some(cancel_tx),
                },
            );
            state.emit(
                "connection-code",
                serde_json::json!({
                    "status": "success",
                    "code": code_string,
                    "send_id": send_id
                }),
            );
            state.emit(
                "send-progress",
                serde_json::json!({
                    "id": send_id,
                    "file_name": file_name,
                    "sent": 0,
                    "total": 0,
                    "percentage": 0,
                    "code": conn.code().to_string(),
                    "status": "waiting"
                }),
            );
            conn
        }
        Err(e) => {
            let error_msg = format!("Failed to connect: {}", e);
            state.emit("connection-code", serde_json::json!({ "status": "error", "message": error_msg.clone() }));
            state.emit("send-error", serde_json::json!({ "id": send_id, "file_name": file_name, "error": error_msg.clone() }));
            return Err(error_msg);
        }
    };

    let relay_hints = build_relay_hints(&state).await;
    let abilities = transit::Abilities::ALL;
    let cancel_call = cancel_rx.map(|_| ());

    let wormhole = Wormhole::connect(mailbox_connection).await.map_err(|e| {
        let msg = format!("Failed to connect to Wormhole: {}", e);
        state.emit("send-error", serde_json::json!({ "id": send_id.clone(), "file_name": file_name.clone(), "error": msg.clone() }));
        msg
    })?;

    let path = Path::new(file_path);
    if !path.exists() {
        let error_msg = format!("File does not exist: {}", file_path);
        state.emit("send-error", serde_json::json!({ "id": send_id.clone(), "file_name": file_name.clone(), "error": error_msg.clone() }));
        return Err(error_msg);
    }

    let file_size = std::fs::metadata(path)
        .map_err(|e| format!("Failed to get file metadata: {}", e))?
        .len();

    let file = File::open(path).await.map_err(|e| {
        let msg = format!("Failed to open file: {}", e);
        state.emit("send-error", serde_json::json!({ "id": send_id.clone(), "file_name": file_name.clone(), "error": msg.clone() }));
        msg
    })?;

    let mut compat_file = file.compat();

    let send_code = {
        let active_sends = ACTIVE_SENDS.lock().await;
        active_sends.get(&send_id).map(|s| s.code.clone()).unwrap_or_default()
    };

    let progress_id = send_id.clone();
    let progress_file_name = file_name.clone();
    let progress_state = state.clone();
    let progress_code = send_code.clone();

    let error_state = state.clone();
    let error_id = send_id.clone();
    let error_file_name = file_name.clone();

    let transfer_start = Instant::now();
    transfer::send_file(
        wormhole,
        relay_hints,
        &mut compat_file,
        file_name.clone(),
        file_size,
        abilities,
        |_info| {},
        move |sent, total| {
            let percentage = if total > 0 { (sent as f64 / total as f64 * 100.0) as u64 } else { 0 };
            progress_state.emit("send-progress", serde_json::json!({
                "id": progress_id,
                "file_name": progress_file_name,
                "sent": sent,
                "total": total,
                "percentage": percentage,
                "code": progress_code,
                "status": "sending"
            }));
        },
        cancel_call,
    )
    .await
    .map_err(|e| {
        let msg = format!("Failed to send file: {}", e);
        error_state.emit("send-error", serde_json::json!({ "id": error_id, "file_name": error_file_name, "error": msg.clone() }));
        msg
    })?;

    let elapsed = transfer_start.elapsed();
    if elapsed.as_secs_f64() > 0.0 {
        let mb = file_size as f64 / (1024.0 * 1024.0);
        eprintln!("[wyrmhole-server] File transfer complete: {:.2} MiB in {:?}", mb, elapsed);
    }

    let connection_code = {
        let active_sends = ACTIVE_SENDS.lock().await;
        active_sends.get(&send_id).map(|s| s.code.clone()).unwrap_or_default()
    };
    ACTIVE_SENDS.lock().await.remove(&send_id);

    let file_extension = Path::new(&file_name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_string();

    let file_name_without_ext = if !file_extension.is_empty() && file_name.ends_with(&format!(".{}", file_extension)) {
        file_name.strip_suffix(&format!(".{}", file_extension)).unwrap_or(&file_name).to_string()
    } else {
        file_name.clone()
    };

    let _ = files_json::add_sent_file(
        &state,
        files_json::SentFile {
            file_name: file_name_without_ext,
            file_size,
            file_extension,
            file_paths: vec![file_name.clone()],
            send_time: Local::now(),
            connection_code,
        },
    );

    eprintln!("[wyrmhole-server] send_file_call finished for '{}' in {:?}", file_name, overall_start.elapsed());
    Ok(format!("Successfully sent file '{}' ({} bytes)", file_name, file_size))
}

pub async fn send_multiple_files_call(
    state: Arc<AppState>,
    file_paths: Vec<String>,
    file_names: Vec<String>,
    send_id: String,
    folder_name: Option<String>,
) -> Result<String, String> {
    if file_paths.is_empty() {
        return Err("No files provided".to_string());
    }

    let overall_start = Instant::now();

    let display_name = if let Some(custom_name) = folder_name {
        custom_name
    } else if file_names.len() == 1 {
        file_names[0].clone()
    } else {
        let settings = state.settings.lock().await;
        let format_template = settings.default_folder_name_format.clone();
        drop(settings);
        let format_template = if format_template.trim().is_empty() {
            "#-files-via-wyrmhole".to_string()
        } else {
            format_template
        };
        format_template.replace("#", &file_paths.len().to_string())
    };

    let tarball_name = format!("{}.gz", display_name);

    state.emit("send-progress", serde_json::json!({
        "id": send_id,
        "file_name": tarball_name,
        "sent": 0,
        "total": 0,
        "percentage": 0,
        "code": "",
        "status": "preparing"
    }));

    let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
    let config = transfer::APP_CONFIG.clone();

    let mailbox_connection = match MailboxConnection::create(config, 2).await {
        Ok(conn) => {
            let code_string = conn.code().to_string();
            ACTIVE_SENDS.lock().await.insert(
                send_id.clone(),
                ActiveSend {
                    code: code_string.clone(),
                    cancel_tx: Some(cancel_tx),
                },
            );
            state.emit("connection-code", serde_json::json!({ "status": "success", "code": code_string.clone(), "send_id": send_id }));
            state.emit("send-progress", serde_json::json!({
                "id": send_id,
                "file_name": tarball_name,
                "sent": 0,
                "total": 0,
                "percentage": 0,
                "code": code_string,
                "status": "waiting"
            }));
            conn
        }
        Err(e) => {
            let msg = format!("Failed to connect: {}", e);
            state.emit("connection-code", serde_json::json!({ "status": "error", "message": msg.clone() }));
            state.emit("send-error", serde_json::json!({ "id": send_id, "file_name": display_name, "error": msg.clone() }));
            return Err(msg);
        }
    };

    let relay_hints = build_relay_hints(&state).await;
    let abilities = transit::Abilities::ALL;
    let cancel_call = cancel_rx.map(|_| ());

    let wormhole = Wormhole::connect(mailbox_connection).await.map_err(|e| {
        let msg = format!("Failed to connect to Wormhole: {}", e);
        state.emit("send-error", serde_json::json!({ "id": send_id.clone(), "file_name": display_name.clone(), "error": msg.clone() }));
        msg
    })?;

    let send_code = {
        let active_sends = ACTIVE_SENDS.lock().await;
        active_sends.get(&send_id).map(|s| s.code.clone()).unwrap_or_default()
    };

    state.emit("send-progress", serde_json::json!({
        "id": send_id,
        "file_name": tarball_name,
        "sent": 0,
        "total": 0,
        "percentage": 0,
        "code": send_code,
        "status": "packaging"
    }));

    let temp_dir = std::env::temp_dir();
    let tarball_path = temp_dir.join(format!("wyrmhole_send_{}_{}", Uuid::new_v4(), &tarball_name));

    let _tarball_size = tokio::task::spawn_blocking({
        let tarball_path = tarball_path.clone();
        let tarball_folder_name = display_name.clone();
        let file_paths = file_paths.clone();
        move || create_tarball_from_paths(&file_paths, &tarball_path, &tarball_folder_name)
    })
    .await
    .map_err(|e| format!("Failed to create tarball: {}", e))??;

    let file = File::open(&tarball_path).await.map_err(|e| {
        let msg = format!("Failed to open tarball: {}", e);
        state.emit("send-error", serde_json::json!({ "id": send_id.clone(), "file_name": display_name.clone(), "error": msg.clone() }));
        let tp = tarball_path.clone();
        tokio::spawn(async move { let _ = tokio::fs::remove_file(&tp).await; });
        msg
    })?;

    let actual_tarball_size = file.metadata().await
        .map_err(|e| format!("Failed to get tarball metadata: {}", e))?.len();

    let mut compat_file = file.compat();

    let progress_id = send_id.clone();
    let progress_file_name = tarball_name.clone();
    let progress_state = state.clone();
    let progress_code = {
        let active_sends = ACTIVE_SENDS.lock().await;
        active_sends.get(&send_id).map(|s| s.code.clone()).unwrap_or_default()
    };

    let error_state = state.clone();
    let error_id = send_id.clone();
    let error_file_name = display_name.clone();

    let transfer_start = Instant::now();
    transfer::send_file(
        wormhole,
        relay_hints,
        &mut compat_file,
        tarball_name.clone(),
        actual_tarball_size,
        abilities,
        |_info| {},
        move |sent, total| {
            let percentage = if total > 0 { (sent as f64 / total as f64 * 100.0) as u64 } else { 0 };
            progress_state.emit("send-progress", serde_json::json!({
                "id": progress_id,
                "file_name": progress_file_name,
                "sent": sent,
                "total": total,
                "percentage": percentage,
                "code": progress_code,
                "status": "sending"
            }));
        },
        cancel_call,
    )
    .await
    .map_err(|e| {
        let msg = format!("Failed to send files: {}", e);
        error_state.emit("send-error", serde_json::json!({ "id": error_id, "file_name": error_file_name, "error": msg.clone() }));
        let tp = tarball_path.clone();
        tokio::spawn(async move { let _ = tokio::fs::remove_file(&tp).await; });
        msg
    })?;

    let elapsed = transfer_start.elapsed();
    if elapsed.as_secs_f64() > 0.0 {
        let mb = actual_tarball_size as f64 / (1024.0 * 1024.0);
        eprintln!("[wyrmhole-server] Multi-file transfer complete: {:.2} MiB in {:?}", mb, elapsed);
    }

    let _ = tokio::fs::remove_file(&tarball_path).await;

    let connection_code = {
        let active_sends = ACTIVE_SENDS.lock().await;
        active_sends.get(&send_id).map(|s| s.code.clone()).unwrap_or_default()
    };
    ACTIVE_SENDS.lock().await.remove(&send_id);

    let tarball_name_without_ext = tarball_name.strip_suffix(".gz").unwrap_or(&tarball_name).to_string();

    let _ = files_json::add_sent_file(
        &state,
        files_json::SentFile {
            file_name: tarball_name_without_ext,
            file_size: actual_tarball_size,
            file_extension: "gz".to_string(),
            file_paths: file_names,
            send_time: Local::now(),
            connection_code,
        },
    );

    eprintln!("[wyrmhole-server] send_multiple_files_call finished for {} file(s) in {:?}", file_paths.len(), overall_start.elapsed());
    Ok(format!("Successfully sent {} file(s)", file_paths.len()))
}

pub async fn cancel_send(send_id: String, state: Arc<AppState>) -> Result<String, String> {
    let cancel_tx = {
        let mut active_sends = ACTIVE_SENDS.lock().await;
        if let Some(active_send) = active_sends.remove(&send_id) {
            active_send.cancel_tx
        } else {
            return Err("No active send found for this ID".to_string());
        }
    };
    if let Some(tx) = cancel_tx {
        let _ = tx.send(());
        state.emit("send-error", serde_json::json!({
            "id": send_id,
            "file_name": "Transfer cancelled",
            "error": "Transfer cancelled by user"
        }));
        Ok("Send cancelled".to_string())
    } else {
        Err("No cancel channel found for this send".to_string())
    }
}

pub async fn request_file_call(receive_code: &str, connection_id: String) -> Result<String, String> {
    let mut code_string = receive_code.trim();
    let prefix = "wormhole receive ";
    if code_string.starts_with(prefix) {
        code_string = &code_string[prefix.len()..];
        code_string = code_string.trim_start();
    }
    if code_string.is_empty() {
        return Err("No code provided for receiving file.".to_string());
    }
    let code = code_string.parse::<Code>().map_err(|e| format!("Error parsing code: {}", e))?;

    let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
    {
        let mut active_connections = ACTIVE_CONNECTIONS.lock().await;
        active_connections.insert(connection_id.clone(), ActiveConnection { cancel_tx });
    }

    let config = transfer::APP_CONFIG.clone();
    let mailbox_connection = match MailboxConnection::connect(config, code, false).await {
        Ok(conn) => conn,
        Err(e) => {
            ACTIVE_CONNECTIONS.lock().await.remove(&connection_id);
            return Err(format!("Failed to create mailbox: {}", e));
        }
    };

    let connection_id_clone = connection_id.clone();
    let wormhole = Wormhole::connect(mailbox_connection).await.map_err(|e: WormholeError| {
        let cid = connection_id_clone.clone();
        tokio::spawn(async move { ACTIVE_CONNECTIONS.lock().await.remove(&cid); });
        format!("Failed to connect to Wormhole: {}", e)
    })?;

    let relay_hint = transit::RelayHint::from_urls(
        None,
        [transit::DEFAULT_RELAY_SERVER.parse().unwrap()],
    ).unwrap();
    let relay_hints = vec![relay_hint];
    let abilities = transit::Abilities::ALL;
    let cancel_call = cancel_rx.map(|_| ());

    let connection_id_clone2 = connection_id.clone();
    let maybe_request = transfer::request_file(wormhole, relay_hints, abilities, cancel_call)
        .await
        .map_err(|e| {
            let cid = connection_id_clone2.clone();
            tokio::spawn(async move { ACTIVE_CONNECTIONS.lock().await.remove(&cid); });
            format!("Failed to request file: {}", e)
        })?;

    ACTIVE_CONNECTIONS.lock().await.remove(&connection_id);

    if let Some(receive_request) = maybe_request {
        let file_name = receive_request.file_name().to_string();
        let file_size = receive_request.file_size();
        let id = Uuid::new_v4().to_string();
        REQUESTS_HASHMAP.lock().await.insert(id.clone(), OpenRequests { request: receive_request });
        let response = serde_json::json!({ "id": id, "file_name": file_name, "file_size": file_size });
        Ok(response.to_string())
    } else {
        Err("No file was offered by the sender (canceled or empty).".to_string())
    }
}

pub async fn cancel_connection(connection_id: String) -> Result<String, String> {
    let cancel_tx = {
        let mut active_connections = ACTIVE_CONNECTIONS.lock().await;
        if let Some(active_connection) = active_connections.remove(&connection_id) {
            active_connection.cancel_tx
        } else {
            return Err("No active connection found for this ID".to_string());
        }
    };
    let _ = cancel_tx.send(());
    Ok("Connection cancelled".to_string())
}

pub async fn receiving_file_deny(id: String) -> Result<String, String> {
    let mut requests = REQUESTS_HASHMAP.lock().await;
    if let Some(entry) = requests.remove(&id) {
        if let Err(e) = entry.request.reject().await {
            return Err(format!("Failed to close request: {}", e));
        }
        Ok("File offer denied".to_string())
    } else {
        Err("No request found for this ID".to_string())
    }
}

pub async fn receiving_file_accept(id: String, state: Arc<AppState>) -> Result<String, String> {
    let mut requests = REQUESTS_HASHMAP.lock().await;
    if let Some(entry) = requests.remove(&id) {
        let mut connection_type = String::new();
        let mut peer_address: SocketAddr = "0.0.0.0:0".parse().unwrap();

        let transit_handler = |info: transit::TransitInfo| {
            connection_type = match info.conn_type {
                transit::ConnectionType::Direct => "direct".to_string(),
                transit::ConnectionType::Relay { ref name } => {
                    if let Some(n) = name { format!("relay ({})", n) } else { "relay".to_string() }
                }
                _ => "unknown".to_string(),
            };
            peer_address = info.peer_addr.to_owned();
        };

        let download_dir = {
            let settings = state.settings.lock().await;
            settings.download_directory.clone()
        };

        let file_name_with_extension = entry.request.file_name().to_string();
        let file_size = entry.request.file_size();

        let progress_id = id.clone();
        let progress_file_name = file_name_with_extension.clone();
        let progress_state = state.clone();

        let progress_handler = move |transferred: u64, total: u64| {
            let percentage = if total > 0 { (transferred as f64 / total as f64 * 100.0) as u64 } else { 0 };
            progress_state.emit("download-progress", serde_json::json!({
                "id": progress_id,
                "file_name": progress_file_name,
                "transferred": transferred,
                "total": total,
                "percentage": percentage
            }));
        };

        if let Err(e) = tokio::fs::create_dir_all(&download_dir).await {
            let msg = format!("Failed to create download directory: {}", e);
            state.emit("download-error", serde_json::json!({ "id": id, "file_name": file_name_with_extension, "error": msg.clone() }));
            return Err(msg);
        }

        let file_path = find_unique_file_path(&download_dir, &file_name_with_extension);
        let final_file_name = file_path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&file_name_with_extension)
            .to_string();

        let file_name = final_file_name.rsplit_once('.').map(|(b, _)| b.to_string()).unwrap_or_else(|| final_file_name.clone());
        let file_extension = final_file_name.rsplit_once('.').map(|(_, a)| a.to_string()).unwrap_or_default();

        let file = tokio::fs::File::create(&file_path).await.map_err(|e| {
            let msg = format!("Failed to create file: {}: {}", file_path.display(), e);
            state.emit("download-error", serde_json::json!({ "id": id.clone(), "file_name": file_name_with_extension.clone(), "error": msg.clone() }));
            msg
        })?;

        let mut compat_file = file.compat_write();
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        ACTIVE_DOWNLOADS.lock().await.insert(
            id.clone(),
            ActiveDownload { cancel_tx, file_name: file_name_with_extension.clone() },
        );
        let cancel = cancel_rx.map(|_| ());

        let error_state = state.clone();
        let error_id = id.clone();
        let error_file_name = file_name_with_extension.clone();

        entry.request
            .accept(transit_handler, progress_handler, &mut compat_file, cancel)
            .await
            .map_err(|e| {
                let msg = format!("Error accepting file: {}", e);
                let id_clone = id.clone();
                tokio::spawn(async move { ACTIVE_DOWNLOADS.lock().await.remove(&id_clone); });
                error_state.emit("download-error", serde_json::json!({ "id": error_id, "file_name": error_file_name, "error": msg.clone() }));
                msg
            })?;

        ACTIVE_DOWNLOADS.lock().await.remove(&id);

        let is_tarball = final_file_name.ends_with(".tar.gz")
            || final_file_name.ends_with(".tgz")
            || final_file_name.ends_with(".gz");

        if is_tarball {
            let auto_extract = {
                let settings = state.settings.lock().await;
                settings.auto_extract_tarballs
            };

            if auto_extract {
                let extracted_files = tokio::task::spawn_blocking({
                    let file_path = file_path.clone();
                    let download_dir = download_dir.clone();
                    move || extract_tarball(&file_path, &download_dir)
                })
                .await
                .map_err(|e| format!("Failed to extract tarball: {}", e))??;

                for (extracted_file_name, extracted_file_size) in &extracted_files {
                    let (name, ext) = extracted_file_name.rsplit_once('.')
                        .map(|(n, e)| (n.to_string(), e.to_string()))
                        .unwrap_or_else(|| (extracted_file_name.clone(), String::new()));

                    let _ = files_json::add_received_file(&state, files_json::ReceivedFile {
                        file_name: name,
                        file_size: *extracted_file_size,
                        file_extension: ext,
                        download_url: extracted_file_name.clone(),
                        download_time: Local::now(),
                        connection_type: connection_type.clone(),
                        peer_address,
                    });
                }

                let fp = file_path.clone();
                tokio::spawn(async move { let _ = tokio::fs::remove_file(&fp).await; });

                Ok(format!("Tarball extracted! {} file(s) saved", extracted_files.len()))
            } else {
                let _ = files_json::add_received_file(&state, files_json::ReceivedFile {
                    file_name,
                    file_size,
                    file_extension,
                    download_url: final_file_name,
                    download_time: Local::now(),
                    connection_type,
                    peer_address,
                });
                Ok(format!("File saved: {}", file_path.display()))
            }
        } else {
            let _ = files_json::add_received_file(&state, files_json::ReceivedFile {
                file_name,
                file_size,
                file_extension,
                download_url: final_file_name,
                download_time: Local::now(),
                connection_type,
                peer_address,
            });
            Ok(format!("File saved: {}", file_path.display()))
        }
    } else {
        Err("No request found for this id".to_string())
    }
}

pub async fn cancel_download(download_id: String) -> Result<String, String> {
    let cancel_tx = {
        let mut active_downloads = ACTIVE_DOWNLOADS.lock().await;
        if let Some(d) = active_downloads.remove(&download_id) {
            d.cancel_tx
        } else {
            return Err("No active download found for this ID".to_string());
        }
    };
    let _ = cancel_tx.send(());
    Ok("Download cancelled".to_string())
}

pub async fn cancel_all_transfers(state: Arc<AppState>) -> Result<String, String> {
    {
        let mut active_sends = ACTIVE_SENDS.lock().await;
        for (send_id, active_send) in active_sends.drain() {
            if let Some(tx) = active_send.cancel_tx {
                let _ = tx.send(());
                state.emit("send-error", serde_json::json!({
                    "id": send_id,
                    "file_name": "Transfer cancelled",
                    "error": "Transfer cancelled by user"
                }));
            }
        }
    }
    {
        let mut active_downloads = ACTIVE_DOWNLOADS.lock().await;
        for (_, d) in active_downloads.drain() {
            let _ = d.cancel_tx.send(());
        }
    }
    {
        let mut active_connections = ACTIVE_CONNECTIONS.lock().await;
        for (_, c) in active_connections.drain() {
            let _ = c.cancel_tx.send(());
        }
    }
    Ok("All active transfers cancelled".to_string())
}

pub async fn test_relay_server(state: Arc<AppState>) -> Result<String, String> {
    let settings = state.settings.lock().await;
    let user_relay = settings.relay_server_url.as_deref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    drop(settings);

    if let Some(custom) = user_relay {
        let url = custom.parse().map_err(|e| format!("Invalid relay URL: {}", e))?;
        transit::RelayHint::from_urls(None, [url])
            .map_err(|e| format!("Invalid relay configuration: {}", e))?;
        Ok(format!("Custom relay URL looks valid: {}", custom))
    } else {
        let default_url = transit::DEFAULT_RELAY_SERVER.parse()
            .map_err(|e| format!("Internal error: {}", e))?;
        transit::RelayHint::from_urls(None, [default_url])
            .map_err(|e| format!("Default relay invalid: {}", e))?;
        Ok(format!("Using default relay: {}", transit::DEFAULT_RELAY_SERVER))
    }
}

async fn build_relay_hints(state: &Arc<AppState>) -> Vec<transit::RelayHint> {
    let settings = state.settings.lock().await;
    let user_relay = settings.relay_server_url.as_deref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    drop(settings);

    let mut urls = Vec::new();
    if let Some(custom) = user_relay {
        if let Ok(url) = custom.parse() {
            urls.push(url);
        }
    }
    if urls.is_empty() {
        urls.push(transit::DEFAULT_RELAY_SERVER.parse().unwrap());
    }
    vec![transit::RelayHint::from_urls(None, urls).unwrap()]
}

fn create_tarball_from_paths(paths: &[String], output_path: &Path, folder_name: &str) -> Result<u64, String> {
    let tar_gz = std::fs::File::create(output_path).map_err(|e| format!("Failed to create tarball: {}", e))?;
    let enc = GzEncoder::new(tar_gz, Compression::fast());
    let mut tar = Builder::new(enc);

    for file_path in paths {
        let src_path = Path::new(file_path);
        if !src_path.exists() {
            return Err(format!("File does not exist: {}", file_path));
        }
        if src_path.is_dir() {
            let name = src_path.file_name().and_then(|n| n.to_str()).unwrap_or("folder");
            let dest_prefix = Path::new(folder_name).join(name);
            tar.append_dir_all(&dest_prefix, src_path)
                .map_err(|e| format!("Failed to add directory: {}", e))?;
        } else {
            let name = src_path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
            let dest = Path::new(folder_name).join(name);
            tar.append_path_with_name(src_path, &dest)
                .map_err(|e| format!("Failed to add file: {}", e))?;
        }
    }

    tar.finish().map_err(|e| format!("Failed to finish tarball: {}", e))?;
    let size = std::fs::metadata(output_path).map_err(|e| format!("Failed to get tarball metadata: {}", e))?.len();
    Ok(size)
}

fn extract_tarball(tarball_path: &Path, output_dir: &Path) -> Result<Vec<(String, u64)>, String> {
    let tar_gz = std::fs::File::open(tarball_path).map_err(|e| format!("Failed to open tarball: {}", e))?;
    let dec = GzDecoder::new(tar_gz);
    let mut archive = Archive::new(dec);
    let mut extracted_files = Vec::new();

    for entry_result in archive.entries().map_err(|e| format!("Failed to read tarball: {}", e))? {
        let mut entry = entry_result.map_err(|e| format!("Failed to read entry: {}", e))?;
        let path = entry.path().map_err(|e| format!("Failed to get entry path: {}", e))?;
        if entry.header().entry_type().is_dir() {
            continue;
        }
        let path_str = path.to_string_lossy().to_string();
        let display_name = path.file_name().and_then(|n| n.to_str()).map(|s| s.to_string()).unwrap_or_else(|| path_str.clone());
        let output_path = output_dir.join(&path_str);
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create directory: {}", e))?;
        }
        let mut outfile = std::fs::File::create(&output_path).map_err(|e| format!("Failed to create file: {}", e))?;
        std::io::copy(&mut entry, &mut outfile).map_err(|e| format!("Failed to extract file: {}", e))?;
        let metadata = std::fs::metadata(&output_path).map_err(|e| format!("Failed to get file metadata: {}", e))?;
        extracted_files.push((display_name, metadata.len()));
    }

    Ok(extracted_files)
}

fn find_unique_file_path(download_dir: &Path, file_name_with_extension: &str) -> PathBuf {
    let base_path = download_dir.join(file_name_with_extension);
    if !base_path.exists() {
        return base_path;
    }
    let (file_name, extension) = file_name_with_extension.rsplit_once('.')
        .map(|(n, e)| (n.to_string(), format!(".{}", e)))
        .unwrap_or_else(|| (file_name_with_extension.to_string(), String::new()));

    let mut counter = 1;
    loop {
        let new_name = format!("{}({}){}", file_name, counter, extension);
        let new_path = download_dir.join(&new_name);
        if !new_path.exists() {
            return new_path;
        }
        counter += 1;
        if counter > 10000 {
            let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S");
            return download_dir.join(format!("{}_{}{}", file_name, ts, extension));
        }
    }
}
