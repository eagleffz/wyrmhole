mod files;
mod files_json;
mod settings;
mod state;

use axum::{
    body::Body,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Multipart, Path, State,
    },
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post},
    Json, Router,
};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use std::{path::PathBuf, sync::Arc};
use tokio::fs;
use tower_http::{cors::CorsLayer, services::ServeDir};

type AppState = Arc<state::AppState>;

struct ApiError(String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (StatusCode::INTERNAL_SERVER_ERROR, self.0).into_response()
    }
}

impl<E: std::fmt::Display> From<E> for ApiError {
    fn from(e: E) -> Self {
        ApiError(e.to_string())
    }
}

#[tokio::main]
async fn main() {
    let data_dir = std::env::var("DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/data"));

    std::fs::create_dir_all(&data_dir).expect("Failed to create data directory");

    let app_settings = settings::AppSettings::load_or_create(&data_dir);
    let state: AppState = Arc::new(state::AppState::new(app_settings, data_dir.clone()));

    let static_dir = std::env::var("STATIC_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/app/dist"));

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/api/send", post(send_file_route))
        .route("/api/send-multiple", post(send_multiple_route))
        .route("/api/send/:id", delete(cancel_send_route))
        .route("/api/receive", post(receive_route))
        .route("/api/connection/:id", delete(cancel_connection_route))
        .route("/api/accept/:id", post(accept_route))
        .route("/api/deny/:id", post(deny_route))
        .route("/api/download/:id", delete(cancel_download_route))
        .route("/api/transfers", delete(cancel_all_route))
        .route("/api/received-files", get(received_files_route))
        .route("/api/sent-files", get(sent_files_route))
        .route("/api/settings", get(get_settings_route))
        .route("/api/settings", patch(patch_settings_route))
        .route("/api/test-relay", post(test_relay_route))
        .route("/api/export/received-files", get(export_received_route))
        .route("/api/export/sent-files", get(export_sent_route))
        .route("/api/download-file/*path", get(download_file_route))
        .nest_service("/", ServeDir::new(&static_dir).fallback(ServeDir::new(&static_dir)))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let bind_addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let listener = tokio::net::TcpListener::bind(&bind_addr).await.expect("Failed to bind");
    eprintln!("[wyrmhole-server] Listening on {}", bind_addr);
    axum::serve(listener, app).await.expect("Server error");
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_ws(socket, state))
}

async fn handle_ws(socket: WebSocket, state: AppState) {
    let mut rx = state.event_tx.subscribe();
    let (mut sender, _receiver) = socket.split();
    use tokio::sync::broadcast::error::RecvError;
    loop {
        match rx.recv().await {
            Ok(msg) => {
                if sender.send(Message::Text(msg)).await.is_err() {
                    break;
                }
            }
            Err(RecvError::Closed) => break,
            Err(RecvError::Lagged(_)) => continue,
        }
    }
}

async fn send_file_route(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, ApiError> {
    let mut send_id = uuid::Uuid::new_v4().to_string();
    let mut temp_path: Option<PathBuf> = None;
    let mut file_name = String::from("unknown");

    while let Some(field) = multipart.next_field().await? {
        match field.name() {
            Some("send_id") => {
                let val = field.text().await?;
                if !val.is_empty() {
                    send_id = val;
                }
            }
            Some("file") => {
                let fname = field.file_name().unwrap_or("unknown").to_string();
                file_name = fname.clone();
                let bytes = field.bytes().await?;
                let p = std::env::temp_dir().join(format!("wyrmhole_upload_{}", uuid::Uuid::new_v4()));
                fs::write(&p, &bytes).await?;
                temp_path = Some(p);
            }
            _ => {}
        }
    }

    let p = temp_path.ok_or_else(|| ApiError("No file uploaded".to_string()))?;
    let path_str = p.to_str().unwrap_or("").to_string();

    tokio::spawn(async move {
        let _ = files::send_file_call(state, &path_str, file_name, send_id).await;
        let _ = fs::remove_file(&path_str).await;
    });

    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn send_multiple_route(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, ApiError> {
    let mut send_id = uuid::Uuid::new_v4().to_string();
    let mut folder_name: Option<String> = None;
    let mut temp_paths: Vec<PathBuf> = vec![];
    let mut file_names: Vec<String> = vec![];

    while let Some(field) = multipart.next_field().await? {
        match field.name() {
            Some("send_id") => {
                let val = field.text().await?;
                if !val.is_empty() {
                    send_id = val;
                }
            }
            Some("folder_name") => {
                let val = field.text().await?;
                if !val.is_empty() {
                    folder_name = Some(val);
                }
            }
            Some("files") => {
                let fname = field.file_name().unwrap_or("unknown").to_string();
                let bytes = field.bytes().await?;
                let p = std::env::temp_dir().join(format!("wyrmhole_upload_{}", uuid::Uuid::new_v4()));
                fs::write(&p, &bytes).await?;
                file_names.push(fname);
                temp_paths.push(p);
            }
            _ => {}
        }
    }

    if temp_paths.is_empty() {
        return Err(ApiError("No files uploaded".to_string()));
    }

    let path_strs: Vec<String> = temp_paths.iter().map(|p| p.to_str().unwrap_or("").to_string()).collect();

    tokio::spawn(async move {
        let _ = files::send_multiple_files_call(state, path_strs.clone(), file_names, send_id, folder_name).await;
        for p in &path_strs {
            let _ = fs::remove_file(p).await;
        }
    });

    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn cancel_send_route(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    files::cancel_send(id, state).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Deserialize)]
struct ReceiveBody {
    receive_code: String,
    connection_id: String,
}

async fn receive_route(
    Json(body): Json<ReceiveBody>,
) -> Result<impl IntoResponse, ApiError> {
    let result = files::request_file_call(&body.receive_code, body.connection_id).await?;
    let value: serde_json::Value = serde_json::from_str(&result)?;
    Ok(Json(value))
}

async fn cancel_connection_route(
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    files::cancel_connection(id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn accept_route(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    tokio::spawn(async move {
        let _ = files::receiving_file_accept(id, state).await;
    });
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn deny_route(
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    files::receiving_file_deny(id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn cancel_download_route(
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    files::cancel_download(id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn cancel_all_route(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    files::cancel_all_transfers(state).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn received_files_route(State(state): State<AppState>) -> impl IntoResponse {
    Json(files_json::get_received_files_json(&state.data_dir))
}

async fn sent_files_route(State(state): State<AppState>) -> impl IntoResponse {
    Json(files_json::get_sent_files_json(&state.data_dir))
}

async fn get_settings_route(State(state): State<AppState>) -> impl IntoResponse {
    let settings = state.settings.lock().await;
    Json(serde_json::json!({
        "download_directory": settings.download_directory.to_string_lossy(),
        "auto_extract_tarballs": settings.auto_extract_tarballs,
        "default_folder_name_format": settings.default_folder_name_format,
        "relay_server_url": settings.relay_server_url,
    }))
}

#[derive(Deserialize)]
struct PatchSettings {
    auto_extract_tarballs: Option<bool>,
    default_folder_name_format: Option<String>,
    relay_server_url: Option<Option<String>>,
}

async fn patch_settings_route(
    State(state): State<AppState>,
    Json(body): Json<PatchSettings>,
) -> Result<impl IntoResponse, ApiError> {
    let mut settings = state.settings.lock().await;
    if let Some(v) = body.auto_extract_tarballs {
        settings.auto_extract_tarballs = v;
    }
    if let Some(v) = body.default_folder_name_format {
        let clone = v.clone();
        settings.default_folder_name_format = v;
        drop(settings);
        state.emit("default-folder-name-format-updated", serde_json::json!({ "value": clone }));
        let settings = state.settings.lock().await;
        settings.save(&state.data_dir);
        return Ok(Json(serde_json::json!({ "ok": true })));
    }
    if let Some(v) = body.relay_server_url {
        settings.relay_server_url = v;
    }
    settings.save(&state.data_dir);
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn test_relay_route(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    let msg = files::test_relay_server(state).await?;
    Ok(Json(serde_json::json!({ "message": msg })))
}

async fn export_received_route(State(state): State<AppState>) -> impl IntoResponse {
    let path = state.data_dir.join("received_files.json");
    let content = std::fs::read_to_string(&path).unwrap_or_else(|_| "[]".to_string());
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CONTENT_DISPOSITION, "attachment; filename=\"received_files.json\"")
        .body(Body::from(content))
        .unwrap()
}

async fn export_sent_route(State(state): State<AppState>) -> impl IntoResponse {
    let path = state.data_dir.join("sent_files.json");
    let content = std::fs::read_to_string(&path).unwrap_or_else(|_| "[]".to_string());
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CONTENT_DISPOSITION, "attachment; filename=\"sent_files.json\"")
        .body(Body::from(content))
        .unwrap()
}

async fn download_file_route(
    State(state): State<AppState>,
    Path(file_path): Path<String>,
) -> impl IntoResponse {
    let settings = state.settings.lock().await;
    let download_dir = settings.download_directory.clone();
    drop(settings);

    let target = download_dir.join(&file_path);

    let content = match fs::read(&target).await {
        Ok(c) => c,
        Err(_) => return (StatusCode::NOT_FOUND, "File not found").into_response(),
    };

    let filename = target.file_name().and_then(|n| n.to_str()).unwrap_or("file").to_string();
    let mime = mime_guess(&filename);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", filename))
        .body(Body::from(content))
        .unwrap()
}

fn mime_guess(filename: &str) -> &'static str {
    match filename.rsplit_once('.').map(|(_, e)| e).unwrap_or("") {
        "gz" | "tgz" => "application/gzip",
        "tar" => "application/x-tar",
        "zip" => "application/zip",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "txt" | "md" => "text/plain",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}
