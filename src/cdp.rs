//! WebSocket JSON-RPC client for Chrome extension bridge communication.
//!
//! BridgeClient is the transport layer between the Rust MCP server and the
//! Chrome extension. It sends method calls and receives responses over WebSocket.

use std::collections::HashMap;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::{tungstenite::Message, MaybeTlsStream, WebSocketStream};

/// JSON-RPC request sent over the bridge.
#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    id: u64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

/// JSON-RPC response received from the bridge.
#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    id: Option<u64>,
    result: Option<Value>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

impl std::fmt::Display for JsonRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RPC error {}: {}", self.code, self.message)
    }
}

impl std::error::Error for JsonRpcError {}

type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, JsonRpcError>>>>>;

/// WebSocket JSON-RPC client for communicating with the Chrome extension.
#[derive(Clone)]
pub struct BridgeClient {
    tx: mpsc::Sender<Message>,
    pending: PendingMap,
    next_id: Arc<Mutex<u64>>,
}

impl BridgeClient {
    /// Connect from a raw TCP stream (Chrome extension connects to our WebSocket server).
    pub async fn connect_from_stream(
        tcp_stream: TcpStream,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let tls_stream = MaybeTlsStream::Plain(tcp_stream);
        let ws_stream = tokio_tungstenite::accept_async(tls_stream).await?;
        let (write, read) = ws_stream.split();

        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let (tx, mut rx) = mpsc::channel::<Message>(64);

        // Write loop — sends messages from tx channel to WebSocket
        let mut write = write;
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if write.send(msg).await.is_err() {
                    break;
                }
            }
        });

        // Read loop — routes responses to pending senders
        let pending_clone = pending.clone();
        tokio::spawn(async move {
            Self::read_loop(read, pending_clone).await;
        });

        Ok(Self {
            tx,
            pending,
            next_id: Arc::new(Mutex::new(1)),
        })
    }

    /// Read WebSocket messages and route responses to pending callers.
    async fn read_loop(
        mut read: futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
        pending: PendingMap,
    ) {
        while let Some(Ok(msg)) = read.next().await {
            let Message::Text(text) = msg else {
                continue;
            };

            let resp: JsonRpcResponse = match serde_json::from_str(&text) {
                Ok(r) => r,
                Err(_) => continue,
            };

            // Skip events (no id) — only handle responses
            let Some(id) = resp.id else { continue };

            let mut map = pending.lock().await;
            if let Some(sender) = map.remove(&id) {
                let result = match resp.error {
                    Some(err) => Err(err),
                    None => Ok(resp.result.unwrap_or(Value::Null)),
                };
                let _ = sender.send(result);
            }
        }
    }

    /// Send a method call and wait for the response.
    pub async fn send(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        let id = {
            let mut next = self.next_id.lock().await;
            let id = *next;
            *next += 1;
            id
        };

        let req = JsonRpcRequest {
            id,
            method: method.to_string(),
            params,
        };

        let (resp_tx, resp_rx) = oneshot::channel();
        self.pending.lock().await.insert(id, resp_tx);

        let json = serde_json::to_string(&req)?;
        self.tx.send(Message::Text(json.into())).await?;

        match resp_rx.await? {
            Ok(value) => Ok(value),
            Err(rpc_err) => Err(Box::new(rpc_err)),
        }
    }

    /// Navigate to a URL via the extension bridge.
    pub async fn navigate(&self, url: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.send("Page.navigate", Some(serde_json::json!({ "url": url })))
            .await?;
        Ok(())
    }

    /// Evaluate a JavaScript expression via the extension bridge.
    pub async fn evaluate(&self, expression: &str) -> Result<Value, Box<dyn std::error::Error>> {
        let result = self
            .send(
                "Runtime.evaluate",
                Some(serde_json::json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": true,
                })),
            )
            .await?;

        // Check for exception
        if let Some(exception) = result.get("exceptionDetails") {
            let text = exception["exception"]["description"]
                .as_str()
                .or_else(|| exception["text"].as_str())
                .unwrap_or("unknown JS error");
            return Err(text.to_string().into());
        }

        Ok(result
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(Value::Null))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_request_with_params() {
        let req = JsonRpcRequest {
            id: 1,
            method: "Page.navigate".to_string(),
            params: Some(serde_json::json!({"url": "https://example.com"})),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("Page.navigate"));
        assert!(json.contains("example.com"));
    }

    #[test]
    fn serialize_request_without_params() {
        let req = JsonRpcRequest {
            id: 1,
            method: "Bridge.ping".to_string(),
            params: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("params"));
    }

    #[test]
    fn deserialize_success_response() {
        let json = r#"{"id": 1, "result": {"value": 42}}"#;
        let resp: JsonRpcResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.id, Some(1));
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
    }

    #[test]
    fn deserialize_error_response() {
        let json = r#"{"id": 1, "error": {"code": -32000, "message": "not found"}}"#;
        let resp: JsonRpcResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.id, Some(1));
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32000);
        assert_eq!(err.message, "not found");
    }
}
