//! MCP (Model Context Protocol) server implementation.
//!
//! Exposes claw's forge toolkit as MCP tools over stdin/stdout JSON-RPC.
//! This lets AI agents (Claude Code, etc.) use claw's scalpels natively.

use std::collections::HashMap;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::browser;
use crate::cdp::CdpClient;

/// Run the MCP server: read JSON-RPC from stdin, write responses to stdout.
pub async fn serve(port: u16, headless: bool) -> Result<(), Box<dyn std::error::Error>> {
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin);

    // Lazy-init CDP client on first tool call
    let mut client: Option<CdpClient> = None;

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
                // Ensure CDP client is connected
                if client.is_none() {
                    browser::ensure_chrome(port, headless).await?;
                    let ws_url = CdpClient::discover_ws_url(port).await?;
                    client = Some(CdpClient::connect(&ws_url).await?);
                }
                handle_tool_call(&id, &request["params"], client.as_ref().unwrap()).await
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
            "name": "verify_adapter",
            "description": "Verify a claw — dry-run and report per-step health (pass/fail, timing, diagnostics).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "site": { "type": "string", "description": "Site name" },
                    "name": { "type": "string", "description": "Adapter name" },
                    "args": { "type": "object", "description": "Adapter arguments", "additionalProperties": true }
                },
                "required": ["site", "name"]
            }
        },
        {
            "name": "try_step",
            "description": "Try a single pipeline step and return structured result (status, timing, error). Use during forging to test steps incrementally.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "step": { "type": "string", "description": "Pipeline step as YAML (e.g. 'navigate: https://example.com')" },
                    "args": { "type": "object", "description": "Template arguments", "additionalProperties": true }
                },
                "required": ["step"]
            }
        },
        {
            "name": "download",
            "description": "Download a URL to a local file. Returns file path and size.",
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
    ])
}

async fn handle_tool_call(id: &Value, params: &Value, client: &CdpClient) -> Value {
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
    client: &CdpClient,
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
        "verify_adapter" => {
            let site = args["site"].as_str().ok_or("missing site")?;
            let name = args["name"].as_str().ok_or("missing name")?;

            let home = std::env::var("HOME").unwrap_or_default();
            let dirs = ["adapters".to_string(), format!("{}/.claw/adapters", home)];
            let refs: Vec<&str> = dirs.iter().map(|s| s.as_str()).collect();
            let ada = crate::adapter::load_adapter(&refs, site, name)?;

            let mut adapter_args = HashMap::new();
            if let Some(ref defs) = ada.args {
                for (key, def) in defs {
                    if let Some(ref default) = def.default {
                        adapter_args.insert(key.clone(), default.clone());
                    }
                }
            }
            // Merge provided args
            if let Some(obj) = args["args"].as_object() {
                for (k, v) in obj {
                    adapter_args.insert(k.clone(), v.clone());
                }
            }

            let results =
                crate::pipeline::execute_with_report(&ada.pipeline, client, adapter_args, 0).await;
            Ok(json!(results))
        }
        "try_step" => {
            let step_yaml = args["step"].as_str().ok_or("missing step")?;
            let parsed = crate::adapter::parse_single_step(step_yaml)?;

            let mut step_args = HashMap::new();
            if let Some(obj) = args["args"].as_object() {
                for (k, v) in obj {
                    step_args.insert(k.clone(), v.clone());
                }
            }

            let label = crate::pipeline::step_label(&parsed);
            let start = std::time::Instant::now();
            let mut data = Vec::new();
            let mut rows = Vec::new();
            let result = crate::pipeline::execute_single_step(
                &parsed, client, &step_args, &mut data, &mut rows, 0,
            )
            .await;
            let duration_ms = start.elapsed().as_millis();

            Ok(json!({
                "step": label,
                "status": if result.is_ok() { "pass" } else { "fail" },
                "duration_ms": duration_ms,
                "error": result.err().map(|e| e.to_string())
            }))
        }
        "download" => {
            let url = args["url"].as_str().ok_or("missing url")?;
            let output = args["output"].as_str().ok_or("missing output")?;
            let http = reqwest::Client::new();
            let resp = http
                .get(url)
                .send()
                .await
                .map_err(|e| -> Box<dyn std::error::Error> {
                    format!("download failed: {}", e).into()
                })?;
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| -> Box<dyn std::error::Error> {
                    format!("download read failed: {}", e).into()
                })?;
            std::fs::write(output, &bytes)?;
            Ok(json!(format!("{} ({} bytes)", output, bytes.len())))
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
            let mut adapter_args = HashMap::new();
            if let Some(obj) = args["args"].as_object() {
                for (k, v) in obj {
                    adapter_args.insert(k.clone(), v.clone());
                }
            }
            let (columns, rows) = crate::adapter::run_adapter(client, site, name, adapter_args, 0)
                .await
                .map_err(|e| -> Box<dyn std::error::Error> { e })?;
            let json_rows: Vec<Value> = rows
                .iter()
                .map(|row| {
                    let obj: serde_json::Map<String, Value> = row
                        .iter()
                        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                        .collect();
                    Value::Object(obj)
                })
                .collect();
            Ok(json!({
                "columns": columns,
                "rows": json_rows,
                "count": json_rows.len()
            }))
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
        // run_adapter must require site and name
        let tool = tools.iter().find(|t| t["name"] == "run_adapter").unwrap();
        let required = tool["inputSchema"]["required"].as_array().unwrap();
        assert!(required.contains(&json!("site")));
        assert!(required.contains(&json!("name")));
    }
}
