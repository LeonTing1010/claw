use std::path::PathBuf;
use std::process::Command;

/// Build Chrome launch arguments.
fn chrome_launch_args(port: u16, headless: bool, profile_dir: &std::path::Path) -> Vec<String> {
    let mut args = vec![
        format!("--remote-debugging-port={}", port),
        format!("--user-data-dir={}", profile_dir.display()),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
    ];
    if headless {
        args.push("--headless=new".to_string());
    }
    args
}

/// Ensure Chrome is reachable on the given port.
/// If not, auto-launch a Chrome instance with --remote-debugging-port.
pub async fn ensure_chrome(port: u16, headless: bool) -> Result<(), Box<dyn std::error::Error>> {
    // Already reachable?
    if is_chrome_reachable(port).await {
        return Ok(());
    }

    eprintln!("Chrome not found on port {}, launching...", port);

    let chrome_path = find_chrome()?;
    let profile_dir = chrome_profile_dir();

    // Ensure profile directory exists
    std::fs::create_dir_all(&profile_dir).ok();

    // Launch Chrome in background
    let args = chrome_launch_args(port, headless, &profile_dir);
    let mut cmd = Command::new(&chrome_path);
    for arg in &args {
        cmd.arg(arg);
    }
    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to launch Chrome: {}", e))?;

    // Wait for Chrome to be ready (up to 30 seconds — first launch can be slow)
    for i in 0..60 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        if is_chrome_reachable(port).await {
            eprintln!("Chrome ready (took {:.1}s)", (i + 1) as f64 * 0.5);
            return Ok(());
        }
    }

    Err(format!(
        "Chrome launched but not responding on port {} after 30s.\n\
         Profile: {}\n\
         Try manually: {} --remote-debugging-port={} --user-data-dir={}",
        port,
        profile_dir.display(),
        chrome_path.display(),
        port,
        profile_dir.display()
    )
    .into())
}

/// Check if Chrome CDP is reachable on the given port.
async fn is_chrome_reachable(port: u16) -> bool {
    crate::cdp::CdpClient::http_get(port, "/json/version")
        .await
        .is_ok()
}

/// Find Chrome executable path (platform-specific).
fn find_chrome() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let candidates = if cfg!(target_os = "macos") {
        vec![
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        ]
    } else if cfg!(target_os = "linux") {
        vec![
            "/usr/bin/google-chrome",
            "/usr/bin/google-chrome-stable",
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
            "/snap/bin/chromium",
        ]
    } else {
        vec![]
    };

    for path in &candidates {
        if std::path::Path::new(path).exists() {
            return Ok(PathBuf::from(path));
        }
    }

    // Try PATH
    if let Ok(output) = Command::new("which").arg("google-chrome").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Ok(PathBuf::from(path));
            }
        }
    }

    Err("Chrome not found. Install Google Chrome or set its path.".into())
}

/// Persistent Chrome profile directory (~/.claw/chrome-profile/).
fn chrome_profile_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".claw").join("chrome-profile")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_chrome_returns_path_on_macos() {
        // This test only passes on macOS with Chrome installed
        if cfg!(target_os = "macos")
            && std::path::Path::new("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
                .exists()
        {
            let path = find_chrome().unwrap();
            assert!(path.exists());
        }
    }

    #[test]
    fn chrome_profile_dir_under_home() {
        let dir = chrome_profile_dir();
        assert!(dir.to_string_lossy().contains(".claw/chrome-profile"));
    }

    #[test]
    fn chrome_launch_args_headless() {
        let dir = std::path::Path::new("/tmp/test-profile");
        let args = chrome_launch_args(9222, true, dir);
        assert!(args.contains(&"--headless=new".to_string()));
        assert!(args.contains(&"--remote-debugging-port=9222".to_string()));
    }

    #[test]
    fn chrome_launch_args_no_headless() {
        let dir = std::path::Path::new("/tmp/test-profile");
        let args = chrome_launch_args(9222, false, dir);
        assert!(!args.iter().any(|a| a.contains("headless")));
    }
}
