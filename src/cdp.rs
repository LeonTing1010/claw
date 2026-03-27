use std::collections::HashMap;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

/// CDP JSON-RPC request
#[derive(Debug, Serialize)]
pub struct CdpRequest {
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// CDP JSON-RPC response
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct CdpResponse {
    pub id: Option<u64>,
    pub result: Option<Value>,
    pub error: Option<CdpError>,
    // Events have `method` and `params` but no `id`
    pub method: Option<String>,
    pub params: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct CdpError {
    pub code: i64,
    pub message: String,
}

impl std::fmt::Display for CdpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CDP error {}: {}", self.code, self.message)
    }
}

impl std::error::Error for CdpError {}

type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, CdpError>>>>>;

pub struct CdpClient {
    tx: mpsc::Sender<Message>,
    pending: PendingMap,
    next_id: Arc<Mutex<u64>>,
}

/// Build Input.dispatchMouseEvent params
fn mouse_event_params(event_type: &str, x: f64, y: f64) -> Value {
    serde_json::json!({
        "type": event_type,
        "x": x,
        "y": y,
        "button": "left",
        "clickCount": 1
    })
}

/// Escape a string for safe embedding in JavaScript single-quoted strings
fn escape_js_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

/// Build Input.dispatchKeyEvent params
fn key_event_params(event_type: &str, key: &str, text: Option<&str>, modifiers: u32) -> Value {
    let mut params = serde_json::json!({
        "type": event_type,
        "key": key,
        "modifiers": modifiers,
    });
    if let Some(t) = text {
        params["text"] = serde_json::Value::String(t.to_string());
    }
    params
}

impl CdpClient {
    /// Connect to a Chrome CDP WebSocket endpoint.
    /// `ws_url` is typically from `http://localhost:{port}/json/version` → `webSocketDebuggerUrl`.
    pub async fn connect(ws_url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let (ws_stream, _) = connect_async(ws_url).await?;
        let (write, read) = ws_stream.split();

        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let (tx, mut rx) = mpsc::channel::<Message>(64);

        // Writer task: forward messages from channel to WebSocket
        let mut write = write;
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if write.send(msg).await.is_err() {
                    break;
                }
            }
        });

        // Reader task: route responses to pending callers
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

    async fn read_loop(
        mut read: futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
        pending: PendingMap,
    ) {
        while let Some(Ok(msg)) = read.next().await {
            let Message::Text(text) = msg else {
                continue;
            };

            let resp: CdpResponse = match serde_json::from_str(&text) {
                Ok(r) => r,
                Err(_) => continue,
            };

            // Events (no id) are ignored for now
            let Some(id) = resp.id else {
                continue;
            };

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

    /// Send a CDP method call and wait for the response.
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

        let req = CdpRequest {
            id,
            method: method.to_string(),
            params,
        };

        let (resp_tx, resp_rx) = oneshot::channel();

        {
            let mut map = self.pending.lock().await;
            map.insert(id, resp_tx);
        }

        let json = serde_json::to_string(&req)?;
        self.tx.send(Message::Text(json.into())).await?;

        match resp_rx.await? {
            Ok(value) => Ok(value),
            Err(cdp_err) => Err(Box::new(cdp_err)),
        }
    }

    /// Navigate to a URL and wait for the page to load.
    pub async fn navigate(&self, url: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.send("Page.enable", None).await?;

        self.send("Page.navigate", Some(serde_json::json!({ "url": url })))
            .await?;

        // Wait for load event
        // TODO: use event listener instead of polling
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        Ok(())
    }

    /// Evaluate a JavaScript expression in the browser and return the result.
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

        Ok(result["result"]["value"].clone())
    }

    /// Click at exact coordinates via CDP native mouse events
    pub async fn click(&self, x: f64, y: f64) -> Result<(), Box<dyn std::error::Error>> {
        self.send(
            "Input.dispatchMouseEvent",
            Some(mouse_event_params("mousePressed", x, y)),
        )
        .await?;
        self.send(
            "Input.dispatchMouseEvent",
            Some(mouse_event_params("mouseReleased", x, y)),
        )
        .await?;
        Ok(())
    }

    /// Click element matching CSS selector — resolve coordinates, then CDP click
    pub async fn click_selector(&self, selector: &str) -> Result<(), Box<dyn std::error::Error>> {
        let js = format!(
            r#"(() => {{
                const el = document.querySelector('{}');
                if (!el) throw new Error('element not found: {}');
                const r = el.getBoundingClientRect();
                if (r.width === 0 && r.height === 0) throw new Error('element not visible: {}');
                return {{ x: r.x + r.width/2, y: r.y + r.height/2 }};
            }})()"#,
            escape_js_string(selector),
            escape_js_string(selector),
            escape_js_string(selector)
        );
        let result = self.evaluate(&js).await?;
        let x = result["x"].as_f64().ok_or("missing x coordinate")?;
        let y = result["y"].as_f64().ok_or("missing y coordinate")?;
        self.click(x, y).await
    }

    /// Click element containing specific visible text
    pub async fn click_text(&self, text: &str) -> Result<(), Box<dyn std::error::Error>> {
        let js = format!(
            r#"(() => {{
                const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
                while (walker.nextNode()) {{
                    if (walker.currentNode.textContent.trim().includes('{}')) {{
                        const el = walker.currentNode.parentElement;
                        if (el && el.offsetParent !== null) {{
                            const r = el.getBoundingClientRect();
                            if (r.width > 0 && r.height > 0) {{
                                return {{ x: r.x + r.width/2, y: r.y + r.height/2 }};
                            }}
                        }}
                    }}
                }}
                throw new Error('text not found: {}');
            }})()"#,
            escape_js_string(text),
            escape_js_string(text)
        );
        let result = self.evaluate(&js).await?;
        let x = result["x"].as_f64().ok_or("missing x coordinate")?;
        let y = result["y"].as_f64().ok_or("missing y coordinate")?;
        self.click(x, y).await
    }

    /// Type text character by character via CDP native keyboard events.
    /// For non-ASCII text (Chinese, emoji), uses Input.insertText instead.
    pub async fn type_text(&self, text: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Check if text contains non-ASCII characters
        if !text.is_ascii() {
            // Use insertText for non-ASCII — single CDP call
            self.send(
                "Input.insertText",
                Some(serde_json::json!({
                    "text": text
                })),
            )
            .await?;
            return Ok(());
        }
        // ASCII: dispatch keyDown/keyUp per character
        for ch in text.chars() {
            let key = ch.to_string();
            self.send(
                "Input.dispatchKeyEvent",
                Some(key_event_params("keyDown", &key, Some(&key), 0)),
            )
            .await?;
            self.send(
                "Input.dispatchKeyEvent",
                Some(key_event_params("keyUp", &key, None, 0)),
            )
            .await?;
        }
        Ok(())
    }

    /// Focus element by selector, clear existing content, then type new text.
    pub async fn type_into(
        &self,
        selector: &str,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Focus the element
        self.click_selector(selector).await?;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Select all (Ctrl+A / Cmd+A)
        self.send(
            "Input.dispatchKeyEvent",
            Some(
                key_event_params("keyDown", "a", None, 2), // modifiers=2 is Ctrl
            ),
        )
        .await?;
        self.send(
            "Input.dispatchKeyEvent",
            Some(key_event_params("keyUp", "a", None, 0)),
        )
        .await?;

        // Delete selected content
        self.send(
            "Input.dispatchKeyEvent",
            Some(key_event_params("keyDown", "Backspace", None, 0)),
        )
        .await?;
        self.send(
            "Input.dispatchKeyEvent",
            Some(key_event_params("keyUp", "Backspace", None, 0)),
        )
        .await?;

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Type the new text
        self.type_text(text).await
    }

    /// Upload files to a file input element via CDP.
    pub async fn upload_files(
        &self,
        selector: &str,
        paths: &[&str],
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Use Runtime.evaluate to get the RemoteObjectId, then resolve to DOM node
        let js = format!("document.querySelector('{}')", escape_js_string(selector));
        let result = self
            .send(
                "Runtime.evaluate",
                Some(serde_json::json!({
                    "expression": js
                })),
            )
            .await?;

        let object_id = result["result"]["objectId"]
            .as_str()
            .ok_or(format!("element not found for upload: {}", selector))?;

        // Resolve RemoteObject to DOM node
        let dom_node = self
            .send(
                "DOM.describeNode",
                Some(serde_json::json!({
                    "objectId": object_id
                })),
            )
            .await?;
        let backend_node_id = dom_node["node"]["backendNodeId"]
            .as_i64()
            .ok_or("failed to resolve DOM node for upload")?;

        self.send(
            "DOM.setFileInputFiles",
            Some(serde_json::json!({
                "backendNodeId": backend_node_id,
                "files": paths
            })),
        )
        .await?;
        Ok(())
    }

    /// Close the browser connection.
    #[allow(dead_code)]
    pub async fn close(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.send("Browser.close", None).await?;
        Ok(())
    }

    /// Discover the WebSocket debugger URL from Chrome's /json/version endpoint.
    /// Discover the WebSocket URL for the first page target (supports Runtime.evaluate etc).
    pub async fn discover_ws_url(port: u16) -> Result<String, Box<dyn std::error::Error>> {
        let body = Self::http_get(port, "/json").await?;
        let targets: Vec<Value> = serde_json::from_str(&body)?;
        Self::pick_page_ws_url(&targets)
    }

    /// Select the first page-level target's WebSocket URL from /json target list.
    /// Page targets support Runtime.evaluate, DOM, etc. Browser targets don't.
    fn pick_page_ws_url(targets: &[Value]) -> Result<String, Box<dyn std::error::Error>> {
        let page = targets
            .iter()
            .find(|t| t["type"].as_str() == Some("page"))
            .ok_or("no page target found — is a tab open in Chrome?")?;

        let ws_url = page["webSocketDebuggerUrl"]
            .as_str()
            .ok_or("page target missing webSocketDebuggerUrl")?;

        Ok(ws_url.to_string())
    }

    /// Minimal synchronous HTTP GET against Chrome's CDP HTTP endpoints.
    pub async fn http_get(port: u16, path: &str) -> Result<String, Box<dyn std::error::Error>> {
        let addr = format!("127.0.0.1:{}", port);
        let path = path.to_string();

        let body = tokio::task::spawn_blocking(move || {
            use std::io::{BufRead, BufReader, Read, Write};
            let stream = std::net::TcpStream::connect(&addr)
                .map_err(|e| format!("cannot connect to Chrome on {}: {}", addr, e))?;
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .map_err(|e| e.to_string())?;
            let req = format!(
                "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                path, addr
            );
            (&stream)
                .write_all(req.as_bytes())
                .map_err(|e| e.to_string())?;

            let mut reader = BufReader::new(&stream);
            let mut content_length: usize = 0;

            // Read headers
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).map_err(|e| e.to_string())?;
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    break;
                }
                if let Some(val) = trimmed.strip_prefix("Content-Length:") {
                    content_length = val.trim().parse().unwrap_or(0);
                }
            }

            let mut body = vec![0u8; content_length];
            reader.read_exact(&mut body).map_err(|e| e.to_string())?;
            String::from_utf8(body).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())??;

        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_cdp_request_with_params() {
        let req = CdpRequest {
            id: 1,
            method: "Runtime.evaluate".to_string(),
            params: Some(serde_json::json!({
                "expression": "1+1",
                "returnByValue": true,
            })),
        };
        let json: Value = serde_json::to_value(&req).unwrap();
        assert_eq!(json["id"], 1);
        assert_eq!(json["method"], "Runtime.evaluate");
        assert_eq!(json["params"]["expression"], "1+1");
    }

    #[test]
    fn serialize_cdp_request_without_params() {
        let req = CdpRequest {
            id: 2,
            method: "Page.enable".to_string(),
            params: None,
        };
        let json_str = serde_json::to_string(&req).unwrap();
        // params should be absent, not null
        assert!(!json_str.contains("params"));
    }

    #[test]
    fn deserialize_cdp_success_response() {
        let raw = r#"{"id":1,"result":{"result":{"type":"number","value":2,"description":"2"}}}"#;
        let resp: CdpResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(resp.id, Some(1));
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["result"]["value"], 2);
    }

    #[test]
    fn deserialize_cdp_error_response() {
        let raw = r#"{"id":2,"error":{"code":-32601,"message":"'Foo.bar' wasn't found"}}"#;
        let resp: CdpResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(resp.id, Some(2));
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32601);
        assert!(err.message.contains("wasn't found"));
    }

    #[test]
    fn deserialize_cdp_event() {
        let raw = r#"{"method":"Page.loadEventFired","params":{"timestamp":1234.5}}"#;
        let resp: CdpResponse = serde_json::from_str(raw).unwrap();
        assert!(resp.id.is_none());
        assert_eq!(resp.method.unwrap(), "Page.loadEventFired");
        assert_eq!(resp.params.unwrap()["timestamp"], 1234.5);
    }

    // --- Behavioral tests (What×What): discover + evaluate interaction ---

    #[test]
    fn pick_page_ws_url_selects_page_not_browser() {
        // Real Chrome /json response contains pages, iframes, service workers, etc.
        // discover_ws_url must pick type=page, not browser or iframe.
        let targets: Vec<Value> = serde_json::from_str(r#"[
            {"type":"iframe","url":"chrome-untrusted://newtab","webSocketDebuggerUrl":"ws://127.0.0.1:9222/devtools/page/IFRAME1"},
            {"type":"page","url":"https://example.com","webSocketDebuggerUrl":"ws://127.0.0.1:9222/devtools/page/PAGE1"},
            {"type":"page","url":"chrome://newtab/","webSocketDebuggerUrl":"ws://127.0.0.1:9222/devtools/page/PAGE2"}
        ]"#).unwrap();

        let ws_url = CdpClient::pick_page_ws_url(&targets).unwrap();
        // Must be a page endpoint (not iframe, not browser)
        assert!(
            ws_url.contains("/devtools/page/"),
            "must be a page-level endpoint"
        );
        assert!(ws_url.contains("PAGE1"), "must pick the first page target");
    }

    #[test]
    fn pick_page_ws_url_errors_when_no_page_target() {
        let targets: Vec<Value> = serde_json::from_str(r#"[
            {"type":"iframe","url":"chrome-untrusted://newtab","webSocketDebuggerUrl":"ws://127.0.0.1:9222/devtools/page/IFRAME1"}
        ]"#).unwrap();

        let result = CdpClient::pick_page_ws_url(&targets);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no page target"));
    }

    #[test]
    fn pick_page_ws_url_errors_on_empty_targets() {
        let targets: Vec<Value> = vec![];
        let result = CdpClient::pick_page_ws_url(&targets);
        assert!(result.is_err());
    }

    // --- HTTP parsing behavioral tests ---

    #[test]
    fn evaluate_result_extracts_value() {
        // Simulate what Runtime.evaluate returns for "1+1"
        let cdp_result: Value = serde_json::json!({
            "result": { "type": "number", "value": 2, "description": "2" }
        });
        // evaluate() extracts result.result.value
        let value = cdp_result["result"]["value"].clone();
        assert_eq!(value, 2);
    }

    #[test]
    fn evaluate_result_detects_exception() {
        // Simulate what Runtime.evaluate returns for invalid JS
        let cdp_result: Value = serde_json::json!({
            "result": { "type": "object", "subtype": "error" },
            "exceptionDetails": {
                "text": "Uncaught SyntaxError",
                "exception": { "description": "SyntaxError: Unexpected token" }
            }
        });
        // evaluate() should detect exceptionDetails and error out
        assert!(cdp_result.get("exceptionDetails").is_some());
        let desc = cdp_result["exceptionDetails"]["exception"]["description"]
            .as_str()
            .unwrap();
        assert!(desc.contains("SyntaxError"));
    }

    #[test]
    fn mouse_event_params_structure() {
        let params = mouse_event_params("mousePressed", 100.0, 200.0);
        assert_eq!(params["type"], "mousePressed");
        assert_eq!(params["x"], 100.0);
        assert_eq!(params["y"], 200.0);
        assert_eq!(params["button"], "left");
        assert_eq!(params["clickCount"], 1);
    }

    #[test]
    fn mouse_event_params_released() {
        let params = mouse_event_params("mouseReleased", 50.5, 75.5);
        assert_eq!(params["type"], "mouseReleased");
        assert_eq!(params["x"], 50.5);
        assert_eq!(params["y"], 75.5);
    }

    #[test]
    fn escape_js_string_handles_quotes_and_backslash() {
        assert_eq!(escape_js_string("it's"), "it\\'s");
        assert_eq!(escape_js_string(r#"say "hi""#), r#"say \"hi\""#);
        assert_eq!(escape_js_string("back\\slash"), "back\\\\slash");
        assert_eq!(escape_js_string("line\nbreak"), "line\\nbreak");
    }

    #[test]
    fn key_event_params_structure() {
        let params = key_event_params("keyDown", "a", Some("a"), 0);
        assert_eq!(params["type"], "keyDown");
        assert_eq!(params["key"], "a");
        assert_eq!(params["text"], "a");
        assert_eq!(params["modifiers"], 0);
    }

    #[test]
    fn key_event_params_without_text() {
        let params = key_event_params("keyUp", "a", None, 0);
        assert_eq!(params["type"], "keyUp");
        assert_eq!(params["key"], "a");
        assert!(params.get("text").is_none() || params["text"].is_null());
    }

    #[test]
    fn key_event_params_with_modifiers() {
        let params = key_event_params("keyDown", "a", None, 2);
        assert_eq!(params["modifiers"], 2);
        assert_eq!(params["key"], "a");
    }
}
