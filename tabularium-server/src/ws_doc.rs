//! Document WebSocket (`GET /ws`): `op`-only JSON frames (`ws://` or `wss://` from clients).

use std::sync::Arc;

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use serde::{Deserialize, Deserializer};
use serde_json::json;
use tabularium::resource_path::normalize_path_for_rpc;
use tabularium::text_lines::{TailMode, apply_tail_logical_lines};
use tabularium::{EntryId, Error, SqliteDatabase};
use tokio::sync::watch as wait_cell;
use tracing::info;

use crate::web::AppState;

fn deserialize_subscribe_lines<'de, D>(deserializer: D) -> Result<TailMode, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum LinesWire {
        Num(u64),
        Text(String),
    }
    match LinesWire::deserialize(deserializer)? {
        LinesWire::Num(n) => u32::try_from(n)
            .map(TailMode::Last)
            .map_err(|_| serde::de::Error::custom("lines: number out of range")),
        LinesWire::Text(s) => TailMode::from_plus_wire_str(&s).map_err(serde::de::Error::custom),
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
enum ClientMsg {
    Subscribe {
        path: String,
        #[serde(deserialize_with = "deserialize_subscribe_lines")]
        lines: TailMode,
    },
    Unsubscribe {
        path: String,
    },
    Append {
        path: String,
        data: String,
    },
    Say {
        path: String,
        from_id: String,
        data: String,
    },
}

struct ActiveSub {
    path: String,
    did: EntryId,
    tail: TailMode,
    last_full: String,
    rx: wait_cell::Receiver<u64>,
}

pub async fn ws_upgrade(ws: WebSocketUpgrade, State(st): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, st.db.clone()))
}

async fn resolve_did(db: &SqliteDatabase, path: &str) -> tabularium::Result<EntryId> {
    let p = normalize_path_for_rpc(path)?;
    db.resolve_existing_file_path(&p).await
}

async fn send_json(socket: &mut WebSocket, v: &serde_json::Value) -> bool {
    let Ok(text) = serde_json::to_string(v) else {
        return false;
    };
    socket.send(Message::Text(text.into())).await.is_ok()
}

async fn send_err(socket: &mut WebSocket, msg: impl Into<String>) -> bool {
    send_json(socket, &json!({ "op": "error", "message": msg.into() })).await
}

async fn handle_client_text(
    db: &SqliteDatabase,
    socket: &mut WebSocket,
    text: &str,
    active: &mut Option<ActiveSub>,
) -> bool {
    let msg: ClientMsg = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(e) => {
            return send_err(socket, format!("invalid json: {e}")).await;
        }
    };

    match msg {
        ClientMsg::Subscribe { path, lines: tail } => {
            let path = match normalize_path_for_rpc(&path) {
                Ok(x) => x,
                Err(e) => return send_err(socket, e.to_string()).await,
            };
            let did = match resolve_did(db, &path).await {
                Ok(d) => d,
                Err(e) => return send_err(socket, e.to_string()).await,
            };
            let mut rx = match db.subscribe_document_wait(did).await {
                Ok(r) => r,
                Err(e) => return send_err(socket, e.to_string()).await,
            };
            let _ = rx.borrow_and_update();
            let full = match db.get_document(did).await {
                Ok(b) => b,
                Err(e) => return send_err(socket, e.to_string()).await,
            };
            let view = apply_tail_logical_lines(&full, tail);
            if !send_json(
                socket,
                &json!({ "op": "reset", "path": &path, "data": view }),
            )
            .await
            {
                return false;
            }
            *active = Some(ActiveSub {
                path,
                did,
                tail,
                last_full: full,
                rx,
            });
            true
        }
        ClientMsg::Unsubscribe { path } => {
            let p = match normalize_path_for_rpc(&path) {
                Ok(x) => x,
                Err(e) => return send_err(socket, e.to_string()).await,
            };
            if active.as_ref().is_some_and(|s| s.path == p) {
                *active = None;
            }
            true
        }
        ClientMsg::Append { path, data } => {
            let p = match normalize_path_for_rpc(&path) {
                Ok(x) => x,
                Err(e) => return send_err(socket, e.to_string()).await,
            };
            if let Err(e) = db.append_document_by_path(&p, &data).await {
                return send_err(socket, e.to_string()).await;
            }
            info!(
                target: "tabularium_server::api",
                op = "append",
                %path,
                append_len = data.len(),
                "WebSocket document write"
            );
            true
        }
        ClientMsg::Say {
            path,
            from_id,
            data,
        } => {
            let p = match normalize_path_for_rpc(&path) {
                Ok(x) => x,
                Err(e) => return send_err(socket, e.to_string()).await,
            };
            if let Err(e) = db.say_document_by_path(&p, &from_id, &data).await {
                return send_err(socket, e.to_string()).await;
            }
            info!(
                target: "tabularium_server::api",
                op = "say",
                %path,
                from_id = %from_id,
                content_len = data.len(),
                "WebSocket document write"
            );
            true
        }
    }
}

async fn on_doc_changed(db: &SqliteDatabase, socket: &mut WebSocket, sub: &mut ActiveSub) -> bool {
    let new_full = match db.get_document(sub.did).await {
        Ok(b) => b,
        Err(e) => {
            return send_err(socket, e.to_string()).await;
        }
    };

    if new_full.starts_with(&sub.last_full) && new_full.len() >= sub.last_full.len() {
        let suffix = &new_full[sub.last_full.len()..];
        if !suffix.is_empty()
            && !send_json(
                socket,
                &json!({ "op": "append", "path": &sub.path, "data": suffix }),
            )
            .await
        {
            return false;
        }
        sub.last_full = new_full;
        return true;
    }

    let view = apply_tail_logical_lines(&new_full, sub.tail);
    if !send_json(
        socket,
        &json!({ "op": "reset", "path": &sub.path, "data": view }),
    )
    .await
    {
        return false;
    }
    sub.last_full = new_full;
    true
}

async fn handle_socket(mut socket: WebSocket, db: Arc<SqliteDatabase>) {
    let mut active: Option<ActiveSub> = None;

    loop {
        if let Some(mut sub) = active.take() {
            tokio::select! {
                ch = sub.rx.changed() => {
                    if ch.is_err() {
                        let _ = send_err(
                            &mut socket,
                            Error::InvalidInput("document wait closed".into()).to_string(),
                        )
                        .await;
                        break;
                    }
                    let _ = sub.rx.borrow_and_update();
                    if !on_doc_changed(&db, &mut socket, &mut sub).await {
                        break;
                    }
                    active = Some(sub);
                }
                incoming = socket.recv() => {
                    let Some(frame) = incoming else {
                        break;
                    };
                    match frame {
                        Ok(Message::Text(t)) => {
                            active = Some(sub);
                            if !handle_client_text(&db, &mut socket, t.as_str(), &mut active).await
                            {
                                break;
                            }
                        }
                        Ok(Message::Ping(p)) => {
                            if socket.send(Message::Pong(p)).await.is_err() {
                                break;
                            }
                            active = Some(sub);
                        }
                        Ok(Message::Pong(_) | Message::Binary(_)) => {
                            active = Some(sub);
                        }
                        Ok(Message::Close(_)) | Err(_) => break,
                    }
                }
            }
        } else {
            let Some(frame) = socket.recv().await else {
                break;
            };
            match frame {
                Ok(Message::Text(t)) => {
                    if !handle_client_text(&db, &mut socket, t.as_str(), &mut active).await {
                        break;
                    }
                }
                Ok(Message::Ping(p)) => {
                    if socket.send(Message::Pong(p)).await.is_err() {
                        break;
                    }
                }
                Ok(Message::Pong(_) | Message::Binary(_)) => {}
                Ok(Message::Close(_)) | Err(_) => break,
            }
        }
    }
}
