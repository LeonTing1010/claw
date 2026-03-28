//! Sync claws from GitHub — download/update adapter YAMLs to ~/.claw/adapters/.

use serde_json::Value;

const REPO_TREE_URL: &str =
    "https://api.github.com/repos/LeonTing1010/claw/git/trees/master?recursive=1";
const RAW_BASE_URL: &str =
    "https://raw.githubusercontent.com/LeonTing1010/claw/master/adapters";

/// Returns true if ~/.claw/adapters/ is empty or does not exist.
pub fn needs_sync() -> bool {
    let home = match std::env::var("HOME") {
        Ok(h) => h,
        Err(_) => return false,
    };
    let dir = format!("{}/.claw/adapters", home);
    let path = std::path::Path::new(&dir);
    if !path.exists() {
        return true;
    }
    // Check if any .yaml file exists in subdirectories
    let Ok(entries) = std::fs::read_dir(path) else {
        return true;
    };
    for entry in entries.flatten() {
        if entry.path().is_dir() {
            if let Ok(sub) = std::fs::read_dir(entry.path()) {
                for f in sub.flatten() {
                    if f.path().extension().is_some_and(|e| e == "yaml") {
                        return false;
                    }
                }
            }
        }
    }
    true
}

/// Sync claws from GitHub to ~/.claw/adapters/.
/// Skips locally modified files. Prints progress to stderr.
pub async fn sync_claws() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::var("HOME")?;
    let target_dir = format!("{}/.claw/adapters", home);

    let http = reqwest::Client::builder()
        .user_agent("claw-cli")
        .build()?;

    // 1. List remote adapter files via GitHub Trees API (single call)
    let tree_resp: Value = http.get(REPO_TREE_URL).send().await?.json().await?;
    let tree = tree_resp["tree"]
        .as_array()
        .ok_or("GitHub API: missing tree")?;

    let remote_files: Vec<&str> = tree
        .iter()
        .filter_map(|entry| {
            let path = entry["path"].as_str()?;
            let is_blob = entry["type"].as_str()? == "blob";
            if is_blob
                && path.starts_with("adapters/")
                && path.ends_with(".yaml")
                && !path.contains("_templates/")
            {
                Some(path.strip_prefix("adapters/").unwrap())
            } else {
                None
            }
        })
        .collect();

    let total = remote_files.len();
    if total == 0 {
        eprintln!("No claws found in repository.");
        return Ok(());
    }

    let mut written = 0usize;
    let mut skipped = 0usize;

    // 2. Download each file
    for (i, rel_path) in remote_files.iter().enumerate() {
        eprint!("\rSyncing claws... {}/{}", i + 1, total);

        let url = format!("{}/{}", RAW_BASE_URL, rel_path);
        let content = match http.get(&url).send().await {
            Ok(resp) => match resp.text().await {
                Ok(text) => text,
                Err(e) => {
                    eprintln!("\n  warning: failed to read {}: {}", rel_path, e);
                    continue;
                }
            },
            Err(e) => {
                eprintln!("\n  warning: failed to fetch {}: {}", rel_path, e);
                continue;
            }
        };

        let local_path = format!("{}/{}", target_dir, rel_path);

        // Skip locally modified files
        if let Ok(local_content) = std::fs::read_to_string(&local_path) {
            if local_content == content {
                continue; // already current
            }
            skipped += 1;
            eprintln!("\n  skipped {} (locally modified)", rel_path);
            continue;
        }

        // Write new file
        if let Some(parent) = std::path::Path::new(&local_path).parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&local_path, &content)?;
        written += 1;
    }

    eprint!("\r");
    if skipped > 0 {
        eprintln!(
            "Synced {} claws to ~/.claw/adapters/ ({} locally modified, skipped)",
            written, skipped
        );
    } else {
        eprintln!("Synced {} claws to ~/.claw/adapters/", written);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_sync_returns_true_for_missing_dir() {
        // A non-existent path should trigger sync
        std::env::set_var("HOME", "/tmp/claw-test-nonexistent");
        assert!(needs_sync());
    }

    #[test]
    fn needs_sync_returns_true_for_empty_dir() {
        let tmp = std::env::temp_dir().join("claw-test-empty-sync");
        let adapters_dir = tmp.join(".claw/adapters");
        let _ = std::fs::create_dir_all(&adapters_dir);
        std::env::set_var("HOME", tmp.to_str().unwrap());
        assert!(needs_sync());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn needs_sync_returns_false_when_yaml_exists() {
        let tmp = std::env::temp_dir().join("claw-test-has-yaml");
        let site_dir = tmp.join(".claw/adapters/test");
        let _ = std::fs::create_dir_all(&site_dir);
        std::fs::write(site_dir.join("hot.yaml"), "site: test").unwrap();
        std::env::set_var("HOME", tmp.to_str().unwrap());
        assert!(!needs_sync());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
