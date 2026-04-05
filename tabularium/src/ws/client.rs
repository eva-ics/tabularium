//! `tokio-tungstenite` document WebSocket client.

use std::path::Path;

use futures_util::{SinkExt, StreamExt};
use reqwest::header::HeaderMap;
use serde::Deserialize;
use serde::de::Deserializer;
use serde_json::json;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::Result;
use crate::resource_path::normalize_path_for_rpc;
use crate::text_lines::TailMode;

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

fn json_opt_str(m: &serde_json::Value, k: &str) -> Option<String> {
    m.get(k).and_then(|x| x.as_str()).map(String::from)
}

/// Server → client frames (`op`-only schema).
#[derive(Debug, Clone)]
pub enum RecvMessage {
    Reset {
        path: Option<String>,
        data: Option<String>,
    },
    Append {
        path: Option<String>,
        data: Option<String>,
    },
    Error {
        message: Option<String>,
    },
    Unknown {
        op: String,
    },
}

impl<'de> Deserialize<'de> for RecvMessage {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(deserializer)?;
        let op = v
            .get("op")
            .and_then(|x| x.as_str())
            .ok_or_else(|| serde::de::Error::custom("missing op"))?;
        Ok(match op {
            "reset" => RecvMessage::Reset {
                path: json_opt_str(&v, "path"),
                data: json_opt_str(&v, "data"),
            },
            "append" => RecvMessage::Append {
                path: json_opt_str(&v, "path"),
                data: json_opt_str(&v, "data"),
            },
            "error" => RecvMessage::Error {
                message: json_opt_str(&v, "message"),
            },
            other => RecvMessage::Unknown {
                op: other.to_string(),
            },
        })
    }
}

/// WebSocket client for `GET /ws` (`ws://` / `wss://` from [`ws_url_from_http_base`]).
pub struct Client {
    stream: WsStream,
}

impl Client {
    /// Connect to `ws://host:port/ws` (or `http` base URL rewritten to `ws`).
    pub async fn connect(api_base: impl AsRef<str>) -> Result<Self> {
        Self::connect_with_headers(api_base, &HeaderMap::new()).await
    }

    /// Same as [`Self::connect`], with extra HTTP headers on the WebSocket upgrade (same map as [`crate::rpc::Client::extra_headers`]).
    pub async fn connect_with_headers(
        api_base: impl AsRef<str>,
        headers: &HeaderMap,
    ) -> Result<Self> {
        let url = ws_url_from_http_base(api_base.as_ref())?;
        let mut req = url
            .as_str()
            .into_client_request()
            .map_err(|e| crate::Error::InvalidInput(format!("ws request: {e}")))?;
        for (name, v) in headers {
            req.headers_mut().insert(name, v.clone());
        }
        let (stream, _) = tokio_tungstenite::connect_async(req)
            .await
            .map_err(|e| crate::Error::InvalidInput(format!("ws connect: {e}")))?;
        Ok(Self { stream })
    }

    /// `{"op":"subscribe","path":"cat/doc","lines":N}` — numeric `N` = last `N` lines (`0` = none); or `"+K"` from-line-`K` (full doc from start = `"+1"`).
    pub async fn subscribe(&mut self, path: impl AsRef<Path>, mode: TailMode) -> Result<()> {
        let path = normalize_path_for_rpc(path)?;
        let lines_val = match mode {
            TailMode::Last(n) => json!(n),
            TailMode::FromLine(n) => json!(format!("+{n}")),
        };
        let text = json!({
            "op": "subscribe",
            "path": path,
            "lines": lines_val,
        })
        .to_string();
        self.stream
            .send(Message::Text(text.into()))
            .await
            .map_err(|e| crate::Error::InvalidInput(format!("ws send: {e}")))?;
        Ok(())
    }

    /// `{"op":"unsubscribe","path":"cat/doc"}`.
    pub async fn unsubscribe(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = normalize_path_for_rpc(path)?;
        let text = json!({
            "op": "unsubscribe",
            "path": path,
        })
        .to_string();
        self.stream
            .send(Message::Text(text.into()))
            .await
            .map_err(|e| crate::Error::InvalidInput(format!("ws send: {e}")))?;
        Ok(())
    }

    /// `{"op":"append","path":"cat/doc","data":"..."}`.
    pub async fn append(&mut self, path: impl AsRef<Path>, data: impl AsRef<str>) -> Result<()> {
        let path = normalize_path_for_rpc(path)?;
        let text = json!({
            "op": "append",
            "path": path,
            "data": data.as_ref(),
        })
        .to_string();
        self.stream
            .send(Message::Text(text.into()))
            .await
            .map_err(|e| crate::Error::InvalidInput(format!("ws send: {e}")))?;
        Ok(())
    }

    /// `{"op":"say","path":"cat/doc","from_id":"…","data":"…"}` — server appends a markdown chat block.
    pub async fn say(
        &mut self,
        path: impl AsRef<Path>,
        from_id: impl AsRef<str>,
        data: impl AsRef<str>,
    ) -> Result<()> {
        let path = normalize_path_for_rpc(path)?;
        let text = json!({
            "op": "say",
            "path": path,
            "from_id": from_id.as_ref(),
            "data": data.as_ref(),
        })
        .to_string();
        self.stream
            .send(Message::Text(text.into()))
            .await
            .map_err(|e| crate::Error::InvalidInput(format!("ws send: {e}")))?;
        Ok(())
    }

    /// Next text JSON frame, skipping pings. Returns `None` on close.
    pub async fn recv(&mut self) -> Result<Option<RecvMessage>> {
        loop {
            let Some(frame) = self
                .stream
                .next()
                .await
                .transpose()
                .map_err(|e| crate::Error::InvalidInput(format!("ws recv: {e}")))?
            else {
                return Ok(None);
            };
            match frame {
                Message::Text(t) => {
                    let v: RecvMessage = serde_json::from_str(t.as_str())
                        .map_err(|e| crate::Error::InvalidInput(format!("ws json: {e}")))?;
                    return Ok(Some(v));
                }
                Message::Ping(p) => {
                    self.stream
                        .send(Message::Pong(p))
                        .await
                        .map_err(|e| crate::Error::InvalidInput(format!("ws pong: {e}")))?;
                }
                Message::Close(_) => return Ok(None),
                Message::Frame(_) | Message::Pong(_) | Message::Binary(_) => {}
            }
        }
    }

    /// Close the socket gracefully.
    pub async fn close(mut self) -> Result<()> {
        let _ = self.stream.close(None).await;
        Ok(())
    }
}

/// Map `http://host:port` → `ws://host:port/ws`.
pub fn ws_url_from_http_base(base: &str) -> Result<String> {
    let base = base.trim_end_matches('/');
    let rest = base
        .strip_prefix("http://")
        .or_else(|| base.strip_prefix("https://"))
        .ok_or_else(|| {
            crate::Error::InvalidInput("api URL must start with http:// or https://".into())
        })?;
    let scheme = if base.starts_with("https://") {
        "wss"
    } else {
        "ws"
    };
    Ok(format!("{scheme}://{rest}/ws"))
}

#[cfg(test)]
mod tests {
    use super::{RecvMessage, ws_url_from_http_base};

    #[test]
    fn https_base_maps_to_wss_path() {
        assert_eq!(
            ws_url_from_http_base("https://example.com:8443/app").unwrap(),
            "wss://example.com:8443/app/ws"
        );
    }

    #[test]
    fn http_base_maps_to_ws_path() {
        assert_eq!(
            ws_url_from_http_base("http://127.0.0.1:3050").unwrap(),
            "ws://127.0.0.1:3050/ws"
        );
    }

    #[test]
    fn recv_message_parses_known_ops() {
        let r: RecvMessage =
            serde_json::from_str(r#"{"op":"append","path":"a/b","data":"x"}"#).unwrap();
        match r {
            RecvMessage::Append { path, data } => {
                assert_eq!(path.as_deref(), Some("a/b"));
                assert_eq!(data.as_deref(), Some("x"));
            }
            _ => panic!("expected Append"),
        }
    }

    #[test]
    fn recv_message_unknown_op_round_trip() {
        let r: RecvMessage = serde_json::from_str(r#"{"op":"future","x":1}"#).unwrap();
        match r {
            RecvMessage::Unknown { op } => assert_eq!(op, "future"),
            _ => panic!("expected Unknown"),
        }
    }
}
