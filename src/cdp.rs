use std::collections::HashMap;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

/// Stealth JavaScript injected via Page.addScriptToEvaluateOnNewDocument.
/// Patches headless Chrome detection vectors before any page JS runs.
const STEALTH_JS: &str = r#"
// 1. navigator.webdriver → undefined
Object.defineProperty(navigator, 'webdriver', {
    get: () => undefined,
});

// 2. window.chrome runtime stub
if (!window.chrome) {
    window.chrome = {};
}
if (!window.chrome.runtime) {
    window.chrome.runtime = {};
}

// 3. navigator.plugins — headless has empty plugins
Object.defineProperty(navigator, 'plugins', {
    get: () => {
        const plugins = [
            { name: 'Chrome PDF Plugin', filename: 'internal-pdf-viewer', description: 'Portable Document Format', length: 1 },
            { name: 'Chrome PDF Viewer', filename: 'mhjfbmdgcfjbbpaeojofohoefgiehjai', description: '', length: 1 },
            { name: 'Native Client', filename: 'internal-nacl-plugin', description: '', length: 2 },
        ];
        plugins.forEach((p, i) => { plugins[i] = Object.assign(Object.create(Plugin.prototype), p); });
        const list = Object.create(PluginArray.prototype);
        plugins.forEach((p, i) => { list[i] = p; });
        Object.defineProperty(list, 'length', { get: () => plugins.length });
        return list;
    },
});

// 4. navigator.languages
Object.defineProperty(navigator, 'languages', {
    get: () => ['en-US', 'en'],
});

// 5. Permissions.query — headless returns "denied" for notifications
if (navigator.permissions) {
    const originalQuery = navigator.permissions.query.bind(navigator.permissions);
    navigator.permissions.query = (params) => {
        if (params.name === 'notifications') {
            return Promise.resolve({ state: Notification.permission });
        }
        return originalQuery(params);
    };
}

// 6. WebGL renderer — headless returns "Google SwiftShader"
(function() {
    const getParameter = WebGLRenderingContext.prototype.getParameter;
    WebGLRenderingContext.prototype.getParameter = function(param) {
        if (param === 37445) { return 'Intel Inc.'; }
        if (param === 37446) { return 'Intel Iris OpenGL Engine'; }
        return getParameter.call(this, param);
    };
    if (typeof WebGL2RenderingContext !== 'undefined') {
        const getParameter2 = WebGL2RenderingContext.prototype.getParameter;
        WebGL2RenderingContext.prototype.getParameter = function(param) {
            if (param === 37445) { return 'Intel Inc.'; }
            if (param === 37446) { return 'Intel Iris OpenGL Engine'; }
            return getParameter2.call(this, param);
        };
    }
})();

// 7. window.outerWidth/outerHeight — 0 in headless
if (window.outerWidth === 0) {
    Object.defineProperty(window, 'outerWidth', { get: () => window.innerWidth });
}
if (window.outerHeight === 0) {
    Object.defineProperty(window, 'outerHeight', { get: () => window.innerHeight });
}
"#;

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

/// Type of interactive element detected during explore.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ElementType {
    Button,
    Link,
    Input,
    Textarea,
    Select,
    Checkbox,
    Radio,
    ContentEditable,
    Other,
}

/// A single interactive element found on the page.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InteractiveElement {
    pub tag: String,
    pub role: String,
    pub text: String,
    pub selector: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub element_type: ElementType,
}

/// A form field detected on the page.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FormField {
    pub selector: String,
    pub field_type: String,
    pub name: String,
    pub placeholder: String,
}

/// A form detected on the page.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FormInfo {
    pub selector: String,
    pub fields: Vec<FormField>,
    pub submit_selector: Option<String>,
}

/// Heuristic hints: auto-detected primary input, submit button, etc.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExploreHints {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_input: Option<InteractiveElement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub submit_button: Option<InteractiveElement>,
}

impl ExploreHints {
    pub fn from_elements(elements: &[InteractiveElement]) -> Self {
        // Primary input: largest textarea/input/contenteditable by area
        let primary_input = elements
            .iter()
            .filter(|e| {
                matches!(
                    e.element_type,
                    ElementType::Textarea | ElementType::Input | ElementType::ContentEditable
                )
            })
            .max_by_key(|e| e.width * e.height)
            .cloned();

        // Submit button: button closest to primary input
        let submit_button = if let Some(ref input) = primary_input {
            elements
                .iter()
                .filter(|e| e.element_type == ElementType::Button)
                .min_by_key(|b| {
                    let dx = (b.x - input.x) as i64;
                    let dy = (b.y - input.y) as i64;
                    dx * dx + dy * dy
                })
                .cloned()
        } else {
            None
        };

        ExploreHints {
            primary_input,
            submit_button,
        }
    }
}

/// Full page panorama returned by `explore` — everything an Agent needs to forge an adapter.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExploreResult {
    pub url: String,
    pub title: String,
    pub screenshot_path: String,
    pub logged_in: bool,
    pub interactive_elements: Vec<InteractiveElement>,
    pub forms: Vec<FormInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hints: Option<ExploreHints>,
}

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

/// Produce a valid JavaScript string literal (double-quoted, properly escaped).
/// Uses JSON serialization which handles all special characters correctly.
pub(crate) fn js_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| format!("\"{}\"", s))
}

/// Escape a string for safe embedding in JavaScript single-quoted strings
/// DEPRECATED: use js_str() instead — this breaks when input contains single quotes
pub(crate) fn escape_js_string(s: &str) -> String {
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
        let network_log: NetworkLog = Arc::new(Mutex::new(Vec::new()));
        let pending_requests: PendingRequests = Arc::new(Mutex::new(HashMap::new()));
        let network_capture_active: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
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

        // Reader task: route responses and network events
        let pending_clone = pending.clone();
        let net_log_clone = network_log.clone();
        let pending_req_clone = pending_requests.clone();
        let net_active_clone = network_capture_active.clone();
        tokio::spawn(async move {
            Self::read_loop(
                read,
                pending_clone,
                net_log_clone,
                pending_req_clone,
                net_active_clone,
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
        };
        client.ensure_stealth().await?;
        Ok(client)
    }

    /// Inject stealth patches before any page load.
    /// Uses Page.addScriptToEvaluateOnNewDocument — a CDP protocol command
    /// that runs JS before each page's main script context initializes.
    async fn ensure_stealth(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.send("Page.enable", None).await?;
        self.send(
            "Page.addScriptToEvaluateOnNewDocument",
            Some(serde_json::json!({ "source": STEALTH_JS })),
        )
        .await?;
        Ok(())
    }

    async fn read_loop(
        mut read: futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
        pending: PendingMap,
        network_log: NetworkLog,
        pending_requests: PendingRequests,
        network_capture_active: Arc<Mutex<bool>>,
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
                    let active = *network_capture_active.lock().await;
                    if active {
                        let params = resp.params.as_ref().cloned().unwrap_or(Value::Null);
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
    pub async fn wait_for_selector(
        &self,
        selector: &str,
        timeout_secs: f64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let max_attempts = (timeout_secs * 2.0) as u32; // 500ms per attempt
        for attempt in 0..max_attempts {
            match self.resolve_selector(selector).await {
                Ok(_) => return Ok(()),
                Err(e) => {
                    if attempt < 3 {
                        eprintln!("  wait_for poll {}: {}", attempt, e);
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        // Capture page state for diagnostics (this one keeps JS — it's diagnostic only)
        let diag = match self.evaluate("JSON.stringify({url: location.href, ready: document.readyState, title: document.title})").await {
            Ok(v) => v.as_str().unwrap_or("unknown").to_string(),
            Err(_) => "page unreachable".to_string(),
        };
        Err(format!(
            "timeout waiting for selector '{}' after {}s (page: {})",
            selector, timeout_secs, diag
        )
        .into())
    }

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
    pub async fn stop_network_log(&self) -> Result<(), Box<dyn std::error::Error>> {
        *self.network_capture_active.lock().await = false;
        self.send("Network.disable", None).await?;
        Ok(())
    }

    /// Get captured network log entries and clear the buffer.
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
    // TRY — Agent's Fingers (Actions)
    // ================================================================

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
    pub async fn scroll_by(
        &self,
        delta_x: f64,
        delta_y: f64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Get actual viewport size from CDP layout metrics
        let layout = self.send("Page.getLayoutMetrics", None).await?;
        let vw = layout["cssVisualViewport"]["clientWidth"]
            .as_f64()
            .unwrap_or(800.0);
        let vh = layout["cssVisualViewport"]["clientHeight"]
            .as_f64()
            .unwrap_or(600.0);
        self.send(
            "Input.dispatchMouseEvent",
            Some(serde_json::json!({
                "type": "mouseWheel",
                "x": vw / 2.0,
                "y": vh / 2.0,
                "deltaX": delta_x,
                "deltaY": delta_y
            })),
        )
        .await?;
        Ok(())
    }

    /// Press a specific key (Enter, Tab, Escape, ArrowDown, etc.) with optional modifiers.
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
    pub async fn wait_for_text(
        &self,
        text: &str,
        timeout_secs: f64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let js = format!("document.body.innerText.includes({})", js_str(text));
        let max_attempts = (timeout_secs * 4.0) as u32; // 250ms per attempt
        for _ in 0..max_attempts {
            if let Ok(val) = self.evaluate(&js).await {
                if val == true {
                    return Ok(());
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        Err(format!(
            "timeout waiting for text '{}' after {}s",
            text, timeout_secs
        )
        .into())
    }

    /// Wait for URL to match a pattern (substring match).
    pub async fn wait_for_url(
        &self,
        pattern: &str,
        timeout_secs: f64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let js = format!("location.href.includes({})", js_str(pattern));
        let max_attempts = (timeout_secs * 4.0) as u32;
        for _ in 0..max_attempts {
            if let Ok(val) = self.evaluate(&js).await {
                if val == true {
                    return Ok(());
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        Err(format!(
            "timeout waiting for URL pattern '{}' after {}s",
            pattern, timeout_secs
        )
        .into())
    }

    /// Wait for network to become idle (no pending fetch/XHR for duration).
    pub async fn wait_for_network_idle(
        &self,
        timeout_secs: f64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let js = r#"new Promise(resolve => {
            let timer = setTimeout(() => resolve(true), 500);
            const observer = new PerformanceObserver(() => {
                clearTimeout(timer);
                timer = setTimeout(() => resolve(true), 500);
            });
            observer.observe({ entryTypes: ['resource'] });
        })"#;
        let timeout = std::time::Duration::from_secs_f64(timeout_secs);
        match tokio::time::timeout(timeout, self.evaluate(js)).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => {
                Err(format!("timeout waiting for network idle after {}s", timeout_secs).into())
            }
        }
    }

    /// Wait for navigation to complete (URL changes from current).
    #[allow(dead_code)]
    pub async fn wait_for_navigation(
        &self,
        timeout_secs: f64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let current_url = self.evaluate("location.href").await?;
        let current = current_url.as_str().unwrap_or("").to_string();
        let max_attempts = (timeout_secs * 4.0) as u32;
        for _ in 0..max_attempts {
            if let Ok(val) = self.evaluate("location.href").await {
                if val.as_str().unwrap_or("") != current {
                    return Ok(());
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        Err(format!(
            "timeout waiting for navigation from '{}' after {}s",
            current, timeout_secs
        )
        .into())
    }

    /// Assert that a CSS selector exists in the DOM — pure CDP
    pub async fn assert_selector(&self, selector: &str) -> Result<(), Box<dyn std::error::Error>> {
        match self.resolve_selector(selector).await {
            Ok(_) => Ok(()),
            Err(_) => Err(format!("assertion failed: selector '{}' not found", selector).into()),
        }
    }

    /// Assert that visible text exists on the page.
    pub async fn assert_text(&self, text: &str) -> Result<(), Box<dyn std::error::Error>> {
        let js = format!("document.body.innerText.includes({})", js_str(text));
        let result = self.evaluate(&js).await?;
        if result != true {
            return Err(format!("assertion failed: text '{}' not found on page", text).into());
        }
        Ok(())
    }

    /// Assert that the current URL matches a pattern (substring).
    pub async fn assert_url(&self, pattern: &str) -> Result<(), Box<dyn std::error::Error>> {
        let url = self.evaluate("location.href").await?;
        let href = url.as_str().unwrap_or("");
        if !href.contains(pattern) {
            return Err(format!(
                "assertion failed: URL '{}' does not contain '{}'",
                href, pattern
            )
            .into());
        }
        Ok(())
    }

    /// Assert that a CSS selector does NOT exist in the DOM — pure CDP
    pub async fn assert_not_selector(
        &self,
        selector: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match self.resolve_selector(selector).await {
            Ok(_) => {
                Err(format!("assertion failed: selector '{}' should not exist", selector).into())
            }
            Err(_) => Ok(()),
        }
    }

    /// One-shot page exploration — returns everything an Agent needs to forge an adapter.
    /// Combines screenshot + interactive elements + forms + auth state in a single call.
    pub async fn explore(
        &self,
        screenshot_path: &str,
    ) -> Result<ExploreResult, Box<dyn std::error::Error>> {
        // 1. Screenshot
        self.screenshot(screenshot_path).await?;

        // 2. Page info
        let page_info = self.get_page_info().await?;
        let url = page_info["url"].as_str().unwrap_or("").to_string();
        let title = page_info["title"].as_str().unwrap_or("").to_string();

        // 3. Interactive elements + forms + auth detection in a single evaluate
        let js = r#"(() => {
            const elements = [];
            const forms = [];
            let loggedIn = false;

            // Detect login state: no visible login button = likely logged in
            const loginKeywords = ['登录', '登入', 'log in', 'sign in', 'login'];
            const allText = document.body?.innerText?.toLowerCase() || '';
            const hasLoginButton = loginKeywords.some(kw => {
                const els = document.querySelectorAll('button, a, [role=button]');
                return [...els].some(el => el.textContent.trim().toLowerCase().includes(kw)
                    && el.offsetParent !== null);
            });
            loggedIn = !hasLoginButton;

            // Find interactive elements
            const interactiveSelectors = 'button, a[href], input, textarea, select, [role=button], [role=link], [role=textbox], [contenteditable=true]';
            document.querySelectorAll(interactiveSelectors).forEach(el => {
                const rect = el.getBoundingClientRect();
                if (rect.width === 0 || rect.height === 0) return;
                if (el.offsetParent === null && getComputedStyle(el).position !== 'fixed') return;

                const tag = el.tagName.toLowerCase();
                const role = el.getAttribute('role') || tag;
                const text = (el.textContent || el.getAttribute('aria-label') || el.getAttribute('placeholder') || '').trim().slice(0, 80);

                let sel = tag;
                if (el.id) sel = '#' + el.id;
                else if (el.name) sel = tag + '[name=' + JSON.stringify(el.name) + ']';
                else if (el.className && typeof el.className === 'string') {
                    const cls = el.className.trim().split(/\s+/)[0];
                    if (cls && !cls.match(/[A-Z][a-z]+[A-Z]/)) sel = tag + '.' + CSS.escape(cls);
                }

                let elementType = 'other';
                if (tag === 'button' || role === 'button') elementType = 'button';
                else if (tag === 'a' || role === 'link') elementType = 'link';
                else if (tag === 'input') {
                    const t = el.type || 'text';
                    if (t === 'checkbox') elementType = 'checkbox';
                    else if (t === 'radio') elementType = 'radio';
                    else elementType = 'input';
                }
                else if (tag === 'textarea' || role === 'textbox') elementType = 'textarea';
                else if (tag === 'select') elementType = 'select';
                else if (el.isContentEditable) elementType = 'content_editable';

                elements.push({
                    tag, role, text, selector: sel,
                    x: Math.round(rect.x + rect.width/2),
                    y: Math.round(rect.y + rect.height/2),
                    width: Math.round(rect.width),
                    height: Math.round(rect.height),
                    element_type: elementType,
                });
            });

            // Find forms
            document.querySelectorAll('form').forEach(form => {
                let formSel = 'form';
                if (form.id) formSel = '#' + form.id;
                else if (form.name) formSel = 'form[name=' + JSON.stringify(form.name) + ']';

                const fields = [];
                form.querySelectorAll('input, textarea, select').forEach(f => {
                    if (f.type === 'hidden' || f.type === 'submit') return;
                    let fSel = f.tagName.toLowerCase();
                    if (f.id) fSel = '#' + f.id;
                    else if (f.name) fSel = fSel + '[name=' + JSON.stringify(f.name) + ']';
                    fields.push({
                        selector: fSel,
                        field_type: f.type || f.tagName.toLowerCase(),
                        name: f.name || '',
                        placeholder: f.placeholder || f.getAttribute('aria-label') || '',
                    });
                });

                const submitBtn = form.querySelector('button[type=submit], input[type=submit], button:not([type])');
                let submitSel = null;
                if (submitBtn) {
                    if (submitBtn.id) submitSel = '#' + submitBtn.id;
                    else submitSel = 'button[type=submit]';
                }

                forms.push({ selector: formSel, fields, submit_selector: submitSel });
            });

            return { loggedIn, elements, forms };
        })()"#;

        let result = self.evaluate(js).await?;

        let logged_in = result["loggedIn"].as_bool().unwrap_or(false);

        let interactive_elements: Vec<InteractiveElement> = result["elements"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| serde_json::from_value(v.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();

        let forms: Vec<FormInfo> = result["forms"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| serde_json::from_value(v.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();

        let hints = ExploreHints::from_elements(&interactive_elements);

        Ok(ExploreResult {
            url,
            title,
            screenshot_path: screenshot_path.to_string(),
            logged_in,
            interactive_elements,
            forms,
            hints: Some(hints),
        })
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

        // If no page target exists, create one via /json/new
        if !targets.iter().any(|t| t["type"].as_str() == Some("page")) {
            let new_body = Self::http_get(port, "/json/new").await?;
            let new_target: Value = serde_json::from_str(&new_body)?;
            let ws_url = new_target["webSocketDebuggerUrl"]
                .as_str()
                .ok_or("created new tab but it has no webSocketDebuggerUrl")?;
            return Ok(ws_url.to_string());
        }

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

    // --- Stealth tests ---

    #[test]
    fn stealth_js_contains_webdriver_patch() {
        assert!(STEALTH_JS.contains("webdriver"));
        assert!(STEALTH_JS.contains("defineProperty"));
    }

    #[test]
    fn stealth_js_contains_chrome_runtime() {
        assert!(STEALTH_JS.contains("chrome"));
        assert!(STEALTH_JS.contains("runtime"));
    }

    #[test]
    fn stealth_js_contains_plugins_patch() {
        assert!(STEALTH_JS.contains("plugins"));
    }

    #[test]
    fn stealth_js_balanced_syntax() {
        let opens: usize = STEALTH_JS.matches('(').count();
        let closes: usize = STEALTH_JS.matches(')').count();
        assert_eq!(opens, closes, "unbalanced parentheses in STEALTH_JS");

        let open_braces: usize = STEALTH_JS.matches('{').count();
        let close_braces: usize = STEALTH_JS.matches('}').count();
        assert_eq!(open_braces, close_braces, "unbalanced braces in STEALTH_JS");
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

    // ---- explore: one-shot page panorama for adapter forging ----
    // Classification: quality, what — incomplete explore = more round-trips = slow forging
    // Why: Agent needs all forging context in a single call to minimize MCP round-trips

    #[test]
    fn explore_result_contains_all_forging_context() {
        let result = ExploreResult {
            url: "https://example.com".to_string(),
            title: "Example".to_string(),
            screenshot_path: "/tmp/explore.png".to_string(),
            logged_in: false,
            interactive_elements: vec![InteractiveElement {
                tag: "button".to_string(),
                role: "button".to_string(),
                text: "Submit".to_string(),
                selector: "button.submit".to_string(),
                x: 100,
                y: 200,
                width: 80,
                height: 30,
                element_type: ElementType::Button,
            }],
            forms: vec![FormInfo {
                selector: "form#login".to_string(),
                fields: vec![FormField {
                    selector: "input#username".to_string(),
                    field_type: "text".to_string(),
                    name: "username".to_string(),
                    placeholder: "Enter username".to_string(),
                }],
                submit_selector: Some("button[type=submit]".to_string()),
            }],
            hints: None,
        };

        // Must contain page identity
        assert!(!result.url.is_empty());
        assert!(!result.title.is_empty());
        // Must contain screenshot for visual context
        assert!(!result.screenshot_path.is_empty());
        // Must report auth state
        assert!(!result.logged_in);
        // Must enumerate interactive elements with stable selectors
        assert_eq!(result.interactive_elements.len(), 1);
        assert!(!result.interactive_elements[0].selector.is_empty());
        // Must detect forms with their fields
        assert_eq!(result.forms.len(), 1);
        assert_eq!(result.forms[0].fields.len(), 1);
    }

    #[test]
    fn explore_result_serializes_to_json() {
        // Why: MCP tools return JSON — explore must be serializable
        let result = ExploreResult {
            url: "https://test.com".to_string(),
            title: "Test".to_string(),
            screenshot_path: "/tmp/test.png".to_string(),
            logged_in: true,
            interactive_elements: vec![],
            forms: vec![],
            hints: None,
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["url"], "https://test.com");
        assert_eq!(json["logged_in"], true);
        assert!(json["interactive_elements"].is_array());
        assert!(json["forms"].is_array());
    }

    // ---- explore hints: auto-detect primary input + submit ----
    // Classification: delight, what — saves Agent one round of analysis
    // Why: Agent shouldn't scan 22 elements to find the obvious input+submit pair

    #[test]
    fn explore_result_identifies_primary_input() {
        // The largest visible textbox/textarea = primary input
        let result = ExploreResult {
            url: "https://example.com".to_string(),
            title: "Test".to_string(),
            screenshot_path: "/tmp/test.png".to_string(),
            logged_in: true,
            interactive_elements: vec![
                InteractiveElement {
                    tag: "input".to_string(),
                    role: "input".to_string(),
                    text: "Search".to_string(),
                    selector: "input.search".to_string(),
                    x: 100,
                    y: 50,
                    width: 200,
                    height: 30,
                    element_type: ElementType::Input,
                },
                InteractiveElement {
                    tag: "div".to_string(),
                    role: "textbox".to_string(),
                    text: "Enter prompt".to_string(),
                    selector: "div.tiptap".to_string(),
                    x: 400,
                    y: 200,
                    width: 900,
                    height: 96,
                    element_type: ElementType::Textarea,
                },
            ],
            forms: vec![],
            hints: None,
        };
        let hints = ExploreHints::from_elements(&result.interactive_elements);
        // Must pick the largest textbox as primary input
        assert_eq!(hints.primary_input.as_ref().unwrap().selector, "div.tiptap");
    }

    #[test]
    fn explore_result_identifies_submit_button() {
        // Primary button closest to primary input = submit
        let elements = vec![
            InteractiveElement {
                tag: "div".to_string(),
                role: "textbox".to_string(),
                text: "Enter prompt".to_string(),
                selector: "div.tiptap".to_string(),
                x: 400,
                y: 200,
                width: 900,
                height: 96,
                element_type: ElementType::Textarea,
            },
            InteractiveElement {
                tag: "button".to_string(),
                role: "button".to_string(),
                text: "".to_string(),
                selector: "button.submit".to_string(),
                x: 500,
                y: 220,
                width: 36,
                height: 36,
                element_type: ElementType::Button,
            },
            InteractiveElement {
                tag: "button".to_string(),
                role: "button".to_string(),
                text: "Settings".to_string(),
                selector: "button.settings".to_string(),
                x: 50,
                y: 1200,
                width: 80,
                height: 30,
                element_type: ElementType::Button,
            },
        ];
        let hints = ExploreHints::from_elements(&elements);
        // Must pick the button closest to the primary input
        assert_eq!(
            hints.submit_button.as_ref().unwrap().selector,
            "button.submit"
        );
    }
}
