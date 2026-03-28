use std::collections::HashMap;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::{tungstenite::Message, MaybeTlsStream, WebSocketStream};


/// JavaScript hook injected via Page.addScriptToEvaluateOnNewDocument.
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

/// A captured network request/response pair from CDP Network domain.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NetworkEntry {
    pub request_id: String,
    pub url: String,
    pub method: String,
    pub status: u16,
    pub mime_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_headers: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_headers: Option<Value>,
    /// Populated on demand via Network.getResponseBody
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
}

type NetworkLog = Arc<Mutex<Vec<NetworkEntry>>>;

/// Pending requests (requestWillBeSent but not yet responseReceived)
type PendingRequests = Arc<Mutex<HashMap<String, (String, String, Option<Value>)>>>; // requestId → (url, method, headers)


#[derive(Clone)]
pub struct CdpClient {
    tx: mpsc::Sender<Message>,
    pending: PendingMap,
    next_id: Arc<Mutex<u64>>,
    /// Network entries captured via CDP Network domain events
    network_log: NetworkLog,
    /// In-flight requests awaiting response (written by read_loop via Arc clone)
    #[allow(dead_code)]
    pending_requests: PendingRequests,
    /// Whether network capture is active
    network_capture_active: Arc<Mutex<bool>>,
    /// Fetch.requestPaused events buffer (active request interception)
    paused_fetch: Arc<Mutex<Vec<Value>>>,
}

/// Produce a valid JavaScript string literal (double-quoted, properly escaped).
/// Uses JSON serialization which handles all special characters correctly.
pub(crate) fn js_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| format!("\"{}\"", s))
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

    /// Connect from a raw TCP stream (used by Chrome extension bridge).
    /// Performs WebSocket handshake, then operates like a normal CDP connection.
    pub async fn connect_from_stream(
        tcp_stream: tokio::net::TcpStream,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let tls_stream = MaybeTlsStream::Plain(tcp_stream);
        let ws_stream = tokio_tungstenite::accept_async(tls_stream).await?;
        let (write, read) = ws_stream.split();

        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let network_log: NetworkLog = Arc::new(Mutex::new(Vec::new()));
        let pending_requests: PendingRequests = Arc::new(Mutex::new(HashMap::new()));
        let network_capture_active: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
        let paused_fetch: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let (tx, mut rx) = mpsc::channel::<Message>(64);

        let mut write = write;
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if write.send(msg).await.is_err() {
                    break;
                }
            }
        });

        let pending_clone = pending.clone();
        let net_log_clone = network_log.clone();
        let pending_req_clone = pending_requests.clone();
        let net_active_clone = network_capture_active.clone();
        let paused_fetch_clone = paused_fetch.clone();
        tokio::spawn(async move {
            Self::read_loop(
                read,
                pending_clone,
                net_log_clone,
                pending_req_clone,
                net_active_clone,
                paused_fetch_clone,
            )
            .await;
        });

        let client = Self {
            tx,
            pending,
            next_id: Arc::new(Mutex::new(1)),
            network_log,
            pending_requests,
            network_capture_active,
            paused_fetch,
        };
        Ok(client)
    }


    async fn read_loop(
        mut read: futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
        pending: PendingMap,
        network_log: NetworkLog,
        pending_requests: PendingRequests,
        network_capture_active: Arc<Mutex<bool>>,
        paused_fetch: Arc<Mutex<Vec<Value>>>,
    ) {
        while let Some(Ok(msg)) = read.next().await {
            let Message::Text(text) = msg else {
                continue;
            };

            let resp: CdpResponse = match serde_json::from_str(&text) {
                Ok(r) => r,
                Err(_) => continue,
            };

            // Handle CDP events (no id)
            if resp.id.is_none() {
                if let Some(method) = &resp.method {
                    let params = resp.params.as_ref().cloned().unwrap_or(Value::Null);

                    // Fetch.requestPaused — always capture, regardless of network_capture_active
                    if method == "Fetch.requestPaused" {
                        paused_fetch.lock().await.push(params.clone());
                        continue;
                    }

                    let active = *network_capture_active.lock().await;
                    if active {
                        match method.as_str() {
                            "Network.requestWillBeSent" => {
                                let request_id =
                                    params["requestId"].as_str().unwrap_or("").to_string();
                                let url =
                                    params["request"]["url"].as_str().unwrap_or("").to_string();
                                let method = params["request"]["method"]
                                    .as_str()
                                    .unwrap_or("GET")
                                    .to_string();
                                let headers = params["request"].get("headers").cloned();
                                if !request_id.is_empty() {
                                    pending_requests
                                        .lock()
                                        .await
                                        .insert(request_id, (url, method, headers));
                                }
                            }
                            "Network.responseReceived" => {
                                let request_id =
                                    params["requestId"].as_str().unwrap_or("").to_string();
                                let response = &params["response"];
                                let status = response["status"].as_u64().unwrap_or(0) as u16;
                                let mime_type =
                                    response["mimeType"].as_str().unwrap_or("").to_string();
                                let response_headers = response.get("headers").cloned();

                                if let Some((url, method, request_headers)) =
                                    pending_requests.lock().await.remove(&request_id)
                                {
                                    network_log.lock().await.push(NetworkEntry {
                                        request_id,
                                        url,
                                        method,
                                        status,
                                        mime_type,
                                        request_headers,
                                        response_headers,
                                        body: None, // populated on demand
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }
                continue;
            }

            let id = resp.id.unwrap();
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
    /// Polls document.readyState until "complete" or timeout (30s).
    pub async fn navigate(&self, url: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.send("Page.enable", None).await?;

        self.send("Page.navigate", Some(serde_json::json!({ "url": url })))
            .await?;

        // Wait for document.readyState === "complete" (up to 30s)
        for _ in 0..60 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if let Ok(val) = self.evaluate("document.readyState").await {
                if val == "complete" {
                    return Ok(());
                }
            }
        }
        // Timeout — proceed anyway (page may still be usable)
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

    /// Click at exact coordinates via CDP native mouse events.
    /// Adds random jitter between press/release to mimic human timing.
    pub async fn click(&self, x: f64, y: f64) -> Result<(), Box<dyn std::error::Error>> {
        self.send(
            "Input.dispatchMouseEvent",
            Some(mouse_event_params("mousePressed", x, y)),
        )
        .await?;
        // Human-like delay between press and release (50-150ms)
        tokio::time::sleep(Self::random_delay(50, 150)).await;
        self.send(
            "Input.dispatchMouseEvent",
            Some(mouse_event_params("mouseReleased", x, y)),
        )
        .await?;
        Ok(())
    }

    /// Random delay for humanizing timing patterns
    fn random_delay(min_ms: u64, max_ms: u64) -> std::time::Duration {
        use std::time::{SystemTime, UNIX_EPOCH};
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_nanos() as u64;
        let delta = (seed % (max_ms - min_ms)) + min_ms;
        std::time::Duration::from_millis(delta)
    }

    // ================================================================
    // Pure-CDP DOM helpers — no JS injection
    // ================================================================

    /// Resolve a CSS selector to a DOM nodeId via pure CDP calls.
    /// Enables `DOM` domain, gets the document root, then queries.
    async fn resolve_selector(&self, selector: &str) -> Result<i64, Box<dyn std::error::Error>> {
        self.send("DOM.enable", None).await?;
        let doc = self
            .send("DOM.getDocument", Some(serde_json::json!({"depth": 0})))
            .await?;
        let root_id = doc["root"]["nodeId"]
            .as_i64()
            .ok_or("missing root nodeId")?;
        let node = self
            .send(
                "DOM.querySelector",
                Some(serde_json::json!({
                    "nodeId": root_id,
                    "selector": selector
                })),
            )
            .await?;
        let node_id = node["nodeId"]
            .as_i64()
            .filter(|&id| id != 0)
            .ok_or_else(|| format!("element not found: {}", selector))?;
        Ok(node_id)
    }

    /// Get the center (x, y) of an element's box model via pure CDP.
    /// Uses `DOM.getBoxModel` — no `getBoundingClientRect()` JS injection.
    async fn get_box_center(&self, node_id: i64) -> Result<(f64, f64), Box<dyn std::error::Error>> {
        let box_model = self
            .send(
                "DOM.getBoxModel",
                Some(serde_json::json!({"nodeId": node_id})),
            )
            .await?;
        // content quad is [x1,y1, x2,y2, x3,y3, x4,y4] — four corners
        let content = box_model["model"]["content"]
            .as_array()
            .ok_or("missing box model content quad")?;
        if content.len() < 8 {
            return Err("box model content quad has fewer than 8 values".into());
        }
        let (mut sum_x, mut sum_y) = (0.0, 0.0);
        for i in 0..4 {
            sum_x += content[i * 2].as_f64().unwrap_or(0.0);
            sum_y += content[i * 2 + 1].as_f64().unwrap_or(0.0);
        }
        let cx = sum_x / 4.0;
        let cy = sum_y / 4.0;
        if cx == 0.0 && cy == 0.0 {
            return Err("element not visible (zero-size box)".into());
        }
        Ok((cx, cy))
    }

    /// Resolve a CSS selector to its center coordinates via pure CDP.
    async fn resolve_selector_center(
        &self,
        selector: &str,
    ) -> Result<(f64, f64), Box<dyn std::error::Error>> {
        let node_id = self.resolve_selector(selector).await?;
        // Scroll into view first so coordinates are within viewport
        let _ = self
            .send(
                "DOM.scrollIntoViewIfNeeded",
                Some(serde_json::json!({"nodeId": node_id})),
            )
            .await;
        self.get_box_center(node_id).await
    }

    /// Click element matching CSS selector — pure CDP: resolve via DOM domain, then click
    pub async fn click_selector(&self, selector: &str) -> Result<(), Box<dyn std::error::Error>> {
        let (x, y) = self.resolve_selector_center(selector).await?;
        self.click(x, y).await
    }

    /// Click element containing specific visible text — pure CDP via DOM.performSearch (XPath)
    pub async fn click_text(&self, text: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.send("DOM.enable", None).await?;
        // XPath: find text nodes containing the target string, return their parent elements
        let escaped = text.replace('\'', "\\'");
        let xpath = format!("//*[contains(text(), '{}')]", escaped);
        let search = self
            .send(
                "DOM.performSearch",
                Some(serde_json::json!({"query": xpath})),
            )
            .await?;
        let search_id = search["searchId"]
            .as_str()
            .ok_or("DOM.performSearch returned no searchId")?;
        let count = search["resultCount"].as_i64().unwrap_or(0);
        if count == 0 {
            let _ = self
                .send(
                    "DOM.discardSearchResults",
                    Some(serde_json::json!({"searchId": search_id})),
                )
                .await;
            return Err(format!("text not found: {}", text).into());
        }
        let results = self
            .send(
                "DOM.getSearchResults",
                Some(serde_json::json!({
                    "searchId": search_id,
                    "fromIndex": 0,
                    "toIndex": count.min(10)
                })),
            )
            .await?;
        let _ = self
            .send(
                "DOM.discardSearchResults",
                Some(serde_json::json!({"searchId": search_id})),
            )
            .await;
        let node_ids = results["nodeIds"]
            .as_array()
            .ok_or("missing nodeIds from search")?;
        // Try each matching node — pick the first one with a visible box
        for node_val in node_ids {
            let node_id = node_val.as_i64().unwrap_or(0);
            if node_id == 0 {
                continue;
            }
            let _ = self
                .send(
                    "DOM.scrollIntoViewIfNeeded",
                    Some(serde_json::json!({"nodeId": node_id})),
                )
                .await;
            // Use .ok() to drop Box<dyn Error> before await (Send requirement)
            let center = self.get_box_center(node_id).await.ok();
            if let Some((x, y)) = center {
                return self.click(x, y).await;
            }
        }
        Err(format!("text found but no visible element: {}", text).into())
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

    /// Upload files to a file input element — pure CDP via DOM.querySelector
    pub async fn upload_files(
        &self,
        selector: &str,
        paths: &[&str],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let node_id = self.resolve_selector(selector).await?;
        // DOM.describeNode with nodeId to get backendNodeId for setFileInputFiles
        let desc = self
            .send(
                "DOM.describeNode",
                Some(serde_json::json!({"nodeId": node_id})),
            )
            .await?;
        let backend_node_id = desc["node"]["backendNodeId"]
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

    /// Wait for a CSS selector to appear in the DOM — pure CDP polling via DOM.querySelector.
    /// Take a screenshot and save to a file.
    pub async fn screenshot(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let result = self
            .send(
                "Page.captureScreenshot",
                Some(serde_json::json!({"format": "png"})),
            )
            .await?;
        let data = result["data"].as_str().ok_or("missing screenshot data")?;
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data)?;
        std::fs::write(path, bytes)?;
        Ok(())
    }

    /// Take a full-page screenshot (captures content beyond viewport).
    pub async fn screenshot_full(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let result = self
            .send(
                "Page.captureScreenshot",
                Some(serde_json::json!({"format": "png", "captureBeyondViewport": true})),
            )
            .await?;
        let data = result["data"].as_str().ok_or("missing screenshot data")?;
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data)?;
        std::fs::write(path, bytes)?;
        Ok(())
    }

    // ================================================================
    // SEE — Agent's Eyes (Perception)
    // ================================================================

    /// Get the full accessibility tree — the semantic structure of the page.
    /// This is the primary perception tool: 10k DOM nodes compress to ~500 AX nodes.
    pub async fn get_ax_tree(
        &self,
        depth: Option<i32>,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        self.send("Accessibility.enable", None).await?;
        let mut params = serde_json::json!({});
        if let Some(d) = depth {
            params["depth"] = serde_json::json!(d);
        }
        let result = self
            .send("Accessibility.getFullAXTree", Some(params))
            .await?;
        Ok(result["nodes"].clone())
    }

    /// Get a simplified DOM tree — pruned to semantic elements with key attributes.
    pub async fn get_dom_tree(
        &self,
        selector: Option<&str>,
        depth: i32,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        let root = selector.unwrap_or("body");
        let js = format!(
            r#"(() => {{
            function walk(el, d, max) {{
                if (d > max) return null;
                const tag = el.tagName?.toLowerCase();
                if (!tag || ['script','style','noscript','svg','path','link','meta'].includes(tag)) return null;
                const n = {{ tag }};
                if (el.id) n.id = el.id;
                const cls = el.className;
                if (cls && typeof cls === 'string') {{ const c = cls.trim(); if (c) n.class = c.split(/\s+/).slice(0,3).join(' '); }}
                const role = el.getAttribute('role'); if (role) n.role = role;
                const type = el.getAttribute('type'); if (type) n.type = type;
                const href = el.getAttribute('href'); if (href) n.href = href;
                const ph = el.getAttribute('placeholder'); if (ph) n.placeholder = ph;
                const al = el.getAttribute('aria-label'); if (al) n.ariaLabel = al;
                const nm = el.getAttribute('name'); if (nm) n.name = nm;
                if (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT') n.value = el.value || '';
                const txt = Array.from(el.childNodes).filter(c => c.nodeType === 3).map(c => c.textContent.trim()).filter(t => t).join(' ');
                if (txt && txt.length < 200) n.text = txt;
                const r = el.getBoundingClientRect();
                if (r.width > 0 && r.height > 0) {{ n.box = [Math.round(r.x), Math.round(r.y), Math.round(r.width), Math.round(r.height)]; }}
                const ch = Array.from(el.children).map(c => walk(c, d+1, max)).filter(c => c !== null);
                if (ch.length > 0) n.children = ch;
                return n;
            }}
            const root = document.querySelector({r});
            if (!root) throw new Error('selector not found: ' + {r});
            return walk(root, 0, {depth});
        }})()"#,
            r = js_str(root),
            depth = depth
        );
        self.evaluate(&js).await
    }

    /// Get current page info: URL, title, viewport, scroll position, readyState.
    pub async fn get_page_info(&self) -> Result<Value, Box<dyn std::error::Error>> {
        let layout = self.send("Page.getLayoutMetrics", None).await?;
        let info = self
            .evaluate(
                r#"({
            url: location.href,
            title: document.title,
            readyState: document.readyState,
            scrollX: window.scrollX,
            scrollY: window.scrollY,
            innerWidth: window.innerWidth,
            innerHeight: window.innerHeight
        })"#,
            )
            .await?;
        let mut result = info;
        if let Some(content) = layout.get("cssContentSize") {
            result["contentWidth"] = content["width"].clone();
            result["contentHeight"] = content["height"].clone();
        }
        Ok(result)
    }

    // ================================================================
    // PROBE — Agent's Instruments (Discovery)
    // ================================================================

    /// Find elements by visible text and optional role filter.
    /// Returns list with tag, role, text, selector, coordinates, dimensions.
    pub async fn find_elements(
        &self,
        text: &str,
        role: Option<&str>,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        let role_filter = role.unwrap_or("");
        let q = js_str(text);
        let rf = js_str(role_filter);
        let js = format!(
            r#"(() => {{
            const query = {q};
            const roleFilter = {rf};
            const results = [];
            const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_ELEMENT);
            while (walker.nextNode()) {{
                const el = walker.currentNode;
                const rect = el.getBoundingClientRect();
                if (rect.width === 0 || rect.height === 0) continue;
                if (el.offsetParent === null && getComputedStyle(el).position !== 'fixed') continue;
                const directText = Array.from(el.childNodes).filter(n => n.nodeType === 3).map(n => n.textContent.trim()).join(' ');
                const ariaLabel = el.getAttribute('aria-label') || '';
                const placeholder = el.getAttribute('placeholder') || '';
                const matchText = directText || ariaLabel || placeholder;
                if (!matchText.includes(query)) continue;
                const tag = el.tagName.toLowerCase();
                const role = el.getAttribute('role') || tag;
                if (roleFilter && role !== roleFilter && tag !== roleFilter) continue;
                let sel = tag;
                if (el.id) sel = '#' + el.id;
                else if (el.className && typeof el.className === 'string') {{
                    const cls = el.className.trim().split(/\s+/)[0];
                    if (cls) sel = tag + '.' + CSS.escape(cls);
                }}
                results.push({{
                    tag, role, text: matchText.slice(0, 100),
                    ariaLabel: ariaLabel || undefined,
                    selector: sel,
                    x: Math.round(rect.x + rect.width/2),
                    y: Math.round(rect.y + rect.height/2),
                    width: Math.round(rect.width),
                    height: Math.round(rect.height)
                }});
            }}
            return results;
        }})()"#,
        );
        self.evaluate(&js).await
    }

    /// Deep probe of a single element: tag, attributes, box model, visibility, text.
    pub async fn get_element_info(
        &self,
        selector: &str,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        let s = js_str(selector);
        let js = format!(
            r#"(() => {{
            const el = document.querySelector({s});
            if (!el) throw new Error('element not found: ' + {s});
            const rect = el.getBoundingClientRect();
            const cs = getComputedStyle(el);
            const attrs = {{}};
            for (const a of el.attributes) attrs[a.name] = a.value;
            return {{
                tag: el.tagName.toLowerCase(),
                attrs,
                text: (el.textContent || '').trim().slice(0, 500),
                innerText: (el.innerText || '').trim().slice(0, 500),
                value: el.value || undefined,
                box: {{ x: rect.x, y: rect.y, width: rect.width, height: rect.height }},
                visible: rect.width > 0 && rect.height > 0 && cs.visibility !== 'hidden' && cs.display !== 'none' && cs.opacity !== '0',
                editable: el.isContentEditable || el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT',
                disabled: el.disabled || false,
                childCount: el.children.length,
                display: cs.display,
                position: cs.position,
                overflow: cs.overflow,
                zIndex: cs.zIndex
            }};
        }})()"#,
        );
        self.evaluate(&js).await
    }

    /// Get event listeners attached to an element — pure CDP via DOM.resolveNode
    pub async fn get_event_listeners(
        &self,
        selector: &str,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        let node_id = self.resolve_selector(selector).await?;
        // Resolve DOM nodeId to a Runtime RemoteObject to get objectId
        let resolved = self
            .send(
                "DOM.resolveNode",
                Some(serde_json::json!({"nodeId": node_id})),
            )
            .await?;
        let object_id = resolved["object"]["objectId"]
            .as_str()
            .ok_or(format!("failed to resolve node to object: {}", selector))?;

        let listeners = self
            .send(
                "DOMDebugger.getEventListeners",
                Some(serde_json::json!({ "objectId": object_id })),
            )
            .await?;
        Ok(listeners["listeners"].clone())
    }

    /// Get cookies for the current page.
    pub async fn get_cookies(&self) -> Result<Value, Box<dyn std::error::Error>> {
        let result = self.send("Network.getCookies", None).await?;
        Ok(result["cookies"].clone())
    }

    /// Hit-test: what element is at pixel (x, y)?
    pub async fn hit_test(&self, x: f64, y: f64) -> Result<Value, Box<dyn std::error::Error>> {
        let js = format!(
            r#"(() => {{
            const el = document.elementFromPoint({}, {});
            if (!el) return null;
            const rect = el.getBoundingClientRect();
            return {{
                tag: el.tagName.toLowerCase(),
                id: el.id || undefined,
                class: (typeof el.className === 'string' ? el.className : '') || undefined,
                role: el.getAttribute('role') || undefined,
                text: (el.textContent || '').trim().slice(0, 100),
                ariaLabel: el.getAttribute('aria-label') || undefined,
                box: {{ x: rect.x, y: rect.y, width: rect.width, height: rect.height }}
            }};
        }})()"#,
            x, y
        );
        self.evaluate(&js).await
    }

    /// Get top-layer elements (modals, dialogs, popovers).
    pub async fn get_top_layer(&self) -> Result<Value, Box<dyn std::error::Error>> {
        // DOM.getTopLayerElements requires DOM.enable
        let _ = self.send("DOM.enable", None).await;
        match self.send("DOM.getTopLayerElements", None).await {
            Ok(result) => Ok(result["nodeIds"].clone()),
            Err(_) => {
                // Fallback: use JS to find dialog/modal elements
                self.evaluate(
                    r#"(() => {
                    const modals = [];
                    document.querySelectorAll('dialog[open], [role="dialog"], [role="alertdialog"], [aria-modal="true"]').forEach(el => {
                        const r = el.getBoundingClientRect();
                        modals.push({
                            tag: el.tagName.toLowerCase(),
                            id: el.id || undefined,
                            role: el.getAttribute('role') || undefined,
                            text: (el.textContent || '').trim().slice(0, 200),
                            box: { x: r.x, y: r.y, width: r.width, height: r.height }
                        });
                    });
                    return modals;
                })()"#,
                )
                .await
            }
        }
    }

    /// Force pseudo-state (:hover, :focus, :active) on an element — pure CDP
    pub async fn force_pseudo_state(
        &self,
        selector: &str,
        states: &[&str],
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.send("CSS.enable", None).await?;
        let node_id = self.resolve_selector(selector).await?;
        self.send(
            "CSS.forcePseudoState",
            Some(serde_json::json!({
                "nodeId": node_id,
                "forcedPseudoClasses": states
            })),
        )
        .await?;
        Ok(())
    }

    /// Start network capture via CDP Network domain — pure protocol, no JS injection.
    /// Captures all fetch/XHR/navigation requests at the protocol level.
    pub async fn start_network_log(&self) -> Result<(), Box<dyn std::error::Error>> {
        *self.network_capture_active.lock().await = true;
        self.send("Network.enable", None).await?;
        Ok(())
    }

    /// Stop network capture.
    /// Returns full request/response metadata (URL, method, status, headers, mime type).
    pub async fn get_network_log(&self) -> Result<Value, Box<dyn std::error::Error>> {
        let entries: Vec<NetworkEntry> = {
            let mut log = self.network_log.lock().await;
            log.drain(..).collect()
        };
        Ok(serde_json::to_value(&entries)?)
    }

    /// Get captured network log with response bodies for API responses (JSON/text).
    /// Fetches body via Network.getResponseBody for each entry matching the filter.
    pub async fn get_network_log_with_bodies(
        &self,
        url_filter: Option<&str>,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        let entries: Vec<NetworkEntry> = {
            let log = self.network_log.lock().await;
            log.clone()
        };

        let mut enriched = Vec::new();
        for mut entry in entries {
            // Filter by URL pattern if provided
            if let Some(filter) = url_filter {
                if !entry.url.contains(filter) {
                    continue;
                }
            }
            // Skip non-API responses (images, fonts, CSS, etc.)
            let dominated_by_api = entry.mime_type.contains("json")
                || entry.mime_type.contains("text")
                || entry.mime_type.contains("javascript");
            if !dominated_by_api {
                continue;
            }
            // Fetch response body via CDP
            if let Ok(body_result) = self
                .send(
                    "Network.getResponseBody",
                    Some(serde_json::json!({"requestId": entry.request_id})),
                )
                .await
            {
                let body_str = body_result["body"].as_str().unwrap_or("");
                // Try to parse as JSON for structured data
                if let Ok(json_body) = serde_json::from_str::<Value>(body_str) {
                    entry.body = Some(json_body);
                } else {
                    entry.body = Some(Value::String(body_str.chars().take(2000).collect()));
                }
            }
            enriched.push(entry);
        }

        Ok(serde_json::to_value(&enriched)?)
    }

    // ================================================================
    // FORGE — Agent's Scalpels (Deep Inspection)
    // ================================================================

    /// Get all API calls recorded by the injected fetch/XHR hooks.
    /// Returns array of {url, method, status, request_body, response_body, timestamp}.
    pub async fn get_api_log(&self) -> Result<Value, Box<dyn std::error::Error>> {
        self.evaluate("window.__CLAW_API_LOG__ || []").await
    }

    /// Clear the API call log.
    pub async fn clear_api_log(&self) -> Result<Value, Box<dyn std::error::Error>> {
        self.evaluate("(window.__CLAW_API_LOG__ = [], 'cleared')")
            .await
    }

    /// List all global variable names on the page.
    /// Discovers __INITIAL_STATE__, __NEXT_DATA__, __NUXT__, etc.
    pub async fn get_global_names(&self) -> Result<Value, Box<dyn std::error::Error>> {
        // Runtime.globalLexicalScopeNames only returns let/const at top level.
        // For window properties (which is what we need), use evaluate.
        self.evaluate(
            r#"(() => {
                const builtins = new Set(['location','chrome','onerror','onmessage','crypto','caches','cookieStore','onbeforeinput']);
                const interesting = Object.keys(window).filter(k => {
                    if (builtins.has(k)) return false;
                    if (k.startsWith('on')) return false;
                    if (k.startsWith('webkit')) return false;
                    const v = window[k];
                    if (typeof v === 'function') return false;
                    if (v === window || v === document || v === navigator) return false;
                    return true;
                });
                // Also check common SSR/framework globals
                const ssrKeys = ['__INITIAL_STATE__','__NEXT_DATA__','__NUXT__','__REMIX_CONTEXT__','__APP_DATA__','__VUE__','__REACT_DEVTOOLS_GLOBAL_HOOK__','__pinia','__VUEX__'];
                const found = {};
                for (const k of ssrKeys) {
                    if (window[k] !== undefined) {
                        const v = window[k];
                        const t = typeof v;
                        found[k] = t === 'object' ? { type: t, keys: Object.keys(v).slice(0, 20), size: JSON.stringify(v).length } : { type: t };
                    }
                }
                return { globals: interesting.slice(0, 100), framework_globals: found };
            })()"#,
        )
        .await
    }

    /// List all resources (scripts, stylesheets, images) loaded by the page.
    pub async fn get_resource_tree(&self) -> Result<Value, Box<dyn std::error::Error>> {
        let result = self.send("Page.getResourceTree", None).await?;
        // Extract just the resources array, not the full frame tree
        let resources = result
            .get("frameTree")
            .and_then(|ft| ft.get("resources"))
            .cloned()
            .unwrap_or(Value::Array(vec![]));
        Ok(resources)
    }

    /// Get the source content of a loaded resource (script, stylesheet, etc).
    pub async fn get_resource_content(
        &self,
        url: &str,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        // Need frame ID from resource tree
        let tree = self.send("Page.getResourceTree", None).await?;
        let frame_id = tree
            .get("frameTree")
            .and_then(|ft| ft.get("frame"))
            .and_then(|f| f.get("id"))
            .and_then(|id| id.as_str())
            .ok_or("could not determine frame ID")?
            .to_string();

        let result = self
            .send(
                "Page.getResourceContent",
                Some(serde_json::json!({
                    "frameId": frame_id,
                    "url": url
                })),
            )
            .await?;
        let content = result.get("content").cloned().unwrap_or(Value::Null);
        let base64_encoded = result
            .get("base64Encoded")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        Ok(serde_json::json!({
            "content": content,
            "base64_encoded": base64_encoded
        }))
    }

    /// Search within all loaded JavaScript resources for a pattern.
    /// Returns matching lines from all scripts.
    pub async fn search_resources(&self, query: &str) -> Result<Value, Box<dyn std::error::Error>> {
        // Get all JS resources
        let tree = self.send("Page.getResourceTree", None).await?;
        let frame_id = tree
            .get("frameTree")
            .and_then(|ft| ft.get("frame"))
            .and_then(|f| f.get("id"))
            .and_then(|id| id.as_str())
            .ok_or("could not determine frame ID")?
            .to_string();

        let resources = tree
            .get("frameTree")
            .and_then(|ft| ft.get("resources"))
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();

        let mut matches = Vec::new();
        for resource in &resources {
            let rtype = resource["type"].as_str().unwrap_or("");
            if rtype != "Script" && rtype != "Document" {
                continue;
            }
            let url = resource["url"].as_str().unwrap_or("");
            if url.is_empty() {
                continue;
            }

            // Search in this resource
            let search_result = self
                .send(
                    "Page.searchInResource",
                    Some(serde_json::json!({
                        "frameId": frame_id,
                        "url": url,
                        "query": query
                    })),
                )
                .await;
            if let Ok(result) = search_result {
                if let Some(search_matches) = result.get("result").and_then(|r| r.as_array()) {
                    if !search_matches.is_empty() {
                        matches.push(serde_json::json!({
                            "url": url,
                            "matches": search_matches.iter().take(10).collect::<Vec<_>>()
                        }));
                    }
                }
            }
        }
        Ok(Value::Array(matches))
    }

    /// Read localStorage or sessionStorage.
    pub async fn get_storage_items(
        &self,
        storage_type: &str,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        let storage = if storage_type == "session" {
            "sessionStorage"
        } else {
            "localStorage"
        };
        self.evaluate(&format!(
            r#"(() => {{
                const s = {};
                const items = {{}};
                for (let i = 0; i < s.length; i++) {{
                    const k = s.key(i);
                    items[k] = s.getItem(k).slice(0, 500);
                }}
                return items;
            }})()"#,
            storage
        ))
        .await
    }

    /// Replay a request within the page context (using page's cookies/session).
    pub async fn request_replay(
        &self,
        url: &str,
        method: &str,
        headers: Option<&Value>,
        body: Option<&str>,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        let headers_js = headers
            .map(|h| serde_json::to_string(h).unwrap_or_default())
            .unwrap_or_else(|| "{}".to_string());
        let body_js = match body {
            Some(b) => format!("{}", crate::cdp::js_str(b)),
            None => "null".to_string(),
        };
        self.evaluate(&format!(
            r#"(async () => {{
                const resp = await fetch({url}, {{
                    method: {method},
                    credentials: 'include',
                    headers: {headers},
                    body: {body}
                }});
                const ct = resp.headers.get('content-type') || '';
                const status = resp.status;
                const respHeaders = Object.fromEntries(resp.headers.entries());
                let data;
                if (ct.includes('json')) {{
                    data = await resp.json();
                }} else {{
                    const t = await resp.text();
                    data = t.slice(0, 5000);
                }}
                return {{ status, headers: respHeaders, data }};
            }})()"#,
            url = crate::cdp::js_str(url),
            method = crate::cdp::js_str(method),
            headers = headers_js,
            body = body_js,
        ))
        .await
    }

    // ================================================================
    // INTERCEPT — Fetch Domain (Active Request Interception)
    // ================================================================

    /// Start intercepting requests matching a URL pattern.
    /// Intercepted requests are paused — use get_paused_requests() to see them,
    /// then fetch_continue/fetch_fulfill/fetch_fail to handle them.
    pub async fn fetch_enable(&self, url_pattern: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.send(
            "Fetch.enable",
            Some(serde_json::json!({
                "patterns": [{"urlPattern": url_pattern}]
            })),
        )
        .await?;
        Ok(())
    }

    /// Stop intercepting requests.
    pub async fn fetch_disable(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.send("Fetch.disable", None).await?;
        // Drain buffer
        self.paused_fetch.lock().await.clear();
        Ok(())
    }

    /// Get all paused requests (from Fetch.requestPaused events) and clear the buffer.
    pub async fn get_paused_requests(&self) -> Result<Value, Box<dyn std::error::Error>> {
        let requests: Vec<Value> = self.paused_fetch.lock().await.drain(..).collect();
        // Extract useful fields for each paused request
        let simplified: Vec<Value> = requests
            .iter()
            .map(|r| {
                serde_json::json!({
                    "requestId": r["requestId"],
                    "url": r["request"]["url"],
                    "method": r["request"]["method"],
                    "headers": r["request"]["headers"],
                    "postData": r["request"]["postData"],
                    "resourceType": r["resourceType"],
                })
            })
            .collect();
        Ok(Value::Array(simplified))
    }

    /// Continue a paused request, optionally modifying headers or POST body.
    pub async fn fetch_continue(
        &self,
        request_id: &str,
        url: Option<&str>,
        headers: Option<&Value>,
        post_data: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut params = serde_json::json!({"requestId": request_id});
        if let Some(u) = url {
            params["url"] = Value::String(u.to_string());
        }
        if let Some(h) = headers {
            params["headers"] = h.clone();
        }
        if let Some(b) = post_data {
            params["postData"] = Value::String(b.to_string());
        }
        self.send("Fetch.continueRequest", Some(params)).await?;
        Ok(())
    }

    /// Fulfill a paused request with a custom response (bypass the server entirely).
    pub async fn fetch_fulfill(
        &self,
        request_id: &str,
        response_code: u16,
        body: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.send(
            "Fetch.fulfillRequest",
            Some(serde_json::json!({
                "requestId": request_id,
                "responseCode": response_code,
                "body": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, body.as_bytes()),
                "responseHeaders": [{"name": "content-type", "value": "application/json"}]
            })),
        )
        .await?;
        Ok(())
    }

    /// Block a paused request.
    pub async fn fetch_fail(&self, request_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.send(
            "Fetch.failRequest",
            Some(serde_json::json!({
                "requestId": request_id,
                "errorReason": "Aborted"
            })),
        )
        .await?;
        Ok(())
    }

    // ================================================================
    // COOKIES — Precise Auth Control
    // ================================================================

    /// Set a cookie on a domain.
    pub async fn set_cookie(
        &self,
        name: &str,
        value: &str,
        domain: &str,
        path: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.send(
            "Network.setCookie",
            Some(serde_json::json!({
                "name": name,
                "value": value,
                "domain": domain,
                "path": path.unwrap_or("/")
            })),
        )
        .await?;
        Ok(())
    }

    /// Delete cookies matching name and domain.

    /// Hover at exact coordinates (triggers CSS :hover, tooltips, dropdowns).
    pub async fn hover_at(&self, x: f64, y: f64) -> Result<(), Box<dyn std::error::Error>> {
        self.send(
            "Input.dispatchMouseEvent",
            Some(serde_json::json!({
                "type": "mouseMoved",
                "x": x,
                "y": y
            })),
        )
        .await?;
        Ok(())
    }

    /// Hover over element matching CSS selector — pure CDP
    pub async fn hover_selector(&self, selector: &str) -> Result<(), Box<dyn std::error::Error>> {
        let (x, y) = self.resolve_selector_center(selector).await?;
        self.hover_at(x, y).await
    }

    /// Scroll an element into view — pure CDP via DOM.scrollIntoViewIfNeeded
    pub async fn scroll_into_view(&self, selector: &str) -> Result<(), Box<dyn std::error::Error>> {
        let node_id = self.resolve_selector(selector).await?;
        self.send(
            "DOM.scrollIntoViewIfNeeded",
            Some(serde_json::json!({"nodeId": node_id})),
        )
        .await?;
        Ok(())
    }

    /// Scroll by a delta amount (pixels).
    /// Mouse position is dynamically set to viewport center for reliable scrolling at any resolution.
    /// Modifiers bitmask: Alt=1, Ctrl=2, Meta=4, Shift=8.
    pub async fn press_key(
        &self,
        key: &str,
        modifiers: u32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.send(
            "Input.dispatchKeyEvent",
            Some(key_event_params("keyDown", key, None, modifiers)),
        )
        .await?;
        self.send(
            "Input.dispatchKeyEvent",
            Some(key_event_params("keyUp", key, None, 0)),
        )
        .await?;
        Ok(())
    }

    /// Select an option in a <select> dropdown by value.
    pub async fn select_option(
        &self,
        selector: &str,
        value: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let s = js_str(selector);
        let v = js_str(value);
        let js = format!(
            r#"(() => {{
                const sel = document.querySelector({s});
                if (!sel) throw new Error('select not found: ' + {s});
                sel.value = {v};
                sel.dispatchEvent(new Event('change', {{ bubbles: true }}));
                sel.dispatchEvent(new Event('input', {{ bubbles: true }}));
                return sel.value;
            }})()"#,
        );
        self.evaluate(&js).await?;
        Ok(())
    }

    /// Handle a JavaScript dialog (alert, confirm, prompt, beforeunload).
    pub async fn dismiss_dialog(
        &self,
        accept: bool,
        prompt_text: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut params = serde_json::json!({ "accept": accept });
        if let Some(text) = prompt_text {
            params["promptText"] = serde_json::json!(text);
        }
        self.send("Page.handleJavaScriptDialog", Some(params))
            .await?;
        Ok(())
    }

    // ================================================================
    // VERIFY — Agent's Measuring Tape
    // ================================================================

    /// Wait for visible text to appear on the page.
    /// Wait for URL to match a pattern (substring match).
    /// Wait for network to become idle (no pending fetch/XHR for duration).
    #[allow(dead_code)]

    /// Combines screenshot + interactive elements + forms + auth state in a single call.
    #[allow(dead_code)]

    /// This carries cookies, referer, and auth headers — unlike raw HTTP.
    pub fn download_js(url: &str) -> String {
        format!(
            r#"(async () => {{
                const resp = await fetch({url}, {{ credentials: 'include' }});
                if (!resp.ok) throw new Error('HTTP ' + resp.status);
                const blob = await resp.blob();
                return await new Promise((resolve, reject) => {{
                    const reader = new FileReader();
                    reader.onloadend = () => resolve(JSON.stringify({{
                        base64: reader.result.split(',')[1],
                        mime: blob.type,
                        size: blob.size
                    }}));
                    reader.onerror = reject;
                    reader.readAsDataURL(blob);
                }});
            }})()"#,
            url = js_str(url)
        )
    }

    /// Download a URL using the browser's session (cookies/referer) and save to a local file.
    pub async fn download_via_browser(
        &self,
        url: &str,
        output: &str,
    ) -> Result<usize, Box<dyn std::error::Error>> {
        let js = Self::download_js(url);
        let result = self.evaluate(&js).await?;
        let json_str = result.as_str().ok_or("expected JSON string from fetch")?;
        let parsed: Value = serde_json::from_str(json_str)?;
        let base64_data = parsed["base64"]
            .as_str()
            .ok_or("missing base64 field in response")?;
        let bytes = base64_decode(base64_data)?;
        let size = bytes.len();
        std::fs::write(output, &bytes)?;
        Ok(size)
    }

    /// Generate JS that finds an image by CSS selector, resolves its src, and fetches it as base64.
    pub fn save_image_js(selector: &str) -> String {
        format!(
            r#"(async () => {{
                const el = document.querySelector({sel});
                if (!el) throw new Error('element not found: ' + {sel});
                const url = el.currentSrc || el.src || el.getAttribute('src') || el.style.backgroundImage?.replace(/url\(["']?|["']?\)/g, '');
                if (!url) throw new Error('no image source on element');
                const resp = await fetch(url, {{ credentials: 'include' }});
                if (!resp.ok) throw new Error('HTTP ' + resp.status);
                const blob = await resp.blob();
                return await new Promise((resolve, reject) => {{
                    const reader = new FileReader();
                    reader.onloadend = () => resolve(JSON.stringify({{
                        base64: reader.result.split(',')[1],
                        mime: blob.type,
                        size: blob.size,
                        url: url
                    }}));
                    reader.onerror = reject;
                    reader.readAsDataURL(blob);
                }});
            }})()"#,
            sel = js_str(selector)
        )
    }

    /// Download an image from the current page by CSS selector and save to a local file.
    pub async fn save_image(
        &self,
        selector: &str,
        output: &str,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        let js = Self::save_image_js(selector);
        let result = self.evaluate(&js).await?;
        let json_str = result.as_str().ok_or("expected JSON string from fetch")?;
        let parsed: Value = serde_json::from_str(json_str)?;
        let base64_data = parsed["base64"]
            .as_str()
            .ok_or("missing base64 field in response")?;
        let bytes = base64_decode(base64_data)?;
        let size = bytes.len();
        std::fs::write(output, &bytes)?;
        Ok(serde_json::json!({
            "path": output,
            "size": size,
            "mime": parsed["mime"],
            "url": parsed["url"]
        }))
    }
}

/// Decode base64 string to bytes.
fn base64_decode(input: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // Simple base64 decode without external dependency
    let mut result = Vec::new();
    let lut: Vec<u8> = {
        let mut table = vec![255u8; 256];
        for (i, &c) in b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
            .iter()
            .enumerate()
        {
            table[c as usize] = i as u8;
        }
        table
    };
    let clean: Vec<u8> = input
        .bytes()
        .filter(|&b| b != b'\n' && b != b'\r' && b != b'=')
        .collect();
    let chunks = clean.chunks(4);
    for chunk in chunks {
        let mut buf = [0u8; 4];
        for (i, &b) in chunk.iter().enumerate() {
            buf[i] = lut[b as usize];
            if buf[i] == 255 {
                return Err(format!("invalid base64 character: {}", b as char).into());
            }
        }
        let n = chunk.len();
        if n >= 2 {
            result.push((buf[0] << 2) | (buf[1] >> 4));
        }
        if n >= 3 {
            result.push((buf[1] << 4) | (buf[2] >> 2));
        }
        if n >= 4 {
            result.push((buf[2] << 6) | buf[3]);
        }
    }
    Ok(result)
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
    fn js_str_produces_valid_js_string_literals() {
        // Simple string
        assert_eq!(js_str("foo"), r#""foo""#);
        // Single quotes pass through (no escaping needed in double-quoted JS)
        assert_eq!(js_str("it's"), r#""it's""#);
        // Double quotes are escaped
        assert_eq!(js_str(r#"say "hi""#), r#""say \"hi\"""#);
        // Backslash is escaped
        assert_eq!(js_str("back\\slash"), r#""back\\slash""#);
        // Newline is escaped
        assert_eq!(js_str("line\nbreak"), r#""line\nbreak""#);
        // Empty string
        assert_eq!(js_str(""), r#""""#);
        // Unicode passes through JSON serialization correctly
        assert_eq!(js_str("hello"), r#""hello""#);
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


    // --- download_via_browser: uses page JS fetch (carries cookies/referer) ---
    // Why: raw reqwest bypasses browser session — auth-gated images, anti-hotlink fail

    #[test]
    fn download_js_snippet_uses_fetch_and_returns_base64() {
        // The JS snippet must use fetch() (browser context) not XMLHttpRequest
        // and must return base64-encoded data for binary content
        let js = CdpClient::download_js("https://example.com/img.png");
        assert!(
            js.contains("fetch("),
            "must use fetch() for browser session"
        );
        assert!(
            js.contains("base64") || js.contains("btoa") || js.contains("FileReader"),
            "must encode binary as base64 for transport"
        );
        assert!(
            js.contains("https://example.com/img.png"),
            "must contain the target URL"
        );
    }

    #[test]
    fn base64_decode_roundtrip() {
        // Why: download pipeline depends on correct base64 decode of browser fetch response
        let decoded = super::base64_decode("SGVsbG8gV29ybGQ=").unwrap();
        assert_eq!(decoded, b"Hello World");
    }

    #[test]
    fn base64_decode_binary_data() {
        // PNG magic bytes: 0x89 0x50 0x4E 0x47
        let decoded = super::base64_decode("iVBORw==").unwrap();
        assert_eq!(decoded[0], 0x89);
        assert_eq!(decoded[1], 0x50);
        assert_eq!(decoded[2], 0x4E);
        assert_eq!(decoded[3], 0x47);
    }

    #[test]
    fn save_image_js_snippet_resolves_selector_src() {
        // save_image must: find element by selector → get its src/currentSrc → fetch it
        let js = CdpClient::save_image_js("img.hero");
        assert!(
            js.contains("querySelector"),
            "must use querySelector to find element"
        );
        assert!(js.contains("img.hero"), "must contain the CSS selector");
        assert!(
            js.contains("currentSrc") || js.contains(".src"),
            "must resolve the image source URL"
        );
        assert!(
            js.contains("fetch("),
            "must fetch the resolved URL via browser context"
        );
    }
}
