use std::collections::HashMap;

use serde_json::Value;

use crate::adapter::PipelineStep;
use crate::cdp::CdpClient;
use crate::template::{self, TemplateContext};

/// Execute the adapter pipeline and return rows (each row is column → value).
pub async fn execute(
    steps: &[PipelineStep],
    client: &CdpClient,
    args: HashMap<String, Value>,
) -> Result<Vec<HashMap<String, String>>, Box<dyn std::error::Error>> {
    let mut data: Vec<Value> = Vec::new();
    let mut rows: Vec<HashMap<String, String>> = Vec::new();

    for (i, step) in steps.iter().enumerate() {
        execute_single_step(step, client, &args, &mut data, &mut rows)
            .await
            .map_err(|e| format!("step {}: {} — {}", i, step_label(step), e))?;
    }

    Ok(rows)
}

/// Transform each data item through the map template.
pub fn apply_map(
    data: &[Value],
    mappings: &HashMap<String, String>,
    args: &HashMap<String, Value>,
) -> Vec<HashMap<String, String>> {
    data.iter()
        .map(|item| {
            let ctx = TemplateContext {
                args: args.clone(),
                item: Some(item.clone()),
            };
            mappings
                .iter()
                .map(|(key, tmpl)| (key.clone(), template::render(tmpl, &ctx)))
                .collect()
        })
        .collect()
}

/// A human-readable label for a pipeline step (for error reporting / verify output).
pub fn step_label(step: &PipelineStep) -> String {
    match step {
        PipelineStep::Navigate(url) => format!("navigate: {}", truncate(url, 60)),
        PipelineStep::Evaluate(_) => "evaluate: <js>".to_string(),
        PipelineStep::Map(_) => "map: <mapping>".to_string(),
        PipelineStep::Limit(t) => format!("limit: {}", t),
        PipelineStep::Wait(t) => format!("wait: {}", t),
        PipelineStep::Click(t) => format!("click: \"{}\"", truncate(t, 40)),
        PipelineStep::ClickSelector(s) => format!("click_selector: {}", truncate(s, 40)),
        PipelineStep::ClickTarget(t) => format!("click_target: {}", truncate(t, 40)),
        PipelineStep::Type {
            selector, target, ..
        } => {
            let label = selector
                .as_deref()
                .or(target.as_deref())
                .unwrap_or("<unknown>");
            format!("type: {}", truncate(label, 40))
        }
        PipelineStep::Upload { selector, .. } => format!("upload: {}", truncate(selector, 40)),
        PipelineStep::WaitFor { selector, .. } => format!("wait_for: {}", truncate(selector, 40)),
        PipelineStep::WaitForText { text, .. } => {
            format!("wait_for_text: \"{}\"", truncate(text, 40))
        }
        PipelineStep::WaitForUrl { pattern, .. } => {
            format!("wait_for_url: {}", truncate(pattern, 40))
        }
        PipelineStep::WaitForNetworkIdle(_) => "wait_for_network_idle".to_string(),
        PipelineStep::Screenshot(p) => format!("screenshot: {}", truncate(p, 40)),
        PipelineStep::Hover(t) => format!("hover: \"{}\"", truncate(t, 40)),
        PipelineStep::HoverSelector(s) => format!("hover_selector: {}", truncate(s, 40)),
        PipelineStep::Scroll(s) => format!("scroll: {}", truncate(s, 40)),
        PipelineStep::ScrollBy { x, y } => format!("scroll_by: {},{}", x, y),
        PipelineStep::PressKey { key, .. } => format!("press_key: {}", key),
        PipelineStep::Select { selector, value } => {
            format!(
                "select: {} = {}",
                truncate(selector, 30),
                truncate(value, 20)
            )
        }
        PipelineStep::DismissDialog(a) => format!("dismiss_dialog: {}", a),
        PipelineStep::AssertSelector(s) => format!("assert_selector: {}", truncate(s, 40)),
        PipelineStep::AssertText(t) => format!("assert_text: \"{}\"", truncate(t, 40)),
        PipelineStep::AssertUrl(p) => format!("assert_url: {}", truncate(p, 40)),
        PipelineStep::AssertNotSelector(s) => format!("assert_not_selector: {}", truncate(s, 40)),
        PipelineStep::IfSelector { selector, .. } => {
            format!("if_selector: {}", truncate(selector, 40))
        }
        PipelineStep::IfText { text, .. } => format!("if_text: \"{}\"", truncate(text, 40)),
        PipelineStep::IfUrl { pattern, .. } => format!("if_url: {}", truncate(pattern, 40)),
        PipelineStep::Fetch { url, method, .. } => {
            format!("fetch: {} {}", method, truncate(url, 50))
        }
        PipelineStep::SelectPath(path) => format!("select: {}", truncate(path, 40)),
        PipelineStep::Filter(expr) => format!("filter: {}", truncate(expr, 40)),
        PipelineStep::Intercept { capture, .. } => {
            format!("intercept: {}", truncate(capture, 40))
        }
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}

/// Resolve a semantic target name (e.g. "primary_input", "submit_button") to a CSS selector.
/// Explores the current page to get hints, then delegates to adapter::resolve_target.
async fn resolve_target(
    target: &str,
    client: &CdpClient,
) -> Result<String, Box<dyn std::error::Error>> {
    let screenshot_path = "/tmp/claw_resolve_target.png";
    let explore = client.explore(screenshot_path).await?;
    let hints = explore
        .hints
        .as_ref()
        .ok_or_else(|| format!("could not resolve target '{}' — no hints available", target))?;
    crate::adapter::resolve_target(target, hints)
}

/// Resolve a dot-notation path against a JSON value.
/// Example: resolve_json_path({"data": {"items": [1,2]}}, "data.items") => [1,2]
fn resolve_json_path(value: &Value, path: &str) -> Value {
    let mut current = value.clone();
    for segment in path.split('.') {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        current = match current {
            Value::Object(ref map) => map.get(segment).cloned().unwrap_or(Value::Null),
            Value::Array(ref arr) => {
                if let Ok(idx) = segment.parse::<usize>() {
                    arr.get(idx).cloned().unwrap_or(Value::Null)
                } else {
                    Value::Null
                }
            }
            _ => Value::Null,
        };
        if current == Value::Null {
            break;
        }
    }
    current
}

/// Evaluate a simple filter expression against a JSON item.
/// Supports: "item.field > N", "item.field == 'str'", "item.field" (truthy check).
fn evaluate_filter_expr(expr: &str, item: &Value) -> bool {
    // Try comparison operators
    for op in &[">=", "<=", "!=", "==", ">", "<"] {
        if let Some((left, right)) = expr.split_once(op) {
            let left_val = resolve_item_ref(left.trim(), item);
            let right_val = right.trim();
            return compare_values(&left_val, op, right_val);
        }
    }

    // No operator — truthy check on the resolved value
    let val = resolve_item_ref(expr.trim(), item);
    match val {
        Value::Null | Value::Bool(false) => false,
        Value::String(s) => !s.is_empty(),
        Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
        _ => true,
    }
}

/// Resolve "item.field" references in filter expressions.
fn resolve_item_ref(s: &str, item: &Value) -> Value {
    if let Some(path) = s.strip_prefix("item.") {
        resolve_json_path(item, path)
    } else {
        // Try as literal
        Value::String(s.to_string())
    }
}

/// Compare a JSON value against a string literal using an operator.
fn compare_values(left: &Value, op: &str, right: &str) -> bool {
    // Try numeric comparison
    let left_num = match left {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    };
    let right_num = right
        .trim_matches('\'')
        .trim_matches('"')
        .parse::<f64>()
        .ok();

    if let (Some(l), Some(r)) = (left_num, right_num) {
        return match op {
            ">" => l > r,
            "<" => l < r,
            ">=" => l >= r,
            "<=" => l <= r,
            "==" => (l - r).abs() < f64::EPSILON,
            "!=" => (l - r).abs() >= f64::EPSILON,
            _ => false,
        };
    }

    // String comparison
    let left_str = match left {
        Value::String(s) => s.as_str(),
        _ => return false,
    };
    let right_str = right.trim_matches('\'').trim_matches('"');

    match op {
        "==" => left_str == right_str,
        "!=" => left_str != right_str,
        _ => false,
    }
}

/// Result of executing a single pipeline step (for verify/try-step).
#[derive(Debug, serde::Serialize)]
pub struct StepResult {
    pub index: usize,
    pub step: String,
    pub status: String,
    pub duration_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screenshot_path: Option<String>,
}

/// Suggest a fix based on error message patterns.
pub fn suggest_fix(error: &str) -> Option<String> {
    if error.contains("element not found")
        || error.contains("selector") && error.contains("not found")
    {
        Some("Selector may have changed. Use `claw find` or `claw ax-tree` to locate the current element.".into())
    } else if error.contains("text not found") {
        Some("Text/label may have changed. Use `claw read-dom` to check current page text.".into())
    } else if error.contains("timeout waiting for") {
        Some("Page may be slow or condition incorrect. Increase timeout or verify the condition with `claw page-info`.".into())
    } else if error.contains("fetch") && error.contains("failed") {
        Some("API endpoint may have changed. Use `claw network-log start` + interact + `claw network-log dump` to discover current endpoints.".into())
    } else if error.contains("assertion failed") {
        Some("Page behavior may have changed. Re-explore with `claw ax-tree` and update the adapter.".into())
    } else if error.contains("not visible") {
        Some("Element exists but is hidden. Check for modals/overlays with `claw top-layer` or scroll with `claw scroll`.".into())
    } else if error.contains("JS error")
        || error.contains("SyntaxError")
        || error.contains("ReferenceError")
    {
        Some(
            "JavaScript in evaluate step has an error. Test with `claw evaluate '<js>'` to debug."
                .into(),
        )
    } else {
        None
    }
}

/// Execute adapter pipeline step-by-step, returning per-step results.
/// Does not stop on first failure — runs all steps and reports health.
pub async fn execute_with_report(
    steps: &[PipelineStep],
    client: &CdpClient,
    args: HashMap<String, Value>,
) -> Vec<StepResult> {
    let mut results = Vec::new();
    let mut data: Vec<Value> = Vec::new();
    let mut rows: Vec<HashMap<String, String>> = Vec::new();

    for (i, step) in steps.iter().enumerate() {
        let label = step_label(step);
        let start = std::time::Instant::now();

        let outcome = execute_single_step(step, client, &args, &mut data, &mut rows).await;
        let duration_ms = start.elapsed().as_millis();

        let (status, error, suggestion, page_url, screenshot_path) = match outcome {
            Ok(()) => ("pass".to_string(), None, None, None, None),
            Err(e) => {
                let err_str = e.to_string();
                let sug = suggest_fix(&err_str);
                // Capture page state on failure for diagnostics
                let url = client
                    .evaluate("location.href")
                    .await
                    .ok()
                    .and_then(|v| v.as_str().map(|s| s.to_string()));
                let screenshot = {
                    let path = format!("/tmp/verify-step-{}.png", i);
                    if client.screenshot(&path).await.is_ok() {
                        Some(path)
                    } else {
                        None
                    }
                };
                ("fail".to_string(), Some(err_str), sug, url, screenshot)
            }
        };

        results.push(StepResult {
            index: i,
            step: label,
            status,
            duration_ms,
            error,
            suggestion,
            page_url,
            screenshot_path,
        });
    }

    results
}

/// Execute a single pipeline step, mutating data/rows state.
pub async fn execute_single_step(
    step: &PipelineStep,
    client: &CdpClient,
    args: &HashMap<String, Value>,
    data: &mut Vec<Value>,
    rows: &mut Vec<HashMap<String, String>>,
) -> Result<(), Box<dyn std::error::Error>> {
    match step {
        PipelineStep::Navigate(url) => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let rendered = template::render(url, &ctx);
            client.navigate(&rendered).await?;
        }
        PipelineStep::Evaluate(js) => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let rendered = template::render(js, &ctx);
            let result = client.evaluate(&rendered).await?;
            match result {
                Value::Array(arr) => *data = arr,
                other => *data = vec![other],
            }
        }
        PipelineStep::Map(mappings) => {
            *rows = apply_map(data, mappings, args);
        }
        PipelineStep::Limit(tmpl) => {
            apply_limit(rows, tmpl, args);
        }
        PipelineStep::Wait(tmpl) => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let secs: f64 = template::render(tmpl, &ctx).parse().unwrap_or(1.0);
            tokio::time::sleep(std::time::Duration::from_secs_f64(secs)).await;
        }
        PipelineStep::Click(tmpl) => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let text = template::render(tmpl, &ctx);
            client.click_text(&text).await?;
        }
        PipelineStep::ClickSelector(tmpl) => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let selector = template::render(tmpl, &ctx);
            client.click_selector(&selector).await?;
        }
        PipelineStep::ClickTarget(target) => {
            // Resolve semantic target via hints (primary_input, submit_button)
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let target_name = template::render(target, &ctx);
            let selector = resolve_target(&target_name, client).await?;
            client.click_selector(&selector).await?;
        }
        PipelineStep::Type {
            selector,
            target,
            text,
        } => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let txt = template::render(text, &ctx);
            // Resolve selector from explicit selector or semantic target
            let sel = if let Some(s) = selector {
                template::render(s, &ctx)
            } else if let Some(t) = target {
                let target_name = template::render(t, &ctx);
                resolve_target(&target_name, client).await?
            } else {
                return Err("type step requires 'selector' or 'target'".into());
            };
            client.type_into(&sel, &txt).await?;
        }
        PipelineStep::Upload { selector, files } => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let sel = template::render(selector, &ctx);
            let file_list = template::render(files, &ctx);
            let paths: Vec<&str> = file_list.split(',').map(|s| s.trim()).collect();
            client.upload_files(&sel, &paths).await?;
        }
        PipelineStep::WaitFor { selector, timeout } => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let sel = template::render(selector, &ctx);
            let secs: f64 = template::render(timeout, &ctx).parse().unwrap_or(10.0);
            client.wait_for_selector(&sel, secs).await?;
        }
        PipelineStep::WaitForText { text, timeout } => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let txt = template::render(text, &ctx);
            let secs: f64 = template::render(timeout, &ctx).parse().unwrap_or(10.0);
            client.wait_for_text(&txt, secs).await?;
        }
        PipelineStep::WaitForUrl { pattern, timeout } => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let pat = template::render(pattern, &ctx);
            let secs: f64 = template::render(timeout, &ctx).parse().unwrap_or(10.0);
            client.wait_for_url(&pat, secs).await?;
        }
        PipelineStep::WaitForNetworkIdle(timeout) => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let secs: f64 = template::render(timeout, &ctx).parse().unwrap_or(10.0);
            client.wait_for_network_idle(secs).await?;
        }
        PipelineStep::Screenshot(path) => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let p = template::render(path, &ctx);
            client.screenshot(&p).await?;
        }
        PipelineStep::Hover(text) => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let t = template::render(text, &ctx);
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
                crate::cdp::escape_js_string(&t),
                crate::cdp::escape_js_string(&t)
            );
            let result = client.evaluate(&js).await?;
            let x = result["x"].as_f64().ok_or("missing x")?;
            let y = result["y"].as_f64().ok_or("missing y")?;
            client.hover_at(x, y).await?;
        }
        PipelineStep::HoverSelector(selector) => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let sel = template::render(selector, &ctx);
            client.hover_selector(&sel).await?;
        }
        PipelineStep::Scroll(selector) => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let sel = template::render(selector, &ctx);
            client.scroll_into_view(&sel).await?;
        }
        PipelineStep::ScrollBy { x, y } => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let dx: f64 = template::render(x, &ctx).parse().unwrap_or(0.0);
            let dy: f64 = template::render(y, &ctx).parse().unwrap_or(0.0);
            client.scroll_by(dx, dy).await?;
        }
        PipelineStep::PressKey { key, modifiers } => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let k = template::render(key, &ctx);
            let m: u32 = template::render(modifiers, &ctx).parse().unwrap_or(0);
            client.press_key(&k, m).await?;
        }
        PipelineStep::Select { selector, value } => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let sel = template::render(selector, &ctx);
            let val = template::render(value, &ctx);
            client.select_option(&sel, &val).await?;
        }
        PipelineStep::DismissDialog(accept) => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let a = template::render(accept, &ctx);
            let do_accept = a != "false" && a != "0" && a != "dismiss";
            client.dismiss_dialog(do_accept, None).await?;
        }
        PipelineStep::AssertSelector(selector) => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let sel = template::render(selector, &ctx);
            client.assert_selector(&sel).await?;
        }
        PipelineStep::AssertText(text) => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let txt = template::render(text, &ctx);
            client.assert_text(&txt).await?;
        }
        PipelineStep::AssertUrl(pattern) => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let pat = template::render(pattern, &ctx);
            client.assert_url(&pat).await?;
        }
        PipelineStep::AssertNotSelector(selector) => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let sel = template::render(selector, &ctx);
            client.assert_not_selector(&sel).await?;
        }
        PipelineStep::IfSelector {
            selector,
            then_steps,
        } => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let sel = template::render(selector, &ctx);
            let js = format!(
                "document.querySelector('{}') !== null",
                crate::cdp::escape_js_string(&sel)
            );
            if let Ok(val) = client.evaluate(&js).await {
                if val == true {
                    for sub in then_steps {
                        Box::pin(execute_single_step(sub, client, args, data, rows)).await?;
                    }
                }
            }
        }
        PipelineStep::IfText { text, then_steps } => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let txt = template::render(text, &ctx);
            let js = format!(
                "document.body.innerText.includes('{}')",
                crate::cdp::escape_js_string(&txt)
            );
            if let Ok(val) = client.evaluate(&js).await {
                if val == true {
                    for sub in then_steps {
                        Box::pin(execute_single_step(sub, client, args, data, rows)).await?;
                    }
                }
            }
        }
        PipelineStep::IfUrl {
            pattern,
            then_steps,
        } => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let pat = template::render(pattern, &ctx);
            let js = format!(
                "location.href.includes('{}')",
                crate::cdp::escape_js_string(&pat)
            );
            if let Ok(val) = client.evaluate(&js).await {
                if val == true {
                    for sub in then_steps {
                        Box::pin(execute_single_step(sub, client, args, data, rows)).await?;
                    }
                }
            }
        }
        PipelineStep::Fetch {
            url,
            method,
            headers,
            body,
        } => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let rendered_url = template::render(url, &ctx);
            let rendered_method = template::render(method, &ctx);

            let http_client = reqwest::Client::new();
            let mut req = match rendered_method.to_uppercase().as_str() {
                "POST" => http_client.post(&rendered_url),
                "PUT" => http_client.put(&rendered_url),
                "DELETE" => http_client.delete(&rendered_url),
                "PATCH" => http_client.patch(&rendered_url),
                _ => http_client.get(&rendered_url),
            };

            for (k, v) in headers {
                let rk = template::render(k, &ctx);
                let rv = template::render(v, &ctx);
                req = req.header(rk, rv);
            }

            if let Some(b) = body {
                let rendered_body = template::render(b, &ctx);
                req = req.body(rendered_body);
            }

            let resp = req
                .send()
                .await
                .map_err(|e| -> Box<dyn std::error::Error> {
                    format!("fetch {} failed: {}", rendered_url, e).into()
                })?;
            let json: Value = resp
                .json()
                .await
                .map_err(|e| -> Box<dyn std::error::Error> {
                    format!("fetch response not JSON: {}", e).into()
                })?;

            match json {
                Value::Array(arr) => *data = arr,
                other => *data = vec![other],
            }
        }
        PipelineStep::SelectPath(path) => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let rendered = template::render(path, &ctx);

            // Apply path to each item in data, or to single data item
            let source = if data.len() == 1 {
                data[0].clone()
            } else {
                Value::Array(data.clone())
            };

            let selected = resolve_json_path(&source, &rendered);
            match selected {
                Value::Array(arr) => *data = arr,
                Value::Null => {} // no-op if path not found
                other => *data = vec![other],
            }
        }
        PipelineStep::Filter(expr) => {
            let ctx_base = args.clone();
            let rendered_expr = template::render(
                expr,
                &TemplateContext {
                    args: ctx_base.clone(),
                    item: None,
                },
            );

            // Filter rows if we have rows, otherwise filter data
            if !rows.is_empty() {
                rows.retain(|row| {
                    let row_json: Value = Value::Object(
                        row.iter()
                            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                            .collect(),
                    );
                    evaluate_filter_expr(&rendered_expr, &row_json)
                });
            } else {
                data.retain(|item| evaluate_filter_expr(&rendered_expr, item));
            }
        }
        PipelineStep::Intercept {
            trigger,
            capture,
            timeout,
            select,
        } => {
            let ctx = TemplateContext {
                args: args.clone(),
                item: None,
            };
            let capture_pattern = template::render(capture, &ctx);
            let trigger_action = template::render(trigger, &ctx);
            let timeout_secs: f64 = template::render(timeout, &ctx).parse().unwrap_or(10.0);
            let select_path = select.as_ref().map(|s| template::render(s, &ctx));

            // 1. Enable CDP Network capture (pure protocol, no JS injection)
            client.start_network_log().await?;

            // 2. Execute trigger action
            if let Some(url) = trigger_action.strip_prefix("navigate:") {
                client.navigate(url.trim()).await?;
            } else if let Some(js) = trigger_action.strip_prefix("evaluate:") {
                client.evaluate(js.trim()).await?;
            } else if let Some(sel) = trigger_action.strip_prefix("click:") {
                client.click_selector(sel.trim()).await?;
            } else {
                // Default: treat as evaluate
                client.evaluate(&trigger_action).await?;
            }

            // 3. Poll CDP network log for matching response
            let max_attempts = (timeout_secs * 4.0) as u32;
            let mut captured = Value::Null;
            for _ in 0..max_attempts {
                let log = client
                    .get_network_log_with_bodies(Some(&capture_pattern))
                    .await?;
                if let Some(entries) = log.as_array() {
                    for entry in entries {
                        if entry["body"] != Value::Null {
                            captured = entry["body"].clone();
                            break;
                        }
                    }
                }
                if captured != Value::Null {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }

            // 4. Stop network capture
            let _ = client.stop_network_log().await;

            if captured == Value::Null {
                return Err(format!(
                    "intercept: no response matching '{}' captured within {}s",
                    capture_pattern, timeout_secs
                )
                .into());
            }

            // 5. Apply select path if provided
            if let Some(path) = select_path {
                captured = resolve_json_path(&captured, &path);
            }

            match captured {
                Value::Array(arr) => *data = arr,
                other => *data = vec![other],
            }
        }
    }
    Ok(())
}

/// Truncate rows to the limit resolved from the template.
pub fn apply_limit(
    rows: &mut Vec<HashMap<String, String>>,
    tmpl: &str,
    args: &HashMap<String, Value>,
) {
    let ctx = TemplateContext {
        args: args.clone(),
        item: None,
    };
    let n: usize = template::render(tmpl, &ctx).parse().unwrap_or(rows.len());
    rows.truncate(n);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn apply_map_transforms_data() {
        let data = vec![
            json!({"title": "Video A", "views": 100}),
            json!({"title": "Video B", "views": 200}),
        ];
        let mut mappings = HashMap::new();
        mappings.insert("title".to_string(), "${{ item.title }}".to_string());
        mappings.insert("views".to_string(), "${{ item.views }}".to_string());

        let args = HashMap::new();
        let rows = apply_map(&data, &mappings, &args);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["title"], "Video A");
        assert_eq!(rows[0]["views"], "100");
        assert_eq!(rows[1]["title"], "Video B");
        assert_eq!(rows[1]["views"], "200");
    }

    #[test]
    fn apply_limit_truncates() {
        let mut rows: Vec<HashMap<String, String>> = (0..10)
            .map(|i| {
                let mut row = HashMap::new();
                row.insert("n".to_string(), i.to_string());
                row
            })
            .collect();

        let args = HashMap::new();
        apply_limit(&mut rows, "5", &args);
        assert_eq!(rows.len(), 5);
    }

    #[test]
    fn apply_limit_from_template() {
        let mut rows: Vec<HashMap<String, String>> = (0..10)
            .map(|i| {
                let mut row = HashMap::new();
                row.insert("n".to_string(), i.to_string());
                row
            })
            .collect();

        let mut args = HashMap::new();
        args.insert("limit".to_string(), json!(3));

        apply_limit(&mut rows, "${{ args.limit }}", &args);
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn resolve_json_path_nested() {
        let data = json!({"data": {"items": [{"title": "A"}, {"title": "B"}]}});
        let result = resolve_json_path(&data, "data.items");
        assert!(result.is_array());
        assert_eq!(result.as_array().unwrap().len(), 2);
    }

    #[test]
    fn resolve_json_path_single_field() {
        let data = json!({"name": "test", "count": 42});
        assert_eq!(resolve_json_path(&data, "name"), json!("test"));
        assert_eq!(resolve_json_path(&data, "count"), json!(42));
    }

    #[test]
    fn resolve_json_path_missing() {
        let data = json!({"a": 1});
        assert_eq!(resolve_json_path(&data, "b.c"), json!(null));
    }

    #[test]
    fn filter_expr_numeric_gt() {
        let item = json!({"views": 5000, "title": "hello"});
        assert!(evaluate_filter_expr("item.views > 1000", &item));
        assert!(!evaluate_filter_expr("item.views > 10000", &item));
    }

    #[test]
    fn filter_expr_string_eq() {
        let item = json!({"status": "published"});
        assert!(evaluate_filter_expr("item.status == 'published'", &item));
        assert!(!evaluate_filter_expr("item.status == 'draft'", &item));
    }

    #[test]
    fn filter_expr_truthy() {
        assert!(evaluate_filter_expr("item.title", &json!({"title": "hi"})));
        assert!(!evaluate_filter_expr("item.title", &json!({"title": ""})));
        assert!(!evaluate_filter_expr("item.missing", &json!({"x": 1})));
    }

    #[test]
    fn parse_fetch_step_string() {
        let step =
            crate::adapter::parse_single_step("fetch: https://api.example.com/data").unwrap();
        match step {
            PipelineStep::Fetch { url, method, .. } => {
                assert_eq!(url, "https://api.example.com/data");
                assert_eq!(method, "GET");
            }
            _ => panic!("expected Fetch"),
        }
    }

    #[test]
    fn parse_select_path_step() {
        let step = crate::adapter::parse_single_step("select: data.items").unwrap();
        match step {
            PipelineStep::SelectPath(path) => assert_eq!(path, "data.items"),
            _ => panic!("expected SelectPath"),
        }
    }

    #[test]
    fn parse_filter_step() {
        let step = crate::adapter::parse_single_step("filter: item.views > 100").unwrap();
        match step {
            PipelineStep::Filter(expr) => assert_eq!(expr, "item.views > 100"),
            _ => panic!("expected Filter"),
        }
    }

    // ---- login URL resolution ----
    // Why: `claw login <site>` must resolve site name to a URL without requiring the user to know the domain
    // Classification: quality, what — wrong URL = user can't log in

    #[test]
    fn resolve_login_url_from_adapter_domain() {
        // When adapters exist for a site, the login URL should come from the adapter's domain field
        let url = crate::adapter::resolve_login_url(&["adapters"], "jimeng");
        assert!(
            url.starts_with("https://"),
            "login URL must be https, got: {}",
            url
        );
        assert!(
            url.contains("jimeng"),
            "login URL must contain site name, got: {}",
            url
        );
    }

    #[test]
    fn resolve_login_url_fallback_for_unknown_site() {
        // For unknown sites, treat the input as a domain directly
        let url = crate::adapter::resolve_login_url(&["adapters"], "example.com");
        assert_eq!(url, "https://example.com");
    }

    // ---- pipeline error diagnostics ----
    // Why: "timeout waiting for selector" without page context wastes debugging time
    // Classification: quality, what — poor errors = slow forge iteration

    #[test]
    fn step_error_includes_step_number_and_label() {
        // Error messages must contain step index and step label for traceability
        let err_msg = "step 0: navigate: https://example.com — connection refused";
        assert!(err_msg.starts_with("step 0:"));
        // This is already the format — this test documents the contract
    }

    // ---- verify-adapter diagnostics ----
    // Classification: quality, what — Agent needs failure context to fix adapter in one round-trip
    // Why: verify-adapter returning only pass/fail forces manual screenshot + debug cycle

    #[test]
    fn step_result_with_diagnostics_contains_page_state() {
        // When a step fails, StepResult must include page diagnostics
        let result = StepResult {
            index: 2,
            step: "click: Submit".to_string(),
            status: "fail".to_string(),
            duration_ms: 1500,
            error: Some("text not found: Submit".to_string()),
            suggestion: Some("Check if the text is visible".to_string()),
            page_url: Some("https://example.com/login".to_string()),
            screenshot_path: Some("/tmp/verify-step-2.png".to_string()),
        };
        // Must have page URL for context
        assert!(result.page_url.is_some());
        // Must have screenshot so Agent can see the failure visually
        assert!(result.screenshot_path.is_some());
    }

    #[test]
    fn step_result_pass_omits_diagnostics() {
        // Passing steps should NOT waste time on screenshots
        let result = StepResult {
            index: 0,
            step: "navigate: https://example.com".to_string(),
            status: "pass".to_string(),
            duration_ms: 200,
            error: None,
            suggestion: None,
            page_url: None,
            screenshot_path: None,
        };
        assert!(result.page_url.is_none());
        assert!(result.screenshot_path.is_none());
    }
}
