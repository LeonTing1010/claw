use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// Adapter metadata loaded from YAML.
/// Execution is handled by the Chrome extension in v2.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct Adapter {
    pub site: String,
    pub name: String,
    pub description: Option<String>,
    pub domain: Option<String>,
    pub strategy: Option<String>,
    pub browser: Option<bool>,
    pub columns: Vec<String>,
    pub version: Option<String>,
    pub last_forged: Option<String>,
    pub forged_by: Option<String>,
    #[serde(default)]
    pub schema: Option<HashMap<String, String>>,
    #[serde(default)]
    pub health: Option<HealthContract>,
}

/// Health contract: output quality assertions for a claw.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct HealthContract {
    /// Minimum number of rows the adapter must return.
    pub min_rows: Option<usize>,
    /// Columns that must have non-empty values in every row.
    pub non_empty: Option<Vec<String>>,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct AdapterInfo {
    pub site: String,
    pub name: String,
    pub description: String,
    pub strategy: String,
    /// "yaml" or "js"
    pub format: String,
}

/// Scan adapter directories for .yaml and .claw.js files and return metadata.
pub fn list_adapters(base_dirs: &[&str]) -> Vec<AdapterInfo> {
    let mut adapters = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for base in base_dirs {
        let base_path = Path::new(base);
        let Ok(sites) = std::fs::read_dir(base_path) else {
            continue;
        };

        for site_entry in sites.flatten() {
            if !site_entry.path().is_dir() {
                continue;
            }
            let site_name = site_entry.file_name().to_string_lossy().to_string();
            if site_name.starts_with('_') || site_name == "demo" {
                continue;
            }

            let Ok(files) = std::fs::read_dir(site_entry.path()) else {
                continue;
            };
            for file_entry in files.flatten() {
                let path = file_entry.path();
                let filename = path.file_name().and_then(|f| f.to_str()).unwrap_or("");

                // Match .yaml or .claw.js
                let (adapter_name, format) = if filename.ends_with(".yaml") {
                    (filename.strip_suffix(".yaml").unwrap().to_string(), "yaml")
                } else if filename.ends_with(".claw.js") {
                    (
                        filename.strip_suffix(".claw.js").unwrap().to_string(),
                        "js",
                    )
                } else {
                    continue;
                };

                let key = format!("{}/{}", site_name, adapter_name);
                if seen.contains(&key) {
                    continue;
                }
                seen.insert(key);

                let (description, strategy) = if format == "yaml" {
                    parse_yaml_metadata(&path)
                } else {
                    parse_clawjs_metadata(&path)
                };

                adapters.push(AdapterInfo {
                    site: site_name.clone(),
                    name: adapter_name,
                    description,
                    strategy,
                    format: format.to_string(),
                });
            }
        }
    }
    adapters.sort_by(|a, b| (&a.site, &a.name).cmp(&(&b.site, &b.name)));
    adapters
}

/// Parse metadata from a YAML adapter file.
fn parse_yaml_metadata(path: &Path) -> (String, String) {
    let parsed = std::fs::read_to_string(path)
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
    (description, strategy)
}

/// Parse metadata from a .claw.js file using simple regex extraction.
fn parse_clawjs_metadata(path: &Path) -> (String, String) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return (String::new(), "public".to_string()),
    };

    // Extract description: "..." from the JS module
    let description = extract_js_string_field(&content, "description").unwrap_or_default();
    (description, "public".to_string())
}

/// Extract a simple string field value from a .claw.js export default object.
/// Matches patterns like: description: "some text" or description: 'some text'
fn extract_js_string_field(content: &str, field: &str) -> Option<String> {
    let pattern = format!(r#"{}:\s*["']([^"']+)["']"#, regex::escape(field));
    let re = regex::Regex::new(&pattern).ok()?;
    re.captures(content)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
}

/// Compute the standard adapter search directories (YAML + .claw.js).
pub fn adapter_base_dirs() -> Vec<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    vec![
        "adapters".to_string(),
        format!("{}/.claw/adapters", home),
        format!("{}/.claw/claws", home),
    ]
}

/// Parse a HealthContract from a JSON value (e.g., from extension claw metadata).
pub fn parse_health_contract(value: &serde_json::Value) -> Option<HealthContract> {
    let obj = value.as_object()?;
    Some(HealthContract {
        min_rows: obj.get("min_rows").and_then(|v| v.as_u64()).map(|v| v as usize),
        non_empty: obj.get("non_empty").and_then(|v| {
            v.as_array()
                .map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect())
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn list_adapters_finds_yaml_files() {
        let adapters = list_adapters(&["adapters"]);
        assert!(adapters.len() >= 2);
    }

    #[test]
    fn list_adapters_empty_dir() {
        let adapters = list_adapters(&["/nonexistent/path"]);
        assert!(adapters.is_empty());
    }

    #[test]
    fn adapter_base_dirs_includes_claws() {
        let dirs = adapter_base_dirs();
        assert_eq!(dirs.len(), 3);
        assert_eq!(dirs[0], "adapters");
        assert!(dirs[1].contains(".claw/adapters"));
        assert!(dirs[2].contains(".claw/claws"));
    }

    #[test]
    fn extract_js_description() {
        let js = r#"export default {
  site: "hackernews",
  name: "hot",
  description: "Hacker News top stories",
  columns: ["rank", "title"],
}"#;
        let desc = extract_js_string_field(js, "description");
        assert_eq!(desc.unwrap(), "Hacker News top stories");
    }

    #[test]
    fn extract_js_description_single_quotes() {
        let js = "description: 'GitHub Trending'";
        let desc = extract_js_string_field(js, "description");
        assert_eq!(desc.unwrap(), "GitHub Trending");
    }

    #[test]
    fn extract_js_missing_field() {
        let js = "site: 'github'";
        let desc = extract_js_string_field(js, "description");
        assert!(desc.is_none());
    }

    #[test]
    fn parse_health_contract_from_json() {
        let val = json!({"min_rows": 5, "non_empty": ["title", "url"]});
        let hc = parse_health_contract(&val).unwrap();
        assert_eq!(hc.min_rows, Some(5));
        assert_eq!(
            hc.non_empty,
            Some(vec!["title".to_string(), "url".to_string()])
        );
    }

    #[test]
    fn parse_health_contract_partial() {
        let val = json!({"min_rows": 3});
        let hc = parse_health_contract(&val).unwrap();
        assert_eq!(hc.min_rows, Some(3));
        assert!(hc.non_empty.is_none());
    }

    #[test]
    fn list_adapters_finds_clawjs_files() {
        let adapters = list_adapters(&["extension-v2/claws"]);
        assert!(adapters.len() >= 1);
        let js_adapters: Vec<_> = adapters.iter().filter(|a| a.format == "js").collect();
        assert!(!js_adapters.is_empty(), "should find .claw.js files");
    }
}
