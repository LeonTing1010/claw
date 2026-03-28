//! MCP (Model Context Protocol) server implementation.
//!
//! Exposes claw's forge toolkit as MCP tools over stdin/stdout JSON-RPC.
//! This lets AI agents (Claude Code, etc.) use claw's scalpels natively.

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::bridge::BridgeServer;
use crate::cdp::BridgeClient;

/// Run the MCP server: read JSON-RPC from stdin, write responses to stdout.
pub async fn serve() -> Result<(), Box<dyn std::error::Error>> {
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin);

    // Start bridge server immediately — extension can connect anytime
    let bridge = BridgeServer::start();

    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break; // EOF
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let request: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                let err_resp = json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": { "code": -32700, "message": format!("parse error: {}", e) }
                });
                write_response(&mut stdout, &err_resp).await?;
                continue;
            }
        };

        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request["method"].as_str().unwrap_or("");

        let response = match method {
            "initialize" => handle_initialize(&id),
            "notifications/initialized" => continue, // no response needed
            "tools/list" => handle_tools_list(&id),
            "tools/call" => {
                // Wait for extension bridge — poll up to 30s
                let client = match bridge.get_client().await {
                    Some(c) => c,
                    None => {
                        eprintln!("mcp: waiting for Chrome extension...");
                        let mut client_opt = None;
                        for _ in 0..30 {
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                            if let Some(c) = bridge.get_client().await {
                                eprintln!("mcp: connected via Chrome extension bridge");
                                client_opt = Some(c);
                                break;
                            }
                        }
                        match client_opt {
                            Some(c) => c,
                            None => {
                                let err_resp = json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "result": {
                                        "content": [{"type": "text", "text": "error: Chrome extension not connected. Install Claw extension and reload it."}],
                                        "isError": true
                                    }
                                });
                                write_response(&mut stdout, &err_resp).await?;
                                continue;
                            }
                        }
                    }
                };
                handle_tool_call(&id, &request["params"], &client).await
            }
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("method not found: {}", method) }
            }),
        };

        write_response(&mut stdout, &response).await?;
    }

    Ok(())
}

async fn write_response(
    stdout: &mut tokio::io::Stdout,
    response: &Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let s = serde_json::to_string(response)?;
    stdout.write_all(s.as_bytes()).await?;
    stdout.write_all(b"\n").await?;
    stdout.flush().await?;
    Ok(())
}

fn handle_initialize(id: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "claw",
                "version": env!("CARGO_PKG_VERSION")
            }
        }
    })
}

fn handle_tools_list(id: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "tools": tools_schema()
        }
    })
}

fn tools_schema() -> Value {
    json!([
        {
            "name": "screenshot",
            "description": "Take a screenshot of the current page. Returns the file path.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Output file path", "default": "/tmp/claw-screenshot.png" },
                    "full_page": { "type": "boolean", "description": "Capture full page beyond viewport", "default": false }
                }
            }
        },
        {
            "name": "navigate",
            "description": "Navigate the browser to a URL.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Target URL" }
                },
                "required": ["url"]
            }
        },
        {
            "name": "ax_tree",
            "description": "Get the accessibility tree — semantic page structure. Primary perception tool.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "depth": { "type": "integer", "description": "Max depth to traverse" }
                }
            }
        },
        {
            "name": "read_dom",
            "description": "Get a simplified DOM tree with key attributes (id, class, role, text, box).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector for subtree root (default: body)" },
                    "depth": { "type": "integer", "description": "Max depth", "default": 10 }
                }
            }
        },
        {
            "name": "page_info",
            "description": "Get current page info: URL, title, viewport, scroll position, readyState.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "find",
            "description": "Find elements by visible text. Returns list with tag, role, text, selector, coordinates.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Text to search for" },
                    "role": { "type": "string", "description": "Filter by element role (button, link, input, etc.)" }
                },
                "required": ["query"]
            }
        },
        {
            "name": "element_info",
            "description": "Deep probe of a single element: tag, attributes, box model, visibility, editable.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector" }
                },
                "required": ["selector"]
            }
        },
        {
            "name": "click",
            "description": "Click on an element by visible text content. Uses CDP native mouse events.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "Visible text to click" }
                },
                "required": ["text"]
            }
        },
        {
            "name": "click_selector",
            "description": "Click on an element by CSS selector. Uses CDP native mouse events.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector to click" }
                },
                "required": ["selector"]
            }
        },
        {
            "name": "type_text",
            "description": "Type text into an input element. Focuses, clears, then types via CDP keyboard events.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector of the input" },
                    "text": { "type": "string", "description": "Text to type" }
                },
                "required": ["selector", "text"]
            }
        },
        {
            "name": "hover",
            "description": "Hover over an element. Triggers CSS :hover, tooltips, dropdown menus.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector to hover" }
                },
                "required": ["selector"]
            }
        },
        {
            "name": "scroll",
            "description": "Scroll an element into view.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector to scroll to" }
                },
                "required": ["selector"]
            }
        },
        {
            "name": "press_key",
            "description": "Press a specific key (Enter, Tab, Escape, ArrowDown, etc.).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "Key name" },
                    "modifiers": { "type": "integer", "description": "Modifier bitmask: Alt=1, Ctrl=2, Meta=4, Shift=8", "default": 0 }
                },
                "required": ["key"]
            }
        },
        {
            "name": "select",
            "description": "Select an option in a <select> dropdown.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector of the <select>" },
                    "value": { "type": "string", "description": "Value to select" }
                },
                "required": ["selector", "value"]
            }
        },
        {
            "name": "upload",
            "description": "Upload files to a file input element via CDP.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector of the file input" },
                    "files": { "type": "string", "description": "Comma-separated file paths" }
                },
                "required": ["selector", "files"]
            }
        },
        {
            "name": "evaluate",
            "description": "Evaluate a JavaScript expression in the browser and return the result.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "expression": { "type": "string", "description": "JS expression to evaluate" }
                },
                "required": ["expression"]
            }
        },
        {
            "name": "cookies",
            "description": "Get cookies for the current page.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "dismiss_dialog",
            "description": "Handle a JavaScript dialog (alert/confirm/prompt).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "accept": { "type": "boolean", "description": "Accept or dismiss", "default": true },
                    "prompt_text": { "type": "string", "description": "Text for prompt dialogs" }
                }
            }
        },
        {
            "name": "force_state",
            "description": "Force pseudo-state (:hover, :focus) on an element without actually hovering.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector" },
                    "states": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Pseudo-states: hover, focus, active, focus-within"
                    }
                },
                "required": ["selector", "states"]
            }
        },
        {
            "name": "hit_test",
            "description": "What element is at pixel (x, y)?",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "x": { "type": "number", "description": "X coordinate" },
                    "y": { "type": "number", "description": "Y coordinate" }
                },
                "required": ["x", "y"]
            }
        },
        {
            "name": "top_layer",
            "description": "Find blocking modals/dialogs in the top layer.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "event_listeners",
            "description": "List event listeners on an element (click, input, etc.).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector" }
                },
                "required": ["selector"]
            }
        },
        {
            "name": "network_log_start",
            "description": "Start capturing network requests via CDP Network domain (pure protocol, no JS injection). Call this BEFORE triggering page actions to capture API calls.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "network_log_dump",
            "description": "Get captured network log entries (URL, method, status, headers, mime type) and clear the buffer.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "network_log_dump_bodies",
            "description": "Get captured network log with full response bodies for API responses. Use this to discover API endpoints and their data structure. Filters to JSON/text responses only. RECOMMENDED: call network_log_start first, trigger an action, then call this to see what APIs the page called.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url_filter": { "type": "string", "description": "Optional substring to filter URLs (e.g. 'api/' or 'graphql')" }
                }
            }
        },
        {
            "name": "download",
            "description": "Download a URL to a local file using the browser session (cookies, referer, auth). Works with auth-gated and anti-hotlink URLs.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL to download" },
                    "output": { "type": "string", "description": "Output file path" }
                },
                "required": ["url", "output"]
            }
        },
        {
            "name": "save_image",
            "description": "Download an image from the current page by CSS selector. Uses the browser session so it works with auth-gated images.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector of the image element (e.g. 'img.hero', '#main-photo')" },
                    "output": { "type": "string", "description": "Output file path (e.g. '/tmp/image.png')" }
                },
                "required": ["selector", "output"]
            }
        },
        {
            "name": "page_intelligence",
            "description": "One-shot page analysis for claw forging. Returns framework detection, SSR state (with data samples), API endpoint hints, interactive elements, auth state, and ranked strategy recommendations — all in a single call. Replaces 5-8 separate tool calls (screenshot + ax_tree + global_names + api_log + page_info). Call this FIRST when forging a new claw.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Navigate to this URL before analysis (optional — omit to analyze current page)" }
                }
            }
        },
        {
            "name": "list_adapters",
            "description": "List all available claws. Returns site, name, and description for each. Use this to discover what websites Claw can access.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "run_adapter",
            "description": "Run a claw and return structured data (JSON rows). This is the primary way to get data from websites. Example: run_adapter({site: 'weibo', name: 'hot'}) returns trending topics.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "site": { "type": "string", "description": "Site name (e.g. 'weibo', 'bilibili')" },
                    "name": { "type": "string", "description": "Adapter name (e.g. 'hot', 'trending')" },
                    "args": { "type": "object", "description": "Adapter arguments (e.g. {limit: 10})", "additionalProperties": true }
                },
                "required": ["site", "name"]
            }
        },
        // ===== FORGE — Agent's Scalpels (Deep Inspection) =====
        {
            "name": "api_log",
            "description": "Get all API calls (fetch/XHR) recorded since page load. Returns url, method, status, request_body, response_body for each call. This captures everything the page does — no manual network_log needed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clear": { "type": "boolean", "description": "Clear the log after reading (default: false)" }
                }
            }
        },
        {
            "name": "global_names",
            "description": "List all interesting global variables on the page. Discovers __INITIAL_STATE__, __NEXT_DATA__, __NUXT__, __pinia, Redux stores, and other framework/SSR state. One call reveals what data the page already has.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "resource_tree",
            "description": "List all resources (scripts, stylesheets, images) loaded by the page. Use with resource_content and search_resource to find API endpoints in source code.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "resource_content",
            "description": "Get the source content of a loaded resource (JavaScript file, HTML, etc). Use after resource_tree to read specific scripts.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL of the resource to read" }
                },
                "required": ["url"]
            }
        },
        {
            "name": "search_resource",
            "description": "Search within all loaded JavaScript/HTML resources for a pattern (e.g. '/api/', 'fetch(', 'axios'). Finds API endpoints directly from source code without triggering UI.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search pattern (plain text)" }
                },
                "required": ["query"]
            }
        },
        {
            "name": "request_replay",
            "description": "Replay an API request within the page context — uses the page's cookies, origin, and session. Zero anti-crawl detection risk. Use after seeing an API in api_log to test with different parameters.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL to fetch" },
                    "method": { "type": "string", "description": "HTTP method (default: GET)" },
                    "headers": { "type": "object", "description": "Extra headers to send" },
                    "body": { "type": "string", "description": "Request body (for POST/PUT)" }
                },
                "required": ["url"]
            }
        },
        {
            "name": "storage_items",
            "description": "Read localStorage or sessionStorage. Many SPAs store auth tokens, API keys, and user data here.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "type": { "type": "string", "description": "Storage type: 'local' (default) or 'session'" }
                }
            }
        },
        // ===== FORGE — Claw Creation Pipeline =====
        {
            "name": "forge_verify",
            "description": "One-shot test of claw extraction logic. Navigates to URL, waits, evaluates a JS expression in page context, and validates the result shape against expected columns. Combines navigate + wait + evaluate + validate into one call. Use this during forging to iterate quickly on the data extraction logic before saving.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL to navigate to" },
                    "wait_ms": { "type": "integer", "description": "Milliseconds to wait after navigation (default: 2000)", "default": 2000 },
                    "expression": { "type": "string", "description": "JS expression that returns an array of objects (the claw's data extraction logic)" },
                    "columns": { "type": "array", "items": { "type": "string" }, "description": "Expected column names — used to validate the result shape" }
                },
                "required": ["url", "expression"]
            }
        },
        {
            "name": "forge_save",
            "description": "Save a .claw.js file to disk. Writes to ~/.claw/claws/{site}/{name}.claw.js. Use after verifying the claw works with forge_verify.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "site": { "type": "string", "description": "Site name (e.g. 'weibo')" },
                    "name": { "type": "string", "description": "Claw name (e.g. 'hot')" },
                    "code": { "type": "string", "description": "Full .claw.js source code" }
                },
                "required": ["site", "name", "code"]
            }
        },
        // ===== INTERCEPT — Active Request Interception =====
        {
            "name": "intercept_on",
            "description": "Start intercepting requests matching a URL pattern. Paused requests appear in intercept_list. Use intercept_continue/fulfill/fail to handle them. TLS fingerprint unchanged — zero detection risk.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url_pattern": { "type": "string", "description": "URL pattern to match (e.g. '*api/search*', '*douyin.com/aweme*')" }
                },
                "required": ["url_pattern"]
            }
        },
        {
            "name": "intercept_off",
            "description": "Stop intercepting requests and release all paused requests.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "intercept_list",
            "description": "List all paused (intercepted) requests — shows requestId, url, method, headers, postData for each. Use requestId with intercept_continue/fulfill/fail.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "intercept_continue",
            "description": "Continue a paused request (optionally modify URL, headers, or POST body before sending to server).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "request_id": { "type": "string", "description": "Request ID from intercept_list" },
                    "url": { "type": "string", "description": "Override URL" },
                    "headers": { "type": "object", "description": "Override headers" },
                    "post_data": { "type": "string", "description": "Override POST body" }
                },
                "required": ["request_id"]
            }
        },
        {
            "name": "intercept_fulfill",
            "description": "Fulfill a paused request with a custom response (bypass server entirely). Useful for testing or mocking.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "request_id": { "type": "string", "description": "Request ID from intercept_list" },
                    "status": { "type": "number", "description": "HTTP status code (default: 200)" },
                    "body": { "type": "string", "description": "Response body" }
                },
                "required": ["request_id", "body"]
            }
        },
        {
            "name": "intercept_fail",
            "description": "Block a paused request (prevent it from reaching the server).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "request_id": { "type": "string", "description": "Request ID from intercept_list" }
                },
                "required": ["request_id"]
            }
        },
        {
            "name": "set_cookie",
            "description": "Set a cookie on a domain. Use for precise auth control.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "value": { "type": "string" },
                    "domain": { "type": "string" },
                    "path": { "type": "string", "description": "Cookie path (default: /)" }
                },
                "required": ["name", "value", "domain"]
            }
        },
    ])
}

async fn handle_tool_call(id: &Value, params: &Value, client: &BridgeClient) -> Value {
    let tool_name = params["name"].as_str().unwrap_or("");
    let args = &params["arguments"];

    let result = execute_tool(tool_name, args, client).await;

    match result {
        Ok(content) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{
                    "type": "text",
                    "text": if content.is_string() {
                        content.as_str().unwrap().to_string()
                    } else {
                        serde_json::to_string_pretty(&content).unwrap_or_default()
                    }
                }]
            }
        }),
        Err(e) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{
                    "type": "text",
                    "text": format!("error: {}", e)
                }],
                "isError": true
            }
        }),
    }
}

async fn execute_tool(
    name: &str,
    args: &Value,
    client: &BridgeClient,
) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "screenshot" => {
            let path = args["path"].as_str().unwrap_or("/tmp/claw-screenshot.png");
            let full = args["full_page"].as_bool().unwrap_or(false);
            if full {
                client.screenshot_full(path).await?;
            } else {
                client.screenshot(path).await?;
            }
            Ok(json!(path))
        }
        "navigate" => {
            let url = args["url"].as_str().ok_or("missing url")?;
            client.navigate(url).await?;
            Ok(json!(format!("navigated to {}", url)))
        }
        "ax_tree" => {
            let depth = args["depth"].as_i64().map(|d| d as i32);
            client.get_ax_tree(depth).await
        }
        "read_dom" => {
            let selector = args["selector"].as_str();
            let depth = args["depth"].as_i64().unwrap_or(10) as i32;
            client.get_dom_tree(selector, depth).await
        }
        "page_info" => client.get_page_info().await,
        "find" => {
            let query = args["query"].as_str().ok_or("missing query")?;
            let role = args["role"].as_str();
            client.find_elements(query, role).await
        }
        "element_info" => {
            let selector = args["selector"].as_str().ok_or("missing selector")?;
            client.get_element_info(selector).await
        }
        "click" => {
            let text = args["text"].as_str().ok_or("missing text")?;
            client.click_text(text).await?;
            Ok(json!(format!("clicked \"{}\"", text)))
        }
        "click_selector" => {
            let selector = args["selector"].as_str().ok_or("missing selector")?;
            client.click_selector(selector).await?;
            Ok(json!(format!("clicked {}", selector)))
        }
        "type_text" => {
            let selector = args["selector"].as_str().ok_or("missing selector")?;
            let text = args["text"].as_str().ok_or("missing text")?;
            client.type_into(selector, text).await?;
            Ok(json!(format!("typed into {}", selector)))
        }
        "hover" => {
            let selector = args["selector"].as_str().ok_or("missing selector")?;
            client.hover_selector(selector).await?;
            Ok(json!(format!("hovered {}", selector)))
        }
        "scroll" => {
            let selector = args["selector"].as_str().ok_or("missing selector")?;
            client.scroll_into_view(selector).await?;
            Ok(json!(format!("scrolled to {}", selector)))
        }
        "press_key" => {
            let key = args["key"].as_str().ok_or("missing key")?;
            let modifiers = args["modifiers"].as_u64().unwrap_or(0) as u32;
            client.press_key(key, modifiers).await?;
            Ok(json!(format!("pressed {}", key)))
        }
        "select" => {
            let selector = args["selector"].as_str().ok_or("missing selector")?;
            let value = args["value"].as_str().ok_or("missing value")?;
            client.select_option(selector, value).await?;
            Ok(json!(format!("selected {} = {}", selector, value)))
        }
        "upload" => {
            let selector = args["selector"].as_str().ok_or("missing selector")?;
            let files_str = args["files"].as_str().ok_or("missing files")?;
            let paths: Vec<&str> = files_str.split(',').map(|s| s.trim()).collect();
            client.upload_files(selector, &paths).await?;
            Ok(json!(format!("uploaded to {}", selector)))
        }
        "evaluate" => {
            let expr = args["expression"].as_str().ok_or("missing expression")?;
            client.evaluate(expr).await
        }
        "cookies" => client.get_cookies().await,
        "hit_test" => {
            let x = args["x"].as_f64().ok_or("missing x")?;
            let y = args["y"].as_f64().ok_or("missing y")?;
            client.hit_test(x, y).await
        }
        "top_layer" => client.get_top_layer().await,
        "event_listeners" => {
            let selector = args["selector"].as_str().ok_or("missing selector")?;
            client.get_event_listeners(selector).await
        }
        "network_log_start" => {
            client.start_network_log().await?;
            Ok(json!("network capture started (CDP Network domain)"))
        }
        "network_log_dump" => client.get_network_log().await,
        "network_log_dump_bodies" => {
            let url_filter = args["url_filter"].as_str();
            client.get_network_log_with_bodies(url_filter).await
        }
        "dismiss_dialog" => {
            let accept = args["accept"].as_bool().unwrap_or(true);
            let prompt_text = args["prompt_text"].as_str();
            client.dismiss_dialog(accept, prompt_text).await?;
            Ok(json!(if accept { "accepted" } else { "dismissed" }))
        }
        "force_state" => {
            let selector = args["selector"].as_str().ok_or("missing selector")?;
            let states: Vec<&str> = args["states"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            client.force_pseudo_state(selector, &states).await?;
            Ok(json!(format!("forced {:?} on {}", states, selector)))
        }
        "download" => {
            let url = args["url"].as_str().ok_or("missing url")?;
            let output = args["output"].as_str().ok_or("missing output")?;
            let size = client.download_via_browser(url, output).await?;
            Ok(json!(format!("{} ({} bytes)", output, size)))
        }
        "save_image" => {
            let selector = args["selector"].as_str().ok_or("missing selector")?;
            let output = args["output"].as_str().ok_or("missing output")?;
            client.save_image(selector, output).await
        }
        "page_intelligence" => {
            // Navigate first if URL provided
            if let Some(url) = args["url"].as_str() {
                client.navigate(url).await?;
            }
            // Relay to extension — gathers framework, SSR state, APIs, interactive, auth in one call
            let result = client
                .send("Claw.pageIntelligence", Some(json!({})))
                .await?;
            Ok(result)
        }
        "list_adapters" => {
            let base_dirs = crate::adapter::adapter_base_dirs();
            let refs: Vec<&str> = base_dirs.iter().map(|s| s.as_str()).collect();
            let adapters = crate::adapter::list_adapters(&refs);
            let list: Vec<Value> = adapters
                .iter()
                .map(|a| {
                    json!({
                        "site": a.site,
                        "name": a.name,
                        "description": a.description,
                        "strategy": a.strategy
                    })
                })
                .collect();
            Ok(json!(list))
        }
        "run_adapter" => {
            let site = args["site"].as_str().ok_or("missing site")?;
            let name = args["name"].as_str().ok_or("missing name")?;
            let adapter_args = args.get("args").cloned().unwrap_or(json!({}));

            // Relay to Chrome extension via bridge
            let mut result = client
                .send(
                    "Claw.run",
                    Some(json!({
                        "site": site,
                        "name": name,
                        "args": adapter_args
                    })),
                )
                .await?;

            // Health validation: if the result contains rows and a health contract, validate
            if let Some(rows) = result.get("rows").and_then(|r| r.as_array()) {
                if let Some(health_val) = result.get("health") {
                    if let Some(contract) = crate::adapter::parse_health_contract(health_val) {
                        let adapter_name = format!("{}/{}", site, name);
                        let report = crate::health::validate(&adapter_name, &contract, rows);
                        result["health_report"] = serde_json::to_value(&report).unwrap_or_default();
                    }
                }
            }

            Ok(result)
        }
        // ===== FORGE — Claw Creation Pipeline =====
        "forge_verify" => {
            let url = args["url"].as_str().ok_or("missing url")?;
            let wait_ms = args["wait_ms"].as_u64().unwrap_or(2000);
            let expression = args["expression"].as_str().ok_or("missing expression")?;
            let columns: Vec<&str> = args["columns"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();

            let start = std::time::Instant::now();

            // Navigate
            client.navigate(url).await?;

            // Wait
            if wait_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await;
            }

            // Evaluate
            let result = client.evaluate(expression).await?;
            let duration_ms = start.elapsed().as_millis();

            // Validate
            let mut diagnostics = Vec::new();

            let rows = result.as_array();
            let row_count = rows.map(|r| r.len()).unwrap_or(0);

            if rows.is_none() {
                diagnostics.push("FAIL: expression did not return an array".to_string());
            } else if row_count == 0 {
                diagnostics.push("WARN: expression returned empty array".to_string());
            } else {
                diagnostics.push(format!("OK: {} rows returned", row_count));

                // Check columns against first row
                if !columns.is_empty() {
                    if let Some(first_row) = rows.unwrap().first() {
                        if let Some(obj) = first_row.as_object() {
                            let actual_keys: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
                            let missing: Vec<&&str> = columns.iter()
                                .filter(|c| !actual_keys.contains(*c))
                                .collect();
                            let extra: Vec<&&str> = actual_keys.iter()
                                .filter(|k| !columns.contains(*k))
                                .collect();
                            if missing.is_empty() {
                                diagnostics.push(format!("OK: all {} columns present", columns.len()));
                            } else {
                                diagnostics.push(format!("FAIL: missing columns: {:?}", missing));
                            }
                            if !extra.is_empty() {
                                diagnostics.push(format!("INFO: extra fields: {:?}", extra));
                            }
                        } else {
                            diagnostics.push("FAIL: first row is not an object".to_string());
                        }
                    }
                }
            }

            let sample = rows
                .map(|r| r.iter().take(5).cloned().collect::<Vec<_>>())
                .unwrap_or_default();

            Ok(json!({
                "status": if diagnostics.iter().any(|d| d.starts_with("FAIL")) { "fail" } else { "pass" },
                "row_count": row_count,
                "duration_ms": duration_ms,
                "sample": sample,
                "diagnostics": diagnostics
            }))
        }
        "forge_save" => {
            let site = args["site"].as_str().ok_or("missing site")?;
            let name = args["name"].as_str().ok_or("missing name")?;
            let code = args["code"].as_str().ok_or("missing code")?;

            let home = std::env::var("HOME").unwrap_or_default();
            let dir = format!("{}/.claw/claws/{}", home, site);
            std::fs::create_dir_all(&dir)?;

            let path = format!("{}/{}.claw.js", dir, name);
            std::fs::write(&path, code)?;

            Ok(json!(format!("saved to {}", path)))
        }
        // ===== FORGE — Agent's Scalpels (Deep Inspection) =====
        "api_log" => {
            let clear = args["clear"].as_bool().unwrap_or(false);
            let log = client.get_api_log().await?;
            if clear {
                client.clear_api_log().await?;
            }
            Ok(log)
        }
        "global_names" => client.get_global_names().await,
        "resource_tree" => client.get_resource_tree().await,
        "resource_content" => {
            let url = args["url"].as_str().ok_or("missing url")?;
            client.get_resource_content(url).await
        }
        "search_resource" => {
            let query = args["query"].as_str().ok_or("missing query")?;
            client.search_resources(query).await
        }
        "request_replay" => {
            let url = args["url"].as_str().ok_or("missing url")?;
            let method = args["method"].as_str().unwrap_or("GET");
            let headers = args.get("headers").filter(|v| !v.is_null());
            let body = args["body"].as_str();
            client.request_replay(url, method, headers, body).await
        }
        "storage_items" => {
            let storage_type = args["type"].as_str().unwrap_or("local");
            client.get_storage_items(storage_type).await
        }
        // ===== INTERCEPT — Active Request Interception =====
        "intercept_on" => {
            let pattern = args["url_pattern"].as_str().ok_or("missing url_pattern")?;
            client.fetch_enable(pattern).await?;
            Ok(json!(format!(
                "intercepting requests matching '{}'",
                pattern
            )))
        }
        "intercept_off" => {
            client.fetch_disable().await?;
            Ok(json!("interception stopped"))
        }
        "intercept_list" => client.get_paused_requests().await,
        "intercept_continue" => {
            let id = args["request_id"].as_str().ok_or("missing request_id")?;
            let url = args["url"].as_str();
            let headers = args.get("headers").filter(|v| !v.is_null());
            let post_data = args["post_data"].as_str();
            client.fetch_continue(id, url, headers, post_data).await?;
            Ok(json!("request continued"))
        }
        "intercept_fulfill" => {
            let id = args["request_id"].as_str().ok_or("missing request_id")?;
            let status = args["status"].as_u64().unwrap_or(200) as u16;
            let body = args["body"].as_str().ok_or("missing body")?;
            client.fetch_fulfill(id, status, body).await?;
            Ok(json!("request fulfilled"))
        }
        "intercept_fail" => {
            let id = args["request_id"].as_str().ok_or("missing request_id")?;
            client.fetch_fail(id).await?;
            Ok(json!("request blocked"))
        }
        "set_cookie" => {
            let name = args["name"].as_str().ok_or("missing name")?;
            let value = args["value"].as_str().ok_or("missing value")?;
            let domain = args["domain"].as_str().ok_or("missing domain")?;
            let path = args["path"].as_str();
            client.set_cookie(name, value, domain, path).await?;
            Ok(json!(format!("cookie '{}' set on {}", name, domain)))
        }
        _ => Err(format!("unknown tool: {}", name).into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_schema_includes_list_adapters() {
        let schema = tools_schema();
        let tools = schema.as_array().unwrap();
        assert!(
            tools.iter().any(|t| t["name"] == "list_adapters"),
            "MCP tools must include list_adapters"
        );
    }

    #[test]
    fn tools_schema_includes_run_adapter() {
        let schema = tools_schema();
        let tools = schema.as_array().unwrap();
        assert!(
            tools.iter().any(|t| t["name"] == "run_adapter"),
            "MCP tools must include run_adapter"
        );
        let tool = tools.iter().find(|t| t["name"] == "run_adapter").unwrap();
        let required = tool["inputSchema"]["required"].as_array().unwrap();
        assert!(required.contains(&json!("site")));
        assert!(required.contains(&json!("name")));
    }

    #[test]
    fn tools_schema_includes_forge_scalpels() {
        let schema = tools_schema();
        let tools = schema.as_array().unwrap();
        let forge_tools = [
            "api_log",
            "global_names",
            "resource_tree",
            "resource_content",
            "search_resource",
            "request_replay",
            "storage_items",
        ];
        for tool_name in &forge_tools {
            assert!(
                tools.iter().any(|t| t["name"] == *tool_name),
                "MCP tools must include {}",
                tool_name
            );
        }
    }

    #[test]
    fn tools_schema_includes_intercept_tools() {
        let schema = tools_schema();
        let tools = schema.as_array().unwrap();
        let intercept_tools = [
            "intercept_on",
            "intercept_off",
            "intercept_list",
            "intercept_continue",
            "intercept_fulfill",
            "intercept_fail",
            "set_cookie",
        ];
        for tool_name in &intercept_tools {
            assert!(
                tools.iter().any(|t| t["name"] == *tool_name),
                "MCP tools must include {}",
                tool_name
            );
        }
    }
}
