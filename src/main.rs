mod adapter;
mod browser;
mod cdp;
mod output;
mod pipeline;
mod template;

use std::collections::HashMap;

use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use serde_json::Value;

#[derive(Parser)]
#[command(
    name = "claw",
    about = "Turn any website into a CLI — with native browser precision"
)]
#[command(allow_external_subcommands = true)]
struct Cli {
    /// Chrome CDP debugging port
    #[arg(long, default_value_t = 9222, global = true)]
    port: u16,

    /// Output format: table, json, csv
    #[arg(short = 'f', long, default_value = "table", global = true)]
    format: String,

    /// Run Chrome in headless mode (no GUI)
    #[arg(long, global = true)]
    headless: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Evaluate a JavaScript expression in the browser
    Evaluate {
        /// JS expression to evaluate
        expression: String,
    },
    /// Navigate the browser to a URL
    Navigate {
        /// Target URL
        url: String,
    },
    /// Show browser connection info
    Version,
    /// List available adapters
    List,
    /// Diagnose Chrome CDP connection
    Doctor,
    /// Generate shell completions
    Completions {
        /// Shell: bash, zsh, fish, powershell, elvish
        shell: Shell,
    },

    // ---- SEE (Perception) ----
    /// Take a screenshot of the current page
    Screenshot {
        /// Output file path
        #[arg(short, long, default_value = "/tmp/claw-screenshot.png")]
        path: String,
        /// Capture full page beyond viewport
        #[arg(long)]
        full_page: bool,
    },
    /// Get the accessibility tree (semantic page structure)
    #[command(name = "ax-tree")]
    AxTree {
        /// Max depth to traverse
        #[arg(short, long)]
        depth: Option<i32>,
    },
    /// Get a simplified DOM tree with key attributes
    #[command(name = "read-dom")]
    ReadDom {
        /// CSS selector for subtree root (default: body)
        #[arg(short, long)]
        selector: Option<String>,
        /// Max depth to traverse
        #[arg(short, long, default_value_t = 10)]
        depth: i32,
    },
    /// Get current page info (URL, title, viewport, scroll)
    #[command(name = "page-info")]
    PageInfo,

    // ---- PROBE (Discovery) ----
    /// Find elements by visible text and optional role
    Find {
        /// Text to search for
        query: String,
        /// Filter by element role (button, link, input, etc.)
        #[arg(short, long)]
        role: Option<String>,
    },
    /// Deep probe of a single element
    #[command(name = "element-info")]
    ElementInfo {
        /// CSS selector
        selector: String,
    },
    /// List event listeners on an element
    #[command(name = "event-listeners")]
    EventListeners {
        /// CSS selector
        selector: String,
    },
    /// Get cookies for the current page
    Cookies,
    /// Hit-test: what element is at pixel (x, y)?
    #[command(name = "hit-test")]
    HitTest {
        /// X coordinate
        x: f64,
        /// Y coordinate
        y: f64,
    },
    /// Find blocking modals/dialogs in the top layer
    #[command(name = "top-layer")]
    TopLayer,
    /// Force pseudo-state (:hover, :focus) on an element
    #[command(name = "force-state")]
    ForceState {
        /// CSS selector
        selector: String,
        /// Pseudo-state: hover, focus, active, focus-within
        #[arg(short = 's', long, value_delimiter = ',')]
        states: Vec<String>,
    },
    /// Start/stop/dump network request logging
    #[command(name = "network-log")]
    NetworkLog {
        /// Action: start, stop, dump
        action: String,
    },

    // ---- TRY (Actions) ----
    /// Hover over an element (triggers CSS :hover, tooltips)
    Hover {
        /// CSS selector to hover
        selector: String,
    },
    /// Scroll an element into view
    Scroll {
        /// CSS selector to scroll to
        selector: String,
    },
    /// Press a specific key (Enter, Tab, Escape, etc.)
    #[command(name = "press-key")]
    PressKey {
        /// Key name (Enter, Tab, Escape, ArrowDown, etc.)
        key: String,
        /// Modifier keys: alt=1, ctrl=2, meta=4, shift=8 (sum for combos)
        #[arg(short, long, default_value_t = 0)]
        modifiers: u32,
    },
    /// Select an option in a <select> dropdown
    Select {
        /// CSS selector of the <select> element
        selector: String,
        /// Value to select
        value: String,
    },
    /// Click on an element by text content
    Click {
        /// Visible text to click
        text: String,
    },
    /// Click on an element by CSS selector
    #[command(name = "click-selector")]
    ClickSelector {
        /// CSS selector to click
        selector: String,
    },
    /// Type text into an input element
    Type {
        /// CSS selector of the input element
        selector: String,
        /// Text to type
        text: String,
    },
    /// Dismiss a JavaScript dialog (alert/confirm/prompt)
    #[command(name = "dismiss-dialog")]
    DismissDialog {
        /// Accept the dialog (default: true)
        #[arg(long, default_value_t = true)]
        accept: bool,
        /// Text for prompt dialogs
        #[arg(long)]
        prompt_text: Option<String>,
    },

    /// Run an adapter (implicit: claw <site> <name> [--args])
    #[command(external_subcommand)]
    Adapter(Vec<String>),
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if let Err(e) = run(cli).await {
        eprintln!("error: {}", e);
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    match cli.command {
        Command::Version => {
            browser::ensure_chrome(cli.port, cli.headless).await?;
            let ws_url = cdp::CdpClient::discover_ws_url(cli.port).await?;
            println!("{}", ws_url);
        }
        Command::Evaluate { expression } => {
            browser::ensure_chrome(cli.port, cli.headless).await?;
            let ws_url = cdp::CdpClient::discover_ws_url(cli.port).await?;
            let client = cdp::CdpClient::connect(&ws_url).await?;
            let result = client.evaluate(&expression).await?;
            let out = if result.is_string() {
                result.as_str().unwrap().to_string()
            } else {
                serde_json::to_string_pretty(&result)?
            };
            println!("{}", out);
        }
        Command::Navigate { url } => {
            browser::ensure_chrome(cli.port, cli.headless).await?;
            let ws_url = cdp::CdpClient::discover_ws_url(cli.port).await?;
            let client = cdp::CdpClient::connect(&ws_url).await?;
            client.navigate(&url).await?;
            println!("navigated to {}", url);
        }
        Command::List => {
            let dirs = adapter_base_dirs();
            let refs: Vec<&str> = dirs.iter().map(|s| s.as_str()).collect();
            let adapters = adapter::list_adapters(&refs);
            if adapters.is_empty() {
                println!("No adapters found.");
            } else {
                let columns = vec!["site".into(), "name".into(), "description".into()];
                let rows: Vec<std::collections::HashMap<String, String>> = adapters
                    .iter()
                    .map(|a| {
                        let mut row = std::collections::HashMap::new();
                        row.insert("site".into(), a.site.clone());
                        row.insert("name".into(), a.name.clone());
                        row.insert("description".into(), a.description.clone());
                        row
                    })
                    .collect();
                output::print_output(&columns, &rows, &cli.format)?;
            }
        }
        Command::Completions { shell } => {
            let mut cmd = Cli::command();
            generate(shell, &mut cmd, "claw", &mut std::io::stdout());
        }
        Command::Doctor => {
            // 1. TCP connectivity (don't auto-launch for doctor — diagnostic mode)
            let version_body = match cdp::CdpClient::http_get(cli.port, "/json/version").await {
                Ok(body) => {
                    let info: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                    let browser = info["Browser"].as_str().unwrap_or("unknown");
                    println!("[ok] Chrome reachable on port {} ({})", cli.port, browser);
                    Some(body)
                }
                Err(e) => {
                    println!("[fail] Cannot connect to Chrome on port {}", cli.port);
                    println!("       Error: {}", e);
                    println!("       Start Chrome with: chrome --remote-debugging-port={} --user-data-dir=/tmp/claw-chrome", cli.port);
                    None
                }
            };

            if version_body.is_none() {
                return Ok(());
            }

            // 2. Page targets
            match cdp::CdpClient::http_get(cli.port, "/json").await {
                Ok(body) => {
                    let targets: Vec<serde_json::Value> =
                        serde_json::from_str(&body).unwrap_or_default();
                    let pages = targets
                        .iter()
                        .filter(|t| t["type"].as_str() == Some("page"))
                        .count();
                    if pages > 0 {
                        println!("[ok] {} page target(s) found", pages);
                    } else {
                        println!("[warn] No page targets — open a tab in Chrome");
                        return Ok(());
                    }
                }
                Err(e) => {
                    println!("[fail] Cannot list targets: {}", e);
                    return Ok(());
                }
            }

            // 3. JS evaluation
            match cdp::CdpClient::discover_ws_url(cli.port).await {
                Ok(ws_url) => match cdp::CdpClient::connect(&ws_url).await {
                    Ok(client) => match client.evaluate("1+1").await {
                        Ok(val) if val == 2 => println!("[ok] JavaScript evaluation working"),
                        Ok(val) => println!("[warn] Unexpected eval result: {}", val),
                        Err(e) => println!("[fail] JS evaluation failed: {}", e),
                    },
                    Err(e) => println!("[fail] WebSocket connection failed: {}", e),
                },
                Err(e) => println!("[fail] Cannot discover page: {}", e),
            }
        }
        // ---- SEE (Perception) ----
        Command::Screenshot { path, full_page } => {
            browser::ensure_chrome(cli.port, cli.headless).await?;
            let ws_url = cdp::CdpClient::discover_ws_url(cli.port).await?;
            let client = cdp::CdpClient::connect(&ws_url).await?;
            if full_page {
                client.screenshot_full(&path).await?;
            } else {
                client.screenshot(&path).await?;
            }
            println!("{}", path);
        }
        Command::AxTree { depth } => {
            browser::ensure_chrome(cli.port, cli.headless).await?;
            let ws_url = cdp::CdpClient::discover_ws_url(cli.port).await?;
            let client = cdp::CdpClient::connect(&ws_url).await?;
            let tree = client.get_ax_tree(depth).await?;
            println!("{}", serde_json::to_string_pretty(&tree)?);
        }
        Command::ReadDom { selector, depth } => {
            browser::ensure_chrome(cli.port, cli.headless).await?;
            let ws_url = cdp::CdpClient::discover_ws_url(cli.port).await?;
            let client = cdp::CdpClient::connect(&ws_url).await?;
            let tree = client.get_dom_tree(selector.as_deref(), depth).await?;
            println!("{}", serde_json::to_string_pretty(&tree)?);
        }
        Command::PageInfo => {
            browser::ensure_chrome(cli.port, cli.headless).await?;
            let ws_url = cdp::CdpClient::discover_ws_url(cli.port).await?;
            let client = cdp::CdpClient::connect(&ws_url).await?;
            let info = client.get_page_info().await?;
            println!("{}", serde_json::to_string_pretty(&info)?);
        }

        // ---- PROBE (Discovery) ----
        Command::Find { query, role } => {
            browser::ensure_chrome(cli.port, cli.headless).await?;
            let ws_url = cdp::CdpClient::discover_ws_url(cli.port).await?;
            let client = cdp::CdpClient::connect(&ws_url).await?;
            let results = client.find_elements(&query, role.as_deref()).await?;
            println!("{}", serde_json::to_string_pretty(&results)?);
        }
        Command::ElementInfo { selector } => {
            browser::ensure_chrome(cli.port, cli.headless).await?;
            let ws_url = cdp::CdpClient::discover_ws_url(cli.port).await?;
            let client = cdp::CdpClient::connect(&ws_url).await?;
            let info = client.get_element_info(&selector).await?;
            println!("{}", serde_json::to_string_pretty(&info)?);
        }
        Command::EventListeners { selector } => {
            browser::ensure_chrome(cli.port, cli.headless).await?;
            let ws_url = cdp::CdpClient::discover_ws_url(cli.port).await?;
            let client = cdp::CdpClient::connect(&ws_url).await?;
            let listeners = client.get_event_listeners(&selector).await?;
            println!("{}", serde_json::to_string_pretty(&listeners)?);
        }
        Command::Cookies => {
            browser::ensure_chrome(cli.port, cli.headless).await?;
            let ws_url = cdp::CdpClient::discover_ws_url(cli.port).await?;
            let client = cdp::CdpClient::connect(&ws_url).await?;
            let cookies = client.get_cookies().await?;
            println!("{}", serde_json::to_string_pretty(&cookies)?);
        }
        Command::HitTest { x, y } => {
            browser::ensure_chrome(cli.port, cli.headless).await?;
            let ws_url = cdp::CdpClient::discover_ws_url(cli.port).await?;
            let client = cdp::CdpClient::connect(&ws_url).await?;
            let result = client.hit_test(x, y).await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::TopLayer => {
            browser::ensure_chrome(cli.port, cli.headless).await?;
            let ws_url = cdp::CdpClient::discover_ws_url(cli.port).await?;
            let client = cdp::CdpClient::connect(&ws_url).await?;
            let elements = client.get_top_layer().await?;
            println!("{}", serde_json::to_string_pretty(&elements)?);
        }
        Command::ForceState { selector, states } => {
            browser::ensure_chrome(cli.port, cli.headless).await?;
            let ws_url = cdp::CdpClient::discover_ws_url(cli.port).await?;
            let client = cdp::CdpClient::connect(&ws_url).await?;
            let state_refs: Vec<&str> = states.iter().map(|s| s.as_str()).collect();
            client.force_pseudo_state(&selector, &state_refs).await?;
            println!("forced {:?} on {}", states, selector);
        }
        Command::NetworkLog { action } => {
            browser::ensure_chrome(cli.port, cli.headless).await?;
            let ws_url = cdp::CdpClient::discover_ws_url(cli.port).await?;
            let client = cdp::CdpClient::connect(&ws_url).await?;
            match action.as_str() {
                "start" => {
                    client.start_network_log().await?;
                    println!("network logging started");
                }
                "dump" | "stop" => {
                    let log = client.get_network_log().await?;
                    println!("{}", serde_json::to_string_pretty(&log)?);
                }
                _ => {
                    return Err(format!(
                        "unknown network-log action: {} (use: start, stop, dump)",
                        action
                    )
                    .into())
                }
            }
        }

        // ---- TRY (Actions) ----
        Command::Hover { selector } => {
            browser::ensure_chrome(cli.port, cli.headless).await?;
            let ws_url = cdp::CdpClient::discover_ws_url(cli.port).await?;
            let client = cdp::CdpClient::connect(&ws_url).await?;
            client.hover_selector(&selector).await?;
            println!("hovered {}", selector);
        }
        Command::Scroll { selector } => {
            browser::ensure_chrome(cli.port, cli.headless).await?;
            let ws_url = cdp::CdpClient::discover_ws_url(cli.port).await?;
            let client = cdp::CdpClient::connect(&ws_url).await?;
            client.scroll_into_view(&selector).await?;
            println!("scrolled to {}", selector);
        }
        Command::PressKey { key, modifiers } => {
            browser::ensure_chrome(cli.port, cli.headless).await?;
            let ws_url = cdp::CdpClient::discover_ws_url(cli.port).await?;
            let client = cdp::CdpClient::connect(&ws_url).await?;
            client.press_key(&key, modifiers).await?;
            println!("pressed {}", key);
        }
        Command::Select { selector, value } => {
            browser::ensure_chrome(cli.port, cli.headless).await?;
            let ws_url = cdp::CdpClient::discover_ws_url(cli.port).await?;
            let client = cdp::CdpClient::connect(&ws_url).await?;
            client.select_option(&selector, &value).await?;
            println!("selected {} = {}", selector, value);
        }
        Command::Click { text } => {
            browser::ensure_chrome(cli.port, cli.headless).await?;
            let ws_url = cdp::CdpClient::discover_ws_url(cli.port).await?;
            let client = cdp::CdpClient::connect(&ws_url).await?;
            client.click_text(&text).await?;
            println!("clicked \"{}\"", text);
        }
        Command::ClickSelector { selector } => {
            browser::ensure_chrome(cli.port, cli.headless).await?;
            let ws_url = cdp::CdpClient::discover_ws_url(cli.port).await?;
            let client = cdp::CdpClient::connect(&ws_url).await?;
            client.click_selector(&selector).await?;
            println!("clicked {}", selector);
        }
        Command::Type { selector, text } => {
            browser::ensure_chrome(cli.port, cli.headless).await?;
            let ws_url = cdp::CdpClient::discover_ws_url(cli.port).await?;
            let client = cdp::CdpClient::connect(&ws_url).await?;
            client.type_into(&selector, &text).await?;
            println!("typed into {}", selector);
        }
        Command::DismissDialog {
            accept,
            prompt_text,
        } => {
            browser::ensure_chrome(cli.port, cli.headless).await?;
            let ws_url = cdp::CdpClient::discover_ws_url(cli.port).await?;
            let client = cdp::CdpClient::connect(&ws_url).await?;
            client
                .dismiss_dialog(accept, prompt_text.as_deref())
                .await?;
            println!("dialog {}", if accept { "accepted" } else { "dismissed" });
        }

        Command::Adapter(raw_args) => {
            if raw_args.len() < 2 {
                return Err("usage: claw <site> <name> [--arg value ...]".into());
            }
            let site = &raw_args[0];
            let name = &raw_args[1];

            let dirs = adapter_base_dirs();
            let refs: Vec<&str> = dirs.iter().map(|s| s.as_str()).collect();

            let ada = adapter::load_adapter(&refs, site, name)?;

            // Merge defaults + CLI args
            let mut args = HashMap::new();
            if let Some(ref defs) = ada.args {
                for (key, def) in defs {
                    if let Some(ref default) = def.default {
                        args.insert(key.clone(), default.clone());
                    }
                }
            }
            let cli_args = parse_adapter_args(&raw_args[2..]);
            for (k, v) in cli_args {
                args.insert(k, v);
            }

            browser::ensure_chrome(cli.port, cli.headless).await?;
            let ws_url = cdp::CdpClient::discover_ws_url(cli.port).await?;
            let client = cdp::CdpClient::connect(&ws_url).await?;

            let rows = pipeline::execute(&ada.pipeline, &client, args).await?;
            output::print_output(&ada.columns, &rows, &cli.format)?;
        }
    }
    Ok(())
}

fn adapter_base_dirs() -> Vec<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    vec!["adapters".to_string(), format!("{}/.claw/adapters", home)]
}

/// Parse --key value pairs from raw CLI args into a HashMap.
fn parse_adapter_args(raw: &[String]) -> HashMap<String, Value> {
    let mut args = HashMap::new();
    let mut i = 0;
    while i < raw.len() {
        if let Some(key) = raw[i].strip_prefix("--") {
            if i + 1 < raw.len() && !raw[i + 1].starts_with("--") {
                let val = &raw[i + 1];
                let json_val = if let Ok(n) = val.parse::<i64>() {
                    Value::Number(n.into())
                } else if let Ok(f) = val.parse::<f64>() {
                    Value::Number(serde_json::Number::from_f64(f).unwrap())
                } else {
                    Value::String(val.clone())
                };
                args.insert(key.to_string(), json_val);
                i += 2;
            } else {
                args.insert(key.to_string(), Value::Bool(true));
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_adapter_args_numeric() {
        let raw: Vec<String> = vec!["--limit", "5"].into_iter().map(String::from).collect();
        let args = parse_adapter_args(&raw);
        assert_eq!(args.get("limit"), Some(&json!(5)));
    }

    #[test]
    fn parse_adapter_args_string() {
        let raw: Vec<String> = vec!["--query", "rust"]
            .into_iter()
            .map(String::from)
            .collect();
        let args = parse_adapter_args(&raw);
        assert_eq!(args.get("query"), Some(&json!("rust")));
    }

    #[test]
    fn parse_adapter_args_flag() {
        let raw: Vec<String> = vec!["--verbose"].into_iter().map(String::from).collect();
        let args = parse_adapter_args(&raw);
        assert_eq!(args.get("verbose"), Some(&json!(true)));
    }
}
