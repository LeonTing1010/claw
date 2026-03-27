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
