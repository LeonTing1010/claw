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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn adapter_base_dirs_returns_two_dirs() {
        let dirs = adapter_base_dirs();
        assert_eq!(dirs.len(), 2);
        assert_eq!(dirs[0], "adapters");
        assert!(dirs[1].contains(".claw/adapters"));
    }
}
