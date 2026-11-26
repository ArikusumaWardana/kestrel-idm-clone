use axum::{
    extract::{
        ws::{Message, WebSocket},
        Query, State, WebSocketUpgrade,
    },
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::broadcast;

use crate::contracts::{CreateDownloadRequest, WsMessage};
use crate::error::{AppError, AppResult};
use crate::manager::DownloadManager;

#[derive(Deserialize)]
struct WsQuery {
    token: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthResponse {
    status: &'static str,
    active_downloads: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadCreatedResponse {
    download_id: uuid::Uuid,
    filename: String,
}

pub fn build_router(manager: Arc<DownloadManager>) -> Router {
    Router::new()
        .route("/api/health", get(health_handler))
        .route("/api/downloads", post(create_download_handler))
        .route("/ws", get(ws_handler))
        .with_state(manager)
}

pub async fn spawn_bridge_server(manager: Arc<DownloadManager>) -> AppResult<SocketAddr> {
    let port = manager.settings.read().await.bridge_port;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = TcpListener::bind(addr).await?;
    let bound_addr = listener.local_addr()?;
    let router = build_router(manager);

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, router).await {
            eprintln!("Local bridge server stopped: {e}");
        }
    });

    Ok(bound_addr)
}

fn validate_origin(headers: &HeaderMap) -> bool {
    let Some(origin_val) = headers.get("origin") else {
        return true;
    };
    let Ok(origin) = origin_val.to_str() else {
        return false;
    };
    origin.starts_with("chrome-extension://")
        || origin.starts_with("moz-extension://")
        || origin.starts_with("http://127.0.0.1")
        || origin.starts_with("http://localhost")
}

fn validate_token(expected: &str, supplied: Option<&str>) -> bool {
    let Some(candidate) = supplied else {
        return false;
    };
    if expected.is_empty() || candidate.len() != expected.len() {
        return false;
    }
    let diff = expected
        .as_bytes()
        .iter()
        .zip(candidate.as_bytes().iter())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b));
    diff == 0
}

async fn authorize_http(headers: &HeaderMap, manager: &DownloadManager) -> Result<(), StatusCode> {
    if !validate_origin(headers) {
        return Err(StatusCode::FORBIDDEN);
    }
    let expected = manager.settings.read().await.local_auth_token.clone();
    let supplied = headers.get("x-idm-token").and_then(|v| v.to_str().ok());
    if !validate_token(&expected, supplied) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}

async fn health_handler(
    State(manager): State<Arc<DownloadManager>>,
    headers: HeaderMap,
) -> Result<Json<HealthResponse>, StatusCode> {
    authorize_http(&headers, &manager).await?;
    let active_downloads = manager
        .list_active_downloads()
        .await
        .map(|l| l.len())
        .unwrap_or(0);
    Ok(Json(HealthResponse {
        status: "ok",
        active_downloads,
    }))
}

async fn create_download_handler(
    State(manager): State<Arc<DownloadManager>>,
    headers: HeaderMap,
    Json(req): Json<CreateDownloadRequest>,
) -> Result<(StatusCode, Json<DownloadCreatedResponse>), (StatusCode, Json<crate::error::ErrorPayload>)>
{
    if let Err(code) = authorize_http(&headers, &manager).await {
        return Err((code, Json(crate::error::ErrorPayload::from(&AppError::Unauthorized))));
    }

    let prepared = manager.queue_browser_prompt(req).await;
    let prompt_id = uuid::Uuid::new_v4();
    let filename = prepared
        .filename
        .unwrap_or_else(|| "browser-download".to_string());
    Ok((
        StatusCode::ACCEPTED,
        Json(DownloadCreatedResponse {
            download_id: prompt_id,
            filename,
        }),
    ))
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(manager): State<Arc<DownloadManager>>,
    headers: HeaderMap,
    Query(query): Query<WsQuery>,
) -> impl IntoResponse {
    if !validate_origin(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let expected = manager.settings.read().await.local_auth_token.clone();
    if !validate_token(&expected, query.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    ws.on_upgrade(move |socket| handle_ws_socket(socket, manager))
}

async fn handle_ws_socket(mut socket: WebSocket, manager: Arc<DownloadManager>) {
    manager.extension_clients.fetch_add(1, Ordering::Relaxed);
    let _ = manager.state_changed_tx.send(());

    let mut out_rx = manager.ws_out_tx.subscribe();

    loop {
        tokio::select! {
            inbound = socket.recv() => {
                match inbound {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(msg) = serde_json::from_str::<WsMessage>(&text) {
                            match msg {
                                WsMessage::Ping => {
                                    let active_downloads = manager
                                        .list_active_downloads()
                                        .await
                                        .map(|l| l.len())
                                        .unwrap_or(0);
                                    if let Ok(json) = serde_json::to_string(&WsMessage::Pong { active_downloads }) {
                                        if socket.send(Message::Text(json.into())).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                                WsMessage::RefreshLinkResponse {
                                    download_id,
                                    new_url,
                                    cookies,
                                    user_agent,
                                    referer,
                                } => {
                                    let _ = manager
                                        .apply_refreshed_link(download_id, new_url, cookies, user_agent, referer)
                                        .await;
                                }
                                _ => {}
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    _ => {}
                }
            }
            outbound = out_rx.recv() => {
                match outbound {
                    Ok(msg) => {
                        if let Ok(json) = serde_json::to_string(&msg) {
                            if socket.send(Message::Text(json.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }

    manager.extension_clients.fetch_sub(1, Ordering::Relaxed);
    let _ = manager.state_changed_tx.send(());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StoragePaths;
    use tempfile::tempdir;

    async fn spawn_test_server() -> (SocketAddr, String, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let paths = StoragePaths::from_base(dir.path().to_path_buf()).unwrap();
        let mgr = DownloadManager::new(paths, true).unwrap();
        let token = mgr.get_settings().await.local_auth_token;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let router = build_router(mgr);
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });

        (addr, token, dir)
    }

    #[tokio::test]
    async fn rejects_requests_without_valid_token_or_with_bad_origin() {
        let (addr, token, _dir) = spawn_test_server().await;
        let client = reqwest::Client::new();

        // Missing token -> 401
        let res_no_token = client
            .post(format!("http://{addr}/api/downloads"))
            .header("Content-Type", "application/json")
            .body(r#"{"url":"http://127.0.0.1:9/a.bin"}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(res_no_token.status(), StatusCode::UNAUTHORIZED);

        // Wrong token -> 401
        let res_bad_token = client
            .get(format!("http://{addr}/api/health"))
            .header("X-IDM-Token", "wrong-token")
            .send()
            .await
            .unwrap();
        assert_eq!(res_bad_token.status(), StatusCode::UNAUTHORIZED);

        // Disallowed external Origin -> 403
        let res_bad_origin = client
            .get(format!("http://{addr}/api/health"))
            .header("Origin", "https://evil.example")
            .header("X-IDM-Token", &token)
            .send()
            .await
            .unwrap();
        assert_eq!(res_bad_origin.status(), StatusCode::FORBIDDEN);

        // Valid token + chrome-extension Origin -> 200
        let res_ok = client
            .get(format!("http://{addr}/api/health"))
            .header("Origin", "chrome-extension://abcdefghijklmnop")
            .header("X-IDM-Token", &token)
            .send()
            .await
            .unwrap();
        assert_eq!(res_ok.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn rejects_websocket_without_valid_token() {
        let (addr, _token, _dir) = spawn_test_server().await;
        let client = reqwest::Client::new();
        let res = client
            .get(format!("http://{addr}/ws?token=invalid"))
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }
}
