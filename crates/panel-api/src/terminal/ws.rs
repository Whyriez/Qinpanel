//! WebSocket live streaming handler for Web Terminal PTY sessions.
//!
//! Complies with PRD Section 5.4 and Task P6-T2:
//! - Binary and text bidirectional frame streaming
//! - CSWSH origin header verification
//! - Session cookie and token authentication
//! - Bounded ring-buffer scrollback delivery upon initial connect or reconnect
//! - Window resize command forwarding
//! - Periodic heartbeat ping/pong keep-alive
//! - Backpressure control via bounded async channels

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::{
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, info, warn};

use crate::auth::AuthenticatedUser;
use crate::logs::validate_ws_origin;
use crate::routes::AppState;
use crate::terminal::{
    ActiveTerminalSession, TerminalBroadcastEvent, WsClientMessage, WsServerMessage,
};

const HEARTBEAT_INTERVAL_SECS: u64 = 20;
const WS_OUTBOUND_QUEUE_CAPACITY: usize = 128;

/// WebSocket terminal connection handler: `GET /ws/terminal/{id}`
#[allow(clippy::cognitive_complexity)]
pub async fn terminal_ws_handler(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    auth_user: AuthenticatedUser,
    axum::Extension(context): axum::Extension<crate::public_url::PublicRequest>,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    // 1. Validate Origin header to eliminate Cross-Site WebSocket Hijacking (CSWSH)
    if !validate_ws_origin(&headers, context.origin.secure) {
        warn!(
            "Terminal WebSocket rejected due to invalid Origin header for session {}",
            session_id
        );
        return Err((StatusCode::FORBIDDEN, "Invalid Origin"));
    }

    // 2. Validate session existence
    let session = state
        .terminal_hub
        .get_session(&session_id)
        .await
        .ok_or((StatusCode::NOT_FOUND, "Terminal session not found"))?;

    // 3. Validate tenant authorization
    if !auth_user.is_admin() && session.user_id != auth_user.id {
        warn!(
            "Unauthorized terminal access attempt by user {} (id {}) on session {} owned by user {}",
            auth_user.username, auth_user.id, session_id, session.user_id
        );
        return Err((StatusCode::FORBIDDEN, "Access denied to terminal session"));
    }

    // Populate client IP if not yet registered
    let client_ip = crate::audit::extract_client_ip(&headers);
    if let Some(ip) = client_ip {
        let mut cur_ip = session.client_ip.write().await;
        if cur_ip.is_none() {
            *cur_ip = Some(ip);
        }
    }

    info!(
        "Upgrading WebSocket for terminal session {} (user {})",
        session_id, auth_user.username
    );

    Ok(ws.on_upgrade(move |socket| handle_terminal_ws_stream(socket, state, session)))
}

/// Bidirectional WebSocket terminal streaming session.
#[allow(clippy::cognitive_complexity)]
async fn handle_terminal_ws_stream(
    socket: WebSocket,
    state: AppState,
    session: Arc<ActiveTerminalSession>,
) {
    let session_id = session.session_id.clone();
    info!(
        "Terminal WebSocket client connected for session {}",
        session_id
    );
    session.update_activity();

    let (mut ws_sender, mut ws_receiver) = socket.split();
    let (ws_out_tx, mut ws_out_rx) = mpsc::channel::<Message>(WS_OUTBOUND_QUEUE_CAPACITY);

    // Task 1: Dedicated sender loop writing frames to WebSocket connection with backpressure
    let sender_sid = session_id.clone();
    let ws_writer_task = tokio::spawn(async move {
        while let Some(msg) = ws_out_rx.recv().await {
            if let Err(e) = ws_sender.send(msg).await {
                debug!(
                    "WebSocket writer failed for terminal session {}: {:?}",
                    sender_sid, e
                );
                break;
            }
        }
    });

    // 1. Transmit initial connection status message
    let connected_msg = WsServerMessage::Connected {
        session_id: session.session_id.clone(),
        cols: session.cols.load(Ordering::Relaxed),
        rows: session.rows.load(Ordering::Relaxed),
        cwd: session.cwd.clone(),
        username: session.username.clone(),
    };
    if let Ok(json_str) = serde_json::to_string(&connected_msg) {
        let _ = ws_out_tx.send(Message::Text(json_str.into())).await;
    }

    // 2. Deliver scrollback history buffer for clean initial render / reconnect
    let scrollback_data = session.scrollback.read().await.to_vec();
    if !scrollback_data.is_empty() {
        debug!(
            "Delivering {} bytes of scrollback history to reconnecting client for session {}",
            scrollback_data.len(),
            session_id
        );
        let _ = ws_out_tx
            .send(Message::Binary(scrollback_data.into()))
            .await;
    }

    // Task 2: PTY output broadcast subscriber task
    let mut bcast_rx = session.subscribe();
    let pty_out_tx = ws_out_tx.clone();
    let pty_session = Arc::clone(&session);
    let pty_sid = session_id.clone();

    let pty_listener_task = tokio::spawn(async move {
        loop {
            match bcast_rx.recv().await {
                Ok(TerminalBroadcastEvent::Output(bytes)) => {
                    pty_session.update_activity();
                    // Stream raw binary frame to client
                    if pty_out_tx
                        .send(Message::Binary((*bytes).clone().into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(TerminalBroadcastEvent::Closed(exit_code)) => {
                    let closed_msg = WsServerMessage::Closed { exit_code };
                    if let Ok(json_str) = serde_json::to_string(&closed_msg) {
                        let _ = pty_out_tx.send(Message::Text(json_str.into())).await;
                    }
                    break;
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    warn!(
                        "Terminal WebSocket client lagged behind by {} frames for session {}",
                        skipped, pty_sid
                    );
                    // Catch up using current scrollback buffer
                    let catchup_data = pty_session.scrollback.read().await.to_vec();
                    if !catchup_data.is_empty() {
                        let _ = pty_out_tx.send(Message::Binary(catchup_data.into())).await;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }
    });

    // Task 3: Heartbeat ping timer
    let ping_tx = ws_out_tx.clone();
    let heartbeat_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
        // Skip immediate first tick
        interval.tick().await;

        loop {
            interval.tick().await;
            if ping_tx.send(Message::Ping(vec![].into())).await.is_err() {
                break;
            }
        }
    });

    // Task 4: Inbound client message receiver loop (runs on main connection task)
    while let Some(msg_result) = ws_receiver.next().await {
        match msg_result {
            Ok(Message::Binary(bytes)) => {
                session.update_activity();
                if let Err(e) = state
                    .terminal_hub
                    .write_input(&session_id, bytes.to_vec())
                    .await
                {
                    warn!(
                        "Failed to forward binary input to PTY session {}: {:?}",
                        session_id, e
                    );
                }
            }
            Ok(Message::Text(text)) => {
                session.update_activity();
                // Attempt to parse structured control command (resize, ping, input)
                if let Ok(client_msg) = serde_json::from_str::<WsClientMessage>(&text) {
                    match client_msg {
                        WsClientMessage::Resize { cols, rows } => {
                            let clamped_cols = cols.clamp(20, 500);
                            let clamped_rows = rows.clamp(5, 200);
                            if let Err(e) = state
                                .terminal_hub
                                .resize_session(&session_id, clamped_cols, clamped_rows)
                                .await
                            {
                                warn!("Failed to resize terminal session {}: {:?}", session_id, e);
                            }
                        }
                        WsClientMessage::Ping => {
                            let pong = WsServerMessage::Pong;
                            if let Ok(pong_str) = serde_json::to_string(&pong) {
                                let _ = ws_out_tx.send(Message::Text(pong_str.into())).await;
                            }
                        }
                        WsClientMessage::Input { data } => {
                            if let Err(e) = state
                                .terminal_hub
                                .write_input(&session_id, data.into_bytes())
                                .await
                            {
                                warn!(
                                    "Failed to forward JSON input to PTY session {}: {:?}",
                                    session_id, e
                                );
                            }
                        }
                    }
                } else {
                    // Raw text input fallback
                    if let Err(e) = state
                        .terminal_hub
                        .write_input(&session_id, text.as_bytes().to_vec())
                        .await
                    {
                        warn!(
                            "Failed to forward text input to PTY session {}: {:?}",
                            session_id, e
                        );
                    }
                }
            }
            Ok(Message::Ping(payload)) => {
                session.update_activity();
                let _ = ws_out_tx.send(Message::Pong(payload)).await;
            }
            Ok(Message::Pong(_)) => {
                session.update_activity();
            }
            Ok(Message::Close(_)) => {
                info!(
                    "Terminal WebSocket client requested close for session {}",
                    session_id
                );
                break;
            }
            Err(e) => {
                warn!("WebSocket read error for session {}: {:?}", session_id, e);
                break;
            }
        }
    }

    // Clean teardown of companion tasks
    heartbeat_task.abort();
    pty_listener_task.abort();
    drop(ws_out_tx); // Signals ws_writer_task to drain and terminate
    let _ = ws_writer_task.await;

    info!(
        "Terminal WebSocket stream disconnected for session {}",
        session_id
    );
}
