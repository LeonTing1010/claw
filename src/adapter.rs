use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// Represents a parsed YAML adapter file.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct Adapter {
    pub site: String,
    pub name: String,
    pub description: Option<String>,
    pub domain: Option<String>,
    pub strategy: Option<String>,
    pub browser: Option<bool>,
    pub args: Option<HashMap<String, ArgDef>>,
    pub columns: Vec<String>,
    #[serde(deserialize_with = "deserialize_pipeline")]
    pub pipeline: Vec<PipelineStep>,
}

/// Defines an argument with an optional type and default value.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ArgDef {
    #[serde(rename = "type")]
    pub arg_type: Option<String>,
    pub default: Option<serde_json::Value>,
}

/// A single step in the adapter pipeline.
#[derive(Debug)]
pub enum PipelineStep {
    Navigate(String),
    Evaluate(String),
    Map(HashMap<String, String>),
    Limit(String),
    Wait(String),
    Click(String),
    ClickSelector(String),
    Type { selector: String, text: String },
    Upload { selector: String, files: String },
    WaitFor { selector: String, timeout: String },
    WaitForText { text: String, timeout: String },
    WaitForUrl { pattern: String, timeout: String },
    WaitForNetworkIdle(String),
    Screenshot(String),
    Hover(String),
    HoverSelector(String),
    Scroll(String),
    ScrollBy { x: String, y: String },
    PressKey { key: String, modifiers: String },
    Select { selector: String, value: String },
    DismissDialog(String),
    AssertSelector(String),
    AssertText(String),
    AssertUrl(String),
    AssertNotSelector(String),
}

/// Convert a `serde_yml::Value` to a `String`, handling both string values
/// and other scalar types.
fn yml_value_to_string(v: &serde_yml::Value) -> Option<String> {
    match v {
        serde_yml::Value::String(s) => Some(s.clone()),
        serde_yml::Value::Number(n) => Some(n.to_string()),
        serde_yml::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Custom deserializer for the pipeline field. Each element is a single-key
/// map whose key determines the step variant.
fn deserialize_pipeline<'de, D>(deserializer: D) -> Result<Vec<PipelineStep>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: Vec<HashMap<String, serde_yml::Value>> = Vec::deserialize(deserializer)?;

    let mut steps = Vec::with_capacity(raw.len());
    for map in raw {
        if map.len() != 1 {
            return Err(serde::de::Error::custom(
                "each pipeline step must be a single-key map",
            ));
        }
        let (key, value) = map.into_iter().next().unwrap();
        let step = match key.as_str() {
            "navigate" => {
                let s = yml_value_to_string(&value)
                    .ok_or_else(|| serde::de::Error::custom("navigate value must be a string"))?;
                PipelineStep::Navigate(s)
            }
            "evaluate" => {
                let s = yml_value_to_string(&value)
                    .ok_or_else(|| serde::de::Error::custom("evaluate value must be a string"))?;
                PipelineStep::Evaluate(s)
            }
            "map" => {
                let mapping = match value {
                    serde_yml::Value::Mapping(m) => m,
                    _ => {
                        return Err(serde::de::Error::custom("map value must be a mapping"));
                    }
                };
                let mut hm = HashMap::new();
                for (k, v) in mapping {
                    let key_str = yml_value_to_string(&k)
                        .ok_or_else(|| serde::de::Error::custom("map key must be a string"))?;
                    let val_str = yml_value_to_string(&v)
                        .ok_or_else(|| serde::de::Error::custom("map value must be a string"))?;
                    hm.insert(key_str, val_str);
                }
                PipelineStep::Map(hm)
            }
            "limit" => {
                let s = yml_value_to_string(&value)
                    .ok_or_else(|| serde::de::Error::custom("limit value must be a string"))?;
                PipelineStep::Limit(s)
            }
            "wait" => {
                let s = yml_value_to_string(&value).ok_or_else(|| {
                    serde::de::Error::custom("wait value must be a string or number")
                })?;
                PipelineStep::Wait(s)
            }
            "click" => {
                let s = yml_value_to_string(&value)
                    .ok_or_else(|| serde::de::Error::custom("click value must be a string"))?;
                PipelineStep::Click(s)
            }
            "click_selector" => {
                let s = yml_value_to_string(&value).ok_or_else(|| {
                    serde::de::Error::custom("click_selector value must be a string")
                })?;
                PipelineStep::ClickSelector(s)
            }
            "type" => {
                let mapping = match &value {
                    serde_yml::Value::Mapping(m) => m,
                    _ => {
                        return Err(serde::de::Error::custom(
                            "type value must be a mapping with selector and text",
                        ))
                    }
                };
                let selector = mapping
                    .get(serde_yml::Value::String("selector".into()))
                    .and_then(yml_value_to_string)
                    .ok_or_else(|| {
                        serde::de::Error::custom("type step requires 'selector' field")
                    })?;
                let text = mapping
                    .get(serde_yml::Value::String("text".into()))
                    .and_then(yml_value_to_string)
                    .ok_or_else(|| serde::de::Error::custom("type step requires 'text' field"))?;
                PipelineStep::Type { selector, text }
            }
            "upload" => {
                let mapping = match &value {
                    serde_yml::Value::Mapping(m) => m,
                    _ => {
                        return Err(serde::de::Error::custom(
                            "upload value must be a mapping with selector and files",
                        ))
                    }
                };
                let selector = mapping
                    .get(serde_yml::Value::String("selector".into()))
                    .and_then(yml_value_to_string)
                    .ok_or_else(|| {
                        serde::de::Error::custom("upload step requires 'selector' field")
                    })?;
                let files = mapping
                    .get(serde_yml::Value::String("files".into()))
                    .and_then(yml_value_to_string)
                    .ok_or_else(|| {
                        serde::de::Error::custom("upload step requires 'files' field")
                    })?;
                PipelineStep::Upload { selector, files }
            }
            "wait_for" => {
                let mapping = match &value {
                    serde_yml::Value::Mapping(m) => m,
                    _ => {
                        return Err(serde::de::Error::custom(
                            "wait_for value must be a mapping with selector and timeout",
                        ))
                    }
                };
                let selector = mapping
                    .get(serde_yml::Value::String("selector".into()))
                    .and_then(yml_value_to_string)
                    .ok_or_else(|| {
                        serde::de::Error::custom("wait_for step requires 'selector' field")
                    })?;
                let timeout = mapping
                    .get(serde_yml::Value::String("timeout".into()))
                    .and_then(yml_value_to_string)
                    .unwrap_or_else(|| "10".to_string());
                PipelineStep::WaitFor { selector, timeout }
            }
            "screenshot" => {
                let s = yml_value_to_string(&value)
                    .ok_or_else(|| serde::de::Error::custom("screenshot value must be a string"))?;
                PipelineStep::Screenshot(s)
            }
            "hover" => {
                let s = yml_value_to_string(&value)
                    .ok_or_else(|| serde::de::Error::custom("hover value must be a string"))?;
                PipelineStep::Hover(s)
            }
            "hover_selector" => {
                let s = yml_value_to_string(&value).ok_or_else(|| {
                    serde::de::Error::custom("hover_selector value must be a string")
                })?;
                PipelineStep::HoverSelector(s)
            }
            "scroll" => {
                let s = yml_value_to_string(&value)
                    .ok_or_else(|| serde::de::Error::custom("scroll value must be a string"))?;
                PipelineStep::Scroll(s)
            }
            "scroll_by" => {
                let mapping = match &value {
                    serde_yml::Value::Mapping(m) => m,
                    _ => {
                        return Err(serde::de::Error::custom(
                            "scroll_by value must be a mapping with x and y",
                        ))
                    }
                };
                let x = mapping
                    .get(serde_yml::Value::String("x".into()))
                    .and_then(yml_value_to_string)
                    .unwrap_or_else(|| "0".to_string());
                let y = mapping
                    .get(serde_yml::Value::String("y".into()))
                    .and_then(yml_value_to_string)
                    .unwrap_or_else(|| "0".to_string());
                PipelineStep::ScrollBy { x, y }
            }
            "press_key" => match &value {
                serde_yml::Value::Mapping(m) => {
                    let key = m
                        .get(serde_yml::Value::String("key".into()))
                        .and_then(yml_value_to_string)
                        .ok_or_else(|| {
                            serde::de::Error::custom("press_key step requires 'key' field")
                        })?;
                    let modifiers = m
                        .get(serde_yml::Value::String("modifiers".into()))
                        .and_then(yml_value_to_string)
                        .unwrap_or_else(|| "0".to_string());
                    PipelineStep::PressKey { key, modifiers }
                }
                _ => {
                    let s = yml_value_to_string(&value).ok_or_else(|| {
                        serde::de::Error::custom("press_key value must be a string or mapping")
                    })?;
                    PipelineStep::PressKey {
                        key: s,
                        modifiers: "0".to_string(),
                    }
                }
            },
            "select" => {
                let mapping = match &value {
                    serde_yml::Value::Mapping(m) => m,
                    _ => {
                        return Err(serde::de::Error::custom(
                            "select value must be a mapping with selector and value",
                        ))
                    }
                };
                let selector = mapping
                    .get(serde_yml::Value::String("selector".into()))
                    .and_then(yml_value_to_string)
                    .ok_or_else(|| {
                        serde::de::Error::custom("select step requires 'selector' field")
                    })?;
                let val = mapping
                    .get(serde_yml::Value::String("value".into()))
                    .and_then(yml_value_to_string)
                    .ok_or_else(|| {
                        serde::de::Error::custom("select step requires 'value' field")
                    })?;
                PipelineStep::Select {
                    selector,
                    value: val,
                }
            }
            "dismiss_dialog" => {
                let s = yml_value_to_string(&value).unwrap_or_else(|| "true".to_string());
                PipelineStep::DismissDialog(s)
            }
            "wait_for_text" => match &value {
                serde_yml::Value::Mapping(m) => {
                    let text = m
                        .get(serde_yml::Value::String("text".into()))
                        .and_then(yml_value_to_string)
                        .ok_or_else(|| {
                            serde::de::Error::custom("wait_for_text step requires 'text' field")
                        })?;
                    let timeout = m
                        .get(serde_yml::Value::String("timeout".into()))
                        .and_then(yml_value_to_string)
                        .unwrap_or_else(|| "10".to_string());
                    PipelineStep::WaitForText { text, timeout }
                }
                _ => {
                    let s = yml_value_to_string(&value).ok_or_else(|| {
                        serde::de::Error::custom("wait_for_text value must be a string or mapping")
                    })?;
                    PipelineStep::WaitForText {
                        text: s,
                        timeout: "10".to_string(),
                    }
                }
            },
            "wait_for_url" => match &value {
                serde_yml::Value::Mapping(m) => {
                    let pattern = m
                        .get(serde_yml::Value::String("pattern".into()))
                        .and_then(yml_value_to_string)
                        .ok_or_else(|| {
                            serde::de::Error::custom("wait_for_url step requires 'pattern' field")
                        })?;
                    let timeout = m
                        .get(serde_yml::Value::String("timeout".into()))
                        .and_then(yml_value_to_string)
                        .unwrap_or_else(|| "10".to_string());
                    PipelineStep::WaitForUrl { pattern, timeout }
                }
                _ => {
                    let s = yml_value_to_string(&value).ok_or_else(|| {
                        serde::de::Error::custom("wait_for_url value must be a string or mapping")
                    })?;
                    PipelineStep::WaitForUrl {
                        pattern: s,
                        timeout: "10".to_string(),
                    }
                }
            },
            "wait_for_network_idle" => {
                let s = yml_value_to_string(&value).unwrap_or_else(|| "10".to_string());
                PipelineStep::WaitForNetworkIdle(s)
            }
            "assert_selector" => {
                let s = yml_value_to_string(&value).ok_or_else(|| {
                    serde::de::Error::custom("assert_selector value must be a string")
                })?;
                PipelineStep::AssertSelector(s)
            }
            "assert_text" => {
                let s = yml_value_to_string(&value).ok_or_else(|| {
                    serde::de::Error::custom("assert_text value must be a string")
                })?;
                PipelineStep::AssertText(s)
            }
            "assert_url" => {
                let s = yml_value_to_string(&value)
                    .ok_or_else(|| serde::de::Error::custom("assert_url value must be a string"))?;
                PipelineStep::AssertUrl(s)
            }
            "assert_not_selector" => {
                let s = yml_value_to_string(&value).ok_or_else(|| {
                    serde::de::Error::custom("assert_not_selector value must be a string")
                })?;
                PipelineStep::AssertNotSelector(s)
            }
            other => {
                return Err(serde::de::Error::custom(format!(
                    "unknown pipeline step: {}",
                    other
                )));
            }
        };
        steps.push(step);
    }
    Ok(steps)
}

/// Load an adapter by searching through the given base directories for
/// `{base_dir}/{site}/{name}.yaml`. Returns the first match found.
pub fn load_adapter(
    base_dirs: &[&str],
    site: &str,
    name: &str,
) -> Result<Adapter, Box<dyn std::error::Error>> {
    for base in base_dirs {
        let path = Path::new(base).join(site).join(format!("{}.yaml", name));
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            let adapter: Adapter = serde_yml::from_str(&content)?;
            return Ok(adapter);
        }
    }
    Err(format!(
        "adapter not found: {}/{} (searched: {:?})",
        site, name, base_dirs
    )
    .into())
}

#[derive(Debug)]
pub struct AdapterInfo {
    pub site: String,
    pub name: String,
    pub description: String,
}

/// Scan adapter directories for .yaml files and return metadata.
pub fn list_adapters(base_dirs: &[&str]) -> Vec<AdapterInfo> {
    let mut adapters = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for base in base_dirs {
        let base_path = std::path::Path::new(base);
        let Ok(sites) = std::fs::read_dir(base_path) else {
            continue;
        };

        for site_entry in sites.flatten() {
            if !site_entry.path().is_dir() {
                continue;
            }
            let site_name = site_entry.file_name().to_string_lossy().to_string();

            let Ok(files) = std::fs::read_dir(site_entry.path()) else {
                continue;
            };
            for file_entry in files.flatten() {
                let path = file_entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
                    continue;
                }

                let adapter_name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();

                let key = format!("{}/{}", site_name, adapter_name);
                if seen.contains(&key) {
                    continue;
                }
                seen.insert(key);

                // Try to parse for description
                let description = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|content| serde_yml::from_str::<Adapter>(&content).ok())
                    .and_then(|a| a.description)
                    .unwrap_or_default();

                adapters.push(AdapterInfo {
                    site: site_name.clone(),
                    name: adapter_name,
                    description,
                });
            }
        }
    }
    adapters.sort_by(|a, b| (&a.site, &a.name).cmp(&(&b.site, &b.name)));
    adapters
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const BILIBILI_HOT_YAML: &str = r#"
site: bilibili
name: hot
description: B站热门视频
domain: bilibili.com
strategy: cookie
browser: true

args:
  limit:
    type: int
    default: 10

columns: [title, author, views, url]

pipeline:
  - navigate: https://bilibili.com
  - evaluate: |
      (async () => {
        const res = await fetch('/x/web-interface/ranking/v2', { credentials: 'include' });
        const data = await res.json();
        return data.data.list.map(v => ({
          title: v.title, author: v.owner.name,
          views: v.stat.view, url: 'https://bilibili.com/video/' + v.bvid
        }));
      })()
  - map:
      title: ${{ item.title }}
      author: ${{ item.author }}
      views: ${{ item.views }}
      url: ${{ item.url }}
  - limit: ${{ args.limit }}
"#;

    #[test]
    fn parse_bilibili_hot_yaml() {
        let adapter: Adapter = serde_yml::from_str(BILIBILI_HOT_YAML).unwrap();

        assert_eq!(adapter.site, "bilibili");
        assert_eq!(adapter.name, "hot");
        assert_eq!(adapter.description, Some("B站热门视频".to_string()));
        assert_eq!(adapter.domain, Some("bilibili.com".to_string()));
        assert_eq!(adapter.strategy, Some("cookie".to_string()));
        assert_eq!(adapter.browser, Some(true));
        assert_eq!(adapter.columns.len(), 4);
        assert_eq!(adapter.columns, vec!["title", "author", "views", "url"]);
        assert_eq!(adapter.pipeline.len(), 4);

        // First step should be Navigate
        match &adapter.pipeline[0] {
            PipelineStep::Navigate(url) => {
                assert_eq!(url, "https://bilibili.com");
            }
            _ => panic!("expected Navigate as first pipeline step"),
        }
    }

    #[test]
    fn parse_args_with_defaults() {
        let adapter: Adapter = serde_yml::from_str(BILIBILI_HOT_YAML).unwrap();

        let args = adapter.args.as_ref().expect("args should be present");
        let limit = args.get("limit").expect("limit arg should exist");

        assert_eq!(limit.arg_type, Some("int".to_string()));
        assert_eq!(limit.default, Some(Value::Number(10.into())));
    }

    #[test]
    fn parse_map_step() {
        let adapter: Adapter = serde_yml::from_str(BILIBILI_HOT_YAML).unwrap();

        // The map step is the third pipeline step (index 2)
        match &adapter.pipeline[2] {
            PipelineStep::Map(map) => {
                assert_eq!(map.get("title"), Some(&"${{ item.title }}".to_string()));
                assert_eq!(map.get("author"), Some(&"${{ item.author }}".to_string()));
                assert_eq!(map.get("views"), Some(&"${{ item.views }}".to_string()));
                assert_eq!(map.get("url"), Some(&"${{ item.url }}".to_string()));
            }
            _ => panic!("expected Map as third pipeline step"),
        }
    }

    #[test]
    fn parse_wait_step() {
        let yaml = r#"
site: test
name: test
columns: [status]
pipeline:
  - wait: 2.5
"#;
        let adapter: Adapter = serde_yml::from_str(yaml).unwrap();
        match &adapter.pipeline[0] {
            PipelineStep::Wait(s) => assert_eq!(s, "2.5"),
            _ => panic!("expected Wait step"),
        }
    }

    #[test]
    fn parse_click_step() {
        let yaml = r#"
site: test
name: test
columns: [status]
pipeline:
  - click: "Submit"
"#;
        let adapter: Adapter = serde_yml::from_str(yaml).unwrap();
        match &adapter.pipeline[0] {
            PipelineStep::Click(text) => assert_eq!(text, "Submit"),
            _ => panic!("expected Click step"),
        }
    }

    #[test]
    fn parse_click_selector_step() {
        let yaml = r#"
site: test
name: test
columns: [status]
pipeline:
  - click_selector: "button.submit"
"#;
        let adapter: Adapter = serde_yml::from_str(yaml).unwrap();
        match &adapter.pipeline[0] {
            PipelineStep::ClickSelector(sel) => assert_eq!(sel, "button.submit"),
            _ => panic!("expected ClickSelector step"),
        }
    }

    #[test]
    fn parse_type_step() {
        let yaml = r#"
site: test
name: test
columns: [status]
pipeline:
  - type:
      selector: "input.title"
      text: "${{ args.title }}"
"#;
        let adapter: Adapter = serde_yml::from_str(yaml).unwrap();
        match &adapter.pipeline[0] {
            PipelineStep::Type { selector, text } => {
                assert_eq!(selector, "input.title");
                assert_eq!(text, "${{ args.title }}");
            }
            _ => panic!("expected Type step"),
        }
    }

    #[test]
    fn parse_upload_step() {
        let yaml = r#"
site: test
name: test
columns: [status]
pipeline:
  - upload:
      selector: "input[type='file']"
      files: "/tmp/a.png,/tmp/b.png"
"#;
        let adapter: Adapter = serde_yml::from_str(yaml).unwrap();
        match &adapter.pipeline[0] {
            PipelineStep::Upload { selector, files } => {
                assert_eq!(selector, "input[type='file']");
                assert_eq!(files, "/tmp/a.png,/tmp/b.png");
            }
            _ => panic!("expected Upload step"),
        }
    }

    #[test]
    fn load_adapter_not_found() {
        let result = load_adapter(
            &["/nonexistent/path", "/also/nonexistent"],
            "bilibili",
            "hot",
        );
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("adapter not found"),
            "error message should mention 'adapter not found', got: {}",
            err_msg
        );
    }

    #[test]
    fn list_adapters_finds_yaml_files() {
        let adapters = list_adapters(&["adapters"]);
        assert!(adapters.len() >= 2);
        assert!(adapters
            .iter()
            .any(|a| a.site == "bilibili" && a.name == "hot"));
        assert!(adapters
            .iter()
            .any(|a| a.site == "xiaohongshu" && a.name == "publish"));
    }

    #[test]
    fn list_adapters_empty_dir() {
        let adapters = list_adapters(&["/nonexistent/path"]);
        assert!(adapters.is_empty());
    }
}
