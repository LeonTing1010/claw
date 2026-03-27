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

    for step in steps {
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
                    Value::Array(arr) => data = arr,
                    other => data = vec![other],
                }
            }
            PipelineStep::Map(mappings) => {
                rows = apply_map(&data, mappings, &args);
            }
            PipelineStep::Limit(tmpl) => {
                apply_limit(&mut rows, tmpl, &args);
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
            PipelineStep::Type { selector, text } => {
                let ctx = TemplateContext {
                    args: args.clone(),
                    item: None,
                };
                let sel = template::render(selector, &ctx);
                let txt = template::render(text, &ctx);
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
                client.click_text(&t).await.ok(); // resolve coords
                                                  // Actually hover by text - find the element then hover
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
        }
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
}
