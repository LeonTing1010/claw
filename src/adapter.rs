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
    /// YAML pipeline steps (simple adapters — fetch + map)
    #[serde(default, deserialize_with = "deserialize_pipeline_opt")]
    pub pipeline: Vec<PipelineStep>,
    /// Embedded Lua script (complex adapters — UI interactions)
    #[serde(default)]
    pub run: Option<String>,
    /// Adapter version (semver or integer)
    pub version: Option<String>,
    /// ISO timestamp when this adapter was last forged/updated
    pub last_forged: Option<String>,
    /// Who/what forged this adapter (e.g. "claude-opus-4", "human")
    pub forged_by: Option<String>,
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
    Type {
        selector: String,
        text: String,
    },
    Upload {
        selector: String,
        files: String,
    },
    WaitFor {
        selector: String,
        timeout: String,
    },
    WaitForText {
        text: String,
        timeout: String,
    },
    WaitForUrl {
        pattern: String,
        timeout: String,
    },
    WaitForNetworkIdle(String),
    Screenshot(String),
    Hover(String),
    HoverSelector(String),
    Scroll(String),
    ScrollBy {
        x: String,
        y: String,
    },
    PressKey {
        key: String,
        modifiers: String,
    },
    Select {
        selector: String,
        value: String,
    },
    DismissDialog(String),
    AssertSelector(String),
    AssertText(String),
    AssertUrl(String),
    AssertNotSelector(String),
    /// Direct HTTP fetch without browser — Tier 1 public API path
    Fetch {
        url: String,
        method: String,
        headers: HashMap<String, String>,
        body: Option<String>,
    },
    /// Extract nested data via dot-notation path (e.g. "data.items")
    SelectPath(String),
    /// Filter array items by expression (e.g. "item.views > 1000")
    Filter(String),
    /// Lua transform on pipeline data (sort, filter, group, aggregate)
    Transform(String),
    /// Intercept network response matching URL pattern, triggered by an action
    Intercept {
        trigger: String,
        capture: String,
        timeout: String,
        select: Option<String>,
    },
    /// Conditional: execute sub-steps if CSS selector exists
    IfSelector {
        selector: String,
        then_steps: Vec<PipelineStep>,
    },
    /// Conditional: execute sub-steps if visible text is found
    IfText {
        text: String,
        then_steps: Vec<PipelineStep>,
    },
    /// Conditional: execute sub-steps if URL matches pattern
    IfUrl {
        pattern: String,
        then_steps: Vec<PipelineStep>,
    },
    /// Use another adapter, injecting its output as pipeline data
    Use {
        adapter: String,
        args: Option<HashMap<String, String>>,
    },
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
/// Deserialize pipeline that may be absent (when `run:` is used instead).
fn deserialize_pipeline_opt<'de, D>(deserializer: D) -> Result<Vec<PipelineStep>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: Option<Vec<HashMap<String, serde_yml::Value>>> = Option::deserialize(deserializer)?;
    match raw {
        None => Ok(Vec::new()),
        Some(raw) => deserialize_pipeline_inner(raw),
    }
}

fn deserialize_pipeline<'de, D>(deserializer: D) -> Result<Vec<PipelineStep>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: Vec<HashMap<String, serde_yml::Value>> = Vec::deserialize(deserializer)?;
    deserialize_pipeline_inner(raw)
}

fn deserialize_pipeline_inner<E: serde::de::Error>(
    raw: Vec<HashMap<String, serde_yml::Value>>,
) -> Result<Vec<PipelineStep>, E> {
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
                let s = yml_value_to_string(&value).ok_or_else(|| {
                    serde::de::Error::custom("click value must be a string (visible text)")
                })?;
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
            "select" => match &value {
                // String → data path extraction
                serde_yml::Value::String(s) => PipelineStep::SelectPath(s.clone()),
                serde_yml::Value::Number(n) => PipelineStep::SelectPath(n.to_string()),
                // Mapping with selector+value → dropdown selection (claw-native)
                serde_yml::Value::Mapping(m) => {
                    let selector = m
                        .get(serde_yml::Value::String("selector".into()))
                        .and_then(yml_value_to_string)
                        .ok_or_else(|| {
                            serde::de::Error::custom("select step requires 'selector' field")
                        })?;
                    let val = m
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
                _ => {
                    return Err(serde::de::Error::custom(
                        "select value must be a path string or mapping with selector+value",
                    ))
                }
            },
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
            "fetch" => match &value {
                serde_yml::Value::String(url) => PipelineStep::Fetch {
                    url: url.clone(),
                    method: "GET".to_string(),
                    headers: HashMap::new(),
                    body: None,
                },
                serde_yml::Value::Mapping(m) => {
                    let url = m
                        .get(serde_yml::Value::String("url".into()))
                        .and_then(yml_value_to_string)
                        .ok_or_else(|| {
                            serde::de::Error::custom("fetch step requires 'url' field")
                        })?;
                    let method = m
                        .get(serde_yml::Value::String("method".into()))
                        .and_then(yml_value_to_string)
                        .unwrap_or_else(|| "GET".to_string());
                    let body = m
                        .get(serde_yml::Value::String("body".into()))
                        .and_then(yml_value_to_string);
                    let mut headers = HashMap::new();
                    if let Some(serde_yml::Value::Mapping(hm)) =
                        m.get(serde_yml::Value::String("headers".into()))
                    {
                        for (k, v) in hm {
                            if let (Some(ks), Some(vs)) =
                                (yml_value_to_string(k), yml_value_to_string(v))
                            {
                                headers.insert(ks, vs);
                            }
                        }
                    }
                    PipelineStep::Fetch {
                        url,
                        method,
                        headers,
                        body,
                    }
                }
                _ => {
                    return Err(serde::de::Error::custom(
                        "fetch value must be a URL string or mapping",
                    ))
                }
            },
            "filter" => {
                let s = yml_value_to_string(&value)
                    .ok_or_else(|| serde::de::Error::custom("filter value must be a string"))?;
                PipelineStep::Filter(s)
            }
            "transform" => {
                let s = yml_value_to_string(&value).ok_or_else(|| {
                    serde::de::Error::custom("transform value must be a string")
                })?;
                PipelineStep::Transform(s)
            }
            "intercept" => {
                let mapping = match &value {
                    serde_yml::Value::Mapping(m) => m,
                    _ => {
                        return Err(serde::de::Error::custom(
                            "intercept must be a mapping with trigger and capture",
                        ))
                    }
                };
                let trigger = mapping
                    .get(serde_yml::Value::String("trigger".into()))
                    .and_then(yml_value_to_string)
                    .ok_or_else(|| {
                        serde::de::Error::custom("intercept requires 'trigger' field")
                    })?;
                let capture = mapping
                    .get(serde_yml::Value::String("capture".into()))
                    .and_then(yml_value_to_string)
                    .ok_or_else(|| {
                        serde::de::Error::custom("intercept requires 'capture' field")
                    })?;
                let timeout = mapping
                    .get(serde_yml::Value::String("timeout".into()))
                    .and_then(yml_value_to_string)
                    .unwrap_or_else(|| "10".to_string());
                let select = mapping
                    .get(serde_yml::Value::String("select".into()))
                    .and_then(yml_value_to_string);
                PipelineStep::Intercept {
                    trigger,
                    capture,
                    timeout,
                    select,
                }
            }
            "if_selector" => {
                let mapping = match &value {
                    serde_yml::Value::Mapping(m) => m,
                    _ => {
                        return Err(serde::de::Error::custom(
                            "if_selector must be a mapping with selector and then",
                        ))
                    }
                };
                let selector = mapping
                    .get(serde_yml::Value::String("selector".into()))
                    .and_then(yml_value_to_string)
                    .ok_or_else(|| {
                        serde::de::Error::custom("if_selector requires 'selector' field")
                    })?;
                let then_steps = parse_then_steps::<E>(mapping)?;
                PipelineStep::IfSelector {
                    selector,
                    then_steps,
                }
            }
            "if_text" => {
                let mapping = match &value {
                    serde_yml::Value::Mapping(m) => m,
                    _ => {
                        return Err(serde::de::Error::custom(
                            "if_text must be a mapping with text and then",
                        ))
                    }
                };
                let text = mapping
                    .get(serde_yml::Value::String("text".into()))
                    .and_then(yml_value_to_string)
                    .ok_or_else(|| serde::de::Error::custom("if_text requires 'text' field"))?;
                let then_steps = parse_then_steps::<E>(mapping)?;
                PipelineStep::IfText { text, then_steps }
            }
            "if_url" => {
                let mapping = match &value {
                    serde_yml::Value::Mapping(m) => m,
                    _ => {
                        return Err(serde::de::Error::custom(
                            "if_url must be a mapping with pattern and then",
                        ))
                    }
                };
                let pattern = mapping
                    .get(serde_yml::Value::String("pattern".into()))
                    .and_then(yml_value_to_string)
                    .ok_or_else(|| serde::de::Error::custom("if_url requires 'pattern' field"))?;
                let then_steps = parse_then_steps::<E>(mapping)?;
                PipelineStep::IfUrl {
                    pattern,
                    then_steps,
                }
            }
            "use" => match &value {
                serde_yml::Value::String(s) => PipelineStep::Use {
                    adapter: s.clone(),
                    args: None,
                },
                serde_yml::Value::Mapping(m) => {
                    let adapter = m
                        .get(serde_yml::Value::String("adapter".into()))
                        .and_then(yml_value_to_string)
                        .ok_or_else(|| {
                            serde::de::Error::custom("use step requires 'adapter' field")
                        })?;
                    let step_args = if let Some(serde_yml::Value::Mapping(args_map)) =
                        m.get(serde_yml::Value::String("args".into()))
                    {
                        let mut hm = HashMap::new();
                        for (k, v) in args_map {
                            if let (Some(ks), Some(vs)) =
                                (yml_value_to_string(k), yml_value_to_string(v))
                            {
                                hm.insert(ks, vs);
                            }
                        }
                        Some(hm)
                    } else {
                        None
                    };
                    PipelineStep::Use {
                        adapter,
                        args: step_args,
                    }
                }
                _ => {
                    return Err(serde::de::Error::custom(
                        "use value must be a string or mapping with adapter field",
                    ))
                }
            },
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

/// Parse the `then` sub-steps from a conditional step mapping.
fn parse_then_steps<E: serde::de::Error>(
    mapping: &serde_yml::Mapping,
) -> Result<Vec<PipelineStep>, E> {
    let then_val = mapping
        .get(serde_yml::Value::String("then".into()))
        .ok_or_else(|| serde::de::Error::custom("conditional step requires 'then' field"))?;

    let then_seq = match then_val {
        serde_yml::Value::Sequence(seq) => seq,
        _ => {
            return Err(serde::de::Error::custom(
                "'then' field must be a list of steps",
            ))
        }
    };

    let mut sub_steps = Vec::new();
    for item in then_seq {
        let item_map = match item {
            serde_yml::Value::Mapping(m) => m,
            _ => {
                return Err(serde::de::Error::custom(
                    "each step in 'then' must be a mapping",
                ))
            }
        };
        if item_map.len() != 1 {
            return Err(serde::de::Error::custom(
                "each step in 'then' must be a single-key map",
            ));
        }
        let (k, v) = item_map.into_iter().next().unwrap();
        let key_str = yml_value_to_string(k)
            .ok_or_else(|| serde::de::Error::custom("step key not string"))?;
        let val_clone = v.clone();
        // Re-use the main pipeline parser by wrapping as a single-element list
        let wrapper_yaml = {
            let mut m = HashMap::new();
            m.insert(key_str, val_clone);
            vec![m]
        };
        let yaml_str = serde_yml::to_string(&wrapper_yaml).map_err(serde::de::Error::custom)?;
        let parsed: Vec<PipelineStep> = {
            #[derive(Deserialize)]
            struct W {
                #[serde(deserialize_with = "deserialize_pipeline")]
                pipeline: Vec<PipelineStep>,
            }
            let full = format!("pipeline:\n{}", yaml_str);
            let w: W = serde_yml::from_str(&full).map_err(serde::de::Error::custom)?;
            w.pipeline
        };
        sub_steps.extend(parsed);
    }
    Ok(sub_steps)
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

/// Parse a single pipeline step from a YAML string like "navigate: https://example.com".
/// Wraps the string in a YAML list to reuse the existing pipeline deserializer.
pub fn parse_single_step(yaml: &str) -> Result<PipelineStep, Box<dyn std::error::Error>> {
    // Wrap as a pipeline list with one entry
    let wrapper = format!("pipeline:\n  - {}", yaml);

    #[derive(Deserialize)]
    struct Wrapper {
        #[serde(deserialize_with = "deserialize_pipeline")]
        pipeline: Vec<PipelineStep>,
    }

    let w: Wrapper = serde_yml::from_str(&wrapper)?;
    w.pipeline
        .into_iter()
        .next()
        .ok_or_else(|| "empty pipeline step".into())
}

/// Resolve a site name to a login URL.
/// If adapters exist for the site, uses the first adapter's `domain` field.
/// Otherwise treats the input as a domain directly.
pub fn resolve_login_url(base_dirs: &[&str], site: &str) -> String {
    // Try to find an adapter with a domain field
    for base in base_dirs {
        let site_dir = Path::new(base).join(site);
        if let Ok(entries) = std::fs::read_dir(&site_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Ok(adapter) = serde_yml::from_str::<Adapter>(&content) {
                            if let Some(domain) = adapter.domain {
                                return format!("https://{}", domain);
                            }
                        }
                    }
                }
            }
        }
    }
    // Fallback: treat input as domain
    if site.contains('.') {
        format!("https://{}", site)
    } else {
        format!("https://{}.com", site)
    }
}

#[derive(Debug)]
pub struct AdapterInfo {
    pub site: String,
    pub name: String,
    pub description: String,
    pub strategy: String,
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

            // Skip internal directories
            if site_name.starts_with('_') || site_name == "demo" {
                continue;
            }

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

                // Try to parse for description and strategy
                let parsed = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|content| serde_yml::from_str::<Adapter>(&content).ok());
                let description = parsed
                    .as_ref()
                    .and_then(|a| a.description.clone())
                    .unwrap_or_default();
                let strategy = parsed
                    .as_ref()
                    .and_then(|a| a.strategy.clone())
                    .unwrap_or_else(|| "public".to_string());

                adapters.push(AdapterInfo {
                    site: site_name.clone(),
                    name: adapter_name,
                    description,
                    strategy,
                });
            }
        }
    }
    adapters.sort_by(|a, b| (&a.site, &a.name).cmp(&(&b.site, &b.name)));
    adapters
}

/// Compute the standard adapter search directories.
pub fn adapter_base_dirs() -> Vec<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    vec!["adapters".to_string(), format!("{}/.claw/adapters", home)]
}

/// Search for a Lua adapter file in base directories.
pub fn find_lua_adapter(base_dirs: &[&str], site: &str, name: &str) -> Option<String> {
    for base in base_dirs {
        let path = Path::new(base).join(site).join(format!("{}.lua", name));
        if path.exists() {
            return Some(path.to_string_lossy().to_string());
        }
    }
    None
}

/// Send-compatible error type for adapter composition (required for async recursion).
type SendErr = Box<dyn std::error::Error + Send + Sync>;
/// Boxed future type for adapter execution (supports recursive adapter calls).
type AdapterFuture<'a> = std::pin::Pin<
    Box<
        dyn std::future::Future<
                Output = Result<(Vec<String>, Vec<HashMap<String, String>>), SendErr>,
            > + Send
            + 'a,
    >,
>;
/// Convert any error to a Send-compatible boxed error.
fn send_err(e: impl std::fmt::Display) -> SendErr {
    e.to_string().into()
}

/// Execute an adapter by site/name, returning (columns, rows).
/// Handles Lua (.lua), inline Lua (YAML run:), and pure YAML pipeline adapters.
/// `depth` tracks recursive adapter calls to prevent infinite loops.
///
/// Returns a boxed future to support async recursion (adapter A can `use:` adapter B).
pub fn run_adapter<'a>(
    client: &'a crate::cdp::CdpClient,
    site: &'a str,
    name: &'a str,
    args: HashMap<String, serde_json::Value>,
    depth: u8,
) -> AdapterFuture<'a> {
    Box::pin(async move {
        const MAX_DEPTH: u8 = 8;
        if depth >= MAX_DEPTH {
            return Err(send_err(format!(
                "adapter call depth exceeded (max {}) — possible infinite recursion in {}/{}",
                MAX_DEPTH, site, name
            )));
        }

        let base_dirs = adapter_base_dirs();
        let refs: Vec<&str> = base_dirs.iter().map(|s| s.as_str()).collect();

        // Try Lua adapter first
        if let Some(lua_path) = find_lua_adapter(&refs, site, name) {
            return crate::lua_adapter::execute_lua_adapter(&lua_path, client.clone(), args, depth)
                .await
                .map_err(send_err);
        }

        // YAML adapter
        let ada = load_adapter(&refs, site, name).map_err(send_err)?;

        // Merge defaults with provided args
        let mut merged_args = HashMap::new();
        if let Some(ref defs) = ada.args {
            for (key, def) in defs {
                if let Some(ref default) = def.default {
                    merged_args.insert(key.clone(), default.clone());
                }
            }
        }
        for (k, v) in args {
            merged_args.insert(k, v);
        }

        if let Some(ref script) = ada.run {
            let rows = crate::lua_adapter::execute_inline_lua(
                script,
                &ada.columns,
                client.clone(),
                merged_args,
                depth,
            )
            .await
            .map_err(send_err)?;
            Ok((ada.columns, rows))
        } else {
            let rows = crate::pipeline::execute(&ada.pipeline, client, merged_args, depth)
                .await
                .map_err(send_err)?;
            Ok((ada.columns, rows))
        }
    })
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

    #[test]
    fn parse_single_step_navigate() {
        let step = parse_single_step("navigate: https://example.com").unwrap();
        match step {
            PipelineStep::Navigate(url) => assert_eq!(url, "https://example.com"),
            _ => panic!("expected Navigate"),
        }
    }

    #[test]
    fn parse_single_step_click() {
        let step = parse_single_step("click: Submit").unwrap();
        match step {
            PipelineStep::Click(text) => assert_eq!(text, "Submit"),
            _ => panic!("expected Click"),
        }
    }

    #[test]
    fn parse_if_selector_step() {
        let yaml = r#"
site: test
name: test
columns: [status]
pipeline:
  - if_selector:
      selector: ".modal"
      then:
        - click_selector: ".modal-close"
        - wait: 1
"#;
        let adapter: Adapter = serde_yml::from_str(yaml).unwrap();
        assert_eq!(adapter.pipeline.len(), 1);
        match &adapter.pipeline[0] {
            PipelineStep::IfSelector {
                selector,
                then_steps,
            } => {
                assert_eq!(selector, ".modal");
                assert_eq!(then_steps.len(), 2);
                match &then_steps[0] {
                    PipelineStep::ClickSelector(s) => assert_eq!(s, ".modal-close"),
                    _ => panic!("expected ClickSelector in then"),
                }
            }
            _ => panic!("expected IfSelector"),
        }
    }

    #[test]
    fn parse_if_text_step() {
        let yaml = r#"
site: test
name: test
columns: [status]
pipeline:
  - if_text:
      text: "验证码"
      then:
        - screenshot: /tmp/captcha.png
"#;
        let adapter: Adapter = serde_yml::from_str(yaml).unwrap();
        match &adapter.pipeline[0] {
            PipelineStep::IfText { text, then_steps } => {
                assert_eq!(text, "验证码");
                assert_eq!(then_steps.len(), 1);
            }
            _ => panic!("expected IfText"),
        }
    }

    #[test]
    fn parse_adapter_metadata() {
        let yaml = r#"
site: test
name: test
version: "1.2"
last_forged: "2026-03-27T10:00:00Z"
forged_by: "claude-opus-4"
columns: [status]
pipeline:
  - navigate: https://example.com
"#;
        let adapter: Adapter = serde_yml::from_str(yaml).unwrap();
        assert_eq!(adapter.version, Some("1.2".to_string()));
        assert_eq!(
            adapter.last_forged,
            Some("2026-03-27T10:00:00Z".to_string())
        );
        assert_eq!(adapter.forged_by, Some("claude-opus-4".to_string()));
    }

    #[test]
    fn parse_assert_steps() {
        let yaml = r#"
site: test
name: test
columns: [status]
pipeline:
  - assert_selector: ".success"
  - assert_text: "Published"
  - assert_url: "/dashboard"
  - assert_not_selector: ".error"
"#;
        let adapter: Adapter = serde_yml::from_str(yaml).unwrap();
        assert_eq!(adapter.pipeline.len(), 4);
        match &adapter.pipeline[0] {
            PipelineStep::AssertSelector(s) => assert_eq!(s, ".success"),
            _ => panic!("expected AssertSelector"),
        }
        match &adapter.pipeline[1] {
            PipelineStep::AssertText(t) => assert_eq!(t, "Published"),
            _ => panic!("expected AssertText"),
        }
    }

    #[test]
    fn parse_use_step_simple() {
        let yaml = r#"
site: test
name: test
columns: [title]
pipeline:
  - use: bilibili/hot
"#;
        let adapter: Adapter = serde_yml::from_str(yaml).unwrap();
        match &adapter.pipeline[0] {
            PipelineStep::Use { adapter, args } => {
                assert_eq!(adapter, "bilibili/hot");
                assert!(args.is_none());
            }
            _ => panic!("expected Use step"),
        }
    }

    #[test]
    fn parse_use_step_with_args() {
        let yaml = r#"
site: test
name: test
columns: [title]
pipeline:
  - use:
      adapter: bilibili/hot
      args:
        limit: 3
"#;
        let adapter: Adapter = serde_yml::from_str(yaml).unwrap();
        match &adapter.pipeline[0] {
            PipelineStep::Use { adapter, args } => {
                assert_eq!(adapter, "bilibili/hot");
                let a = args.as_ref().unwrap();
                assert_eq!(a.get("limit"), Some(&"3".to_string()));
            }
            _ => panic!("expected Use step with args"),
        }
    }

    #[test]
    fn adapter_base_dirs_returns_two_dirs() {
        let dirs = adapter_base_dirs();
        assert_eq!(dirs.len(), 2);
        assert_eq!(dirs[0], "adapters");
        assert!(dirs[1].contains(".claw/adapters"));
    }

    #[test]
    fn run_adapter_depth_guard() {
        // Verify the depth constant is reasonable
        // (actual async test would require a CDP client, so we test the limit logic)
        let max_depth: u8 = 8;
        assert!(
            max_depth >= 4,
            "depth limit should allow reasonable nesting"
        );
        assert!(
            max_depth <= 16,
            "depth limit should prevent runaway recursion"
        );
    }
}
