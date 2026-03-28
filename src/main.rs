#![recursion_limit = "256"]
mod adapter;
mod bridge;
mod browser;
mod cdp;
mod health;
mod lua_adapter;
mod mcp;
mod output;
mod pipeline;
mod sync;
mod template;

use std::collections::HashMap;

use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use serde_json::Value;

#[derive(Parser)]
#[command(
    name = "claw",
    about = "Make every website programmable by AI",
    version
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
    /// Show browser connection info (WebSocket URL)
    BrowserInfo,
    /// List available claws (website API specs)
    List,
    /// Download/update claws from GitHub
    Sync,
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

    /// One-shot page exploration — screenshot + interactive elements + forms + auth state
    Explore {
        /// URL to explore (navigates first). If omitted, explores the current page.
        url: Option<String>,
        /// Screenshot output path
        #[arg(short, long, default_value = "/tmp/claw-explore.png")]
        screenshot: String,
    },

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

    /// Download a URL to a local file
    Download {
        /// URL to download
        url: String,
        /// Output file path
        #[arg(short, long)]
        output: String,
    },

    // ---- MCP SERVER ----
    /// Run as MCP server (stdin/stdout JSON-RPC) for AI agent integration
    Mcp,

    // ---- FORGE META-TOOLS ----
    /// Execute a single pipeline step and return structured result
    #[command(name = "try-step")]
    TryStep {
        /// Pipeline step as YAML (e.g. 'navigate: https://example.com')
        step: String,
        /// Arguments as key=value pairs
        #[arg(long, value_delimiter = ',')]
        args: Vec<String>,
    },
    /// Verify a claw — dry-run and report per-step health
    #[command(name = "verify-adapter")]
    VerifyAdapter {
        /// Site name
        site: String,
        /// Claw name
        name: String,
        /// Arguments as --key value pairs (passed through)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        adapter_args: Vec<String>,
    },

    /// Health check: run claw and validate output against health/schema contracts
    Check {
        /// Site name (omit with --all)
        site: Option<String>,
        /// Claw name (omit with --all)
        name: Option<String>,
        /// Check all adapters that have health contracts
        #[arg(long)]
        all: bool,
        /// Arguments as --key value pairs (passed through)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        adapter_args: Vec<String>,
    },

    /// Save a claw YAML to ~/.claw/adapters/ (backs up previous version)
    #[command(name = "save-adapter")]
    SaveAdapter {
        /// Path to the claw YAML file to save
        file: String,
    },
    /// Rollback a claw to the previous version
    #[command(name = "rollback-adapter")]
    RollbackAdapter {
        /// Site name
        site: String,
        /// Claw name
        name: String,
    },

    /// Grab a website — auto-generate a claw from an API URL
    Grab {
        /// Site name (becomes directory under adapters/)
        #[arg(long)]
        site: String,
        /// Claw name (becomes filename.yaml)
        #[arg(long)]
        name: String,
        /// Description of what this claw does
        #[arg(long, default_value = "")]
        description: String,
        /// API URL to fetch
        #[arg(long)]
        url: String,
        /// JSON path to the array in the response (empty = response is already an array)
        #[arg(long, default_value = "")]
        select: String,
        /// Field mappings: output_name:source_path,... (__index = array index + 1)
        #[arg(long)]
        fields: String,
        /// Max results to return
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },

    /// Open a site in the browser for manual login (cookie persists in ~/.claw/chrome-profile/)
    Login {
        /// Site domain or site name (e.g. jimeng, xiaohongshu, bilibili)
        site: String,
    },

    /// Run a claw (implicit: claw <site> <name> [--args])
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
        Command::Download { url, output } => {
            let client = browser::connect_browser(cli.port, cli.headless).await?;
            let size = client.download_via_browser(&url, &output).await?;
            println!("{} ({} bytes)", output, size);
        }
        Command::Mcp => {
            mcp::serve(cli.port, cli.headless).await?;
        }
        Command::BrowserInfo => {
            browser::ensure_chrome(cli.port, cli.headless).await?;
            let ws_url = cdp::CdpClient::discover_ws_url(cli.port).await?;
            println!("{}", ws_url);
        }
        Command::Evaluate { expression } => {
            let client = browser::connect_browser(cli.port, cli.headless).await?;
            let result = client.evaluate(&expression).await?;
            let out = if result.is_string() {
                result.as_str().unwrap().to_string()
            } else {
                serde_json::to_string_pretty(&result)?
            };
            println!("{}", out);
        }
        Command::Navigate { url } => {
            let client = browser::connect_browser(cli.port, cli.headless).await?;
            client.navigate(&url).await?;
            println!("navigated to {}", url);
        }
        Command::Sync => {
            sync::sync_claws().await?;
        }
        Command::List => {
            if sync::needs_sync() {
                eprintln!("First run — syncing claws from GitHub...");
                if let Err(e) = sync::sync_claws().await {
                    eprintln!("Warning: sync failed ({}). Continuing with local claws.", e);
                }
            }
            let dirs = adapter::adapter_base_dirs();
            let refs: Vec<&str> = dirs.iter().map(|s| s.as_str()).collect();
            let adapters = adapter::list_adapters(&refs);
            if adapters.is_empty() {
                println!("No claws found. Run `claw sync` or add YAML files to ~/.claw/adapters/");
            } else {
                let columns = vec!["site".into(), "name".into(), "description".into()];
                let mut need_login: Vec<String> = Vec::new();
                let rows: Vec<std::collections::HashMap<String, String>> = adapters
                    .iter()
                    .map(|a| {
                        let mut row = std::collections::HashMap::new();
                        let site_display = if a.strategy == "public" {
                            a.site.clone()
                        } else {
                            if !need_login.contains(&a.site) {
                                need_login.push(a.site.clone());
                            }
                            format!("{} *", a.site)
                        };
                        row.insert("site".into(), site_display);
                        row.insert("name".into(), a.name.clone());
                        row.insert("description".into(), a.description.clone());
                        row
                    })
                    .collect();
                output::print_output(&columns, &rows, &cli.format)?;
                if !need_login.is_empty() {
                    eprintln!(
                        "\n* Need login first: {}",
                        need_login
                            .iter()
                            .map(|s| format!("claw login {}", s))
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
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
            let client = browser::connect_browser(cli.port, cli.headless).await?;
            if full_page {
                client.screenshot_full(&path).await?;
            } else {
                client.screenshot(&path).await?;
            }
            println!("{}", path);
        }
        Command::AxTree { depth } => {
            let client = browser::connect_browser(cli.port, cli.headless).await?;
            let tree = client.get_ax_tree(depth).await?;
            println!("{}", serde_json::to_string_pretty(&tree)?);
        }
        Command::ReadDom { selector, depth } => {
            let client = browser::connect_browser(cli.port, cli.headless).await?;
            let tree = client.get_dom_tree(selector.as_deref(), depth).await?;
            println!("{}", serde_json::to_string_pretty(&tree)?);
        }
        Command::PageInfo => {
            let client = browser::connect_browser(cli.port, cli.headless).await?;
            let info = client.get_page_info().await?;
            println!("{}", serde_json::to_string_pretty(&info)?);
        }

        Command::Explore { url, screenshot } => {
            let client = browser::connect_browser(cli.port, cli.headless).await?;
            if let Some(url) = url {
                client.navigate(&url).await?;
            }
            let result = client.explore(&screenshot).await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }

        // ---- PROBE (Discovery) ----
        Command::Find { query, role } => {
            let client = browser::connect_browser(cli.port, cli.headless).await?;
            let results = client.find_elements(&query, role.as_deref()).await?;
            println!("{}", serde_json::to_string_pretty(&results)?);
        }
        Command::ElementInfo { selector } => {
            let client = browser::connect_browser(cli.port, cli.headless).await?;
            let info = client.get_element_info(&selector).await?;
            println!("{}", serde_json::to_string_pretty(&info)?);
        }
        Command::EventListeners { selector } => {
            let client = browser::connect_browser(cli.port, cli.headless).await?;
            let listeners = client.get_event_listeners(&selector).await?;
            println!("{}", serde_json::to_string_pretty(&listeners)?);
        }
        Command::Cookies => {
            let client = browser::connect_browser(cli.port, cli.headless).await?;
            let cookies = client.get_cookies().await?;
            println!("{}", serde_json::to_string_pretty(&cookies)?);
        }
        Command::HitTest { x, y } => {
            let client = browser::connect_browser(cli.port, cli.headless).await?;
            let result = client.hit_test(x, y).await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::TopLayer => {
            let client = browser::connect_browser(cli.port, cli.headless).await?;
            let elements = client.get_top_layer().await?;
            println!("{}", serde_json::to_string_pretty(&elements)?);
        }
        Command::ForceState { selector, states } => {
            let client = browser::connect_browser(cli.port, cli.headless).await?;
            let state_refs: Vec<&str> = states.iter().map(|s| s.as_str()).collect();
            client.force_pseudo_state(&selector, &state_refs).await?;
            println!("forced {:?} on {}", states, selector);
        }
        Command::NetworkLog { action } => {
            let client = browser::connect_browser(cli.port, cli.headless).await?;
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
            let client = browser::connect_browser(cli.port, cli.headless).await?;
            client.hover_selector(&selector).await?;
            println!("hovered {}", selector);
        }
        Command::Scroll { selector } => {
            let client = browser::connect_browser(cli.port, cli.headless).await?;
            client.scroll_into_view(&selector).await?;
            println!("scrolled to {}", selector);
        }
        Command::PressKey { key, modifiers } => {
            let client = browser::connect_browser(cli.port, cli.headless).await?;
            client.press_key(&key, modifiers).await?;
            println!("pressed {}", key);
        }
        Command::Select { selector, value } => {
            let client = browser::connect_browser(cli.port, cli.headless).await?;
            client.select_option(&selector, &value).await?;
            println!("selected {} = {}", selector, value);
        }
        Command::Click { text } => {
            let client = browser::connect_browser(cli.port, cli.headless).await?;
            client.click_text(&text).await?;
            println!("clicked \"{}\"", text);
        }
        Command::ClickSelector { selector } => {
            let client = browser::connect_browser(cli.port, cli.headless).await?;
            client.click_selector(&selector).await?;
            println!("clicked {}", selector);
        }
        Command::Type { selector, text } => {
            let client = browser::connect_browser(cli.port, cli.headless).await?;
            client.type_into(&selector, &text).await?;
            println!("typed into {}", selector);
        }
        Command::DismissDialog {
            accept,
            prompt_text,
        } => {
            let client = browser::connect_browser(cli.port, cli.headless).await?;
            client
                .dismiss_dialog(accept, prompt_text.as_deref())
                .await?;
            println!("dialog {}", if accept { "accepted" } else { "dismissed" });
        }

        // ---- FORGE META-TOOLS ----
        Command::TryStep { step, args: kv } => {
            let parsed = adapter::parse_single_step(&step)?;

            // Parse key=value args
            let mut args = HashMap::new();
            for pair in &kv {
                if let Some((k, v)) = pair.split_once('=') {
                    args.insert(k.to_string(), Value::String(v.to_string()));
                }
            }

            let client = browser::connect_browser(cli.port, cli.headless).await?;

            let label = pipeline::step_label(&parsed);
            let start = std::time::Instant::now();
            let mut data = Vec::new();
            let mut rows = Vec::new();
            let result = pipeline::execute_single_step(
                &parsed,
                Some(&client),
                &args,
                &mut data,
                &mut rows,
                0,
            )
            .await;
            let duration_ms = start.elapsed().as_millis();

            let (status, error, suggestion) = match result {
                Ok(()) => ("pass".to_string(), None, None),
                Err(e) => {
                    let err_str = e.to_string();
                    let sug = pipeline::suggest_fix(&err_str);
                    ("fail".to_string(), Some(err_str), sug)
                }
            };
            let report = pipeline::StepResult {
                index: 0,
                step: label,
                status,
                duration_ms,
                error,
                suggestion,
                page_url: None,
                screenshot_path: None,
            };
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::VerifyAdapter {
            site,
            name,
            adapter_args,
        } => {
            let dirs = adapter::adapter_base_dirs();
            let refs: Vec<&str> = dirs.iter().map(|s| s.as_str()).collect();
            let ada = adapter::load_adapter(&refs, &site, &name)?;

            // Merge defaults + CLI args
            let mut args = HashMap::new();
            if let Some(ref defs) = ada.args {
                for (key, def) in defs {
                    if let Some(ref default) = def.default {
                        args.insert(key.clone(), default.clone());
                    }
                }
            }
            let cli_args = parse_adapter_args(&adapter_args);
            for (k, v) in cli_args {
                args.insert(k, v);
            }

            let client = browser::connect_browser(cli.port, cli.headless).await?;

            let results =
                pipeline::execute_with_report(&ada.pipeline, Some(&client), args, 0).await;

            let pass_count = results.iter().filter(|r| r.status == "pass").count();
            let total = results.len();

            println!("{}", serde_json::to_string_pretty(&results)?);
            eprintln!(
                "\n{}/{} steps passed ({})",
                pass_count,
                total,
                if pass_count == total {
                    "healthy"
                } else {
                    "BROKEN"
                }
            );

            if pass_count != total {
                std::process::exit(1);
            }
        }

        Command::Check {
            site,
            name,
            all,
            adapter_args,
        } => {
            let dirs = adapter::adapter_base_dirs();
            let refs: Vec<&str> = dirs.iter().map(|s| s.as_str()).collect();

            if all {
                let adapters = adapter::list_adapters(&refs);
                let mut reports = Vec::new();

                for info in &adapters {
                    let ada = match adapter::load_adapter(&refs, &info.site, &info.name) {
                        Ok(a) => a,
                        Err(_) => continue,
                    };
                    if ada.health.is_none() && ada.schema.is_none() {
                        continue;
                    }

                    let mut args = HashMap::new();
                    if let Some(ref defs) = ada.args {
                        for (key, def) in defs {
                            if let Some(ref default) = def.default {
                                args.insert(key.clone(), default.clone());
                            }
                        }
                    }

                    let needs_browser = ada.run.is_some() || ada.browser != Some(false);
                    let client = if needs_browser {
                        match browser::connect_browser(cli.port, cli.headless).await {
                            Ok(c) => Some(c),
                            Err(e) => {
                                reports.push(health::HealthReport {
                                    adapter: format!("{}/{}", info.site, info.name),
                                    status: health::HealthStatus::Broken,
                                    checks: vec![health::CheckResult {
                                        name: "execution".to_string(),
                                        passed: false,
                                        message: format!("browser connection failed: {}", e),
                                    }],
                                });
                                continue;
                            }
                        }
                    } else {
                        None
                    };

                    match adapter::run_adapter(client.as_ref(), &info.site, &info.name, args, 0)
                        .await
                    {
                        Ok((_cols, rows)) => {
                            reports.push(health::validate(&ada, &rows));
                        }
                        Err(e) => {
                            reports.push(health::HealthReport {
                                adapter: format!("{}/{}", info.site, info.name),
                                status: health::HealthStatus::Broken,
                                checks: vec![health::CheckResult {
                                    name: "execution".to_string(),
                                    passed: false,
                                    message: format!("pipeline error: {}", e),
                                }],
                            });
                        }
                    }
                }

                match cli.format.as_str() {
                    "json" => println!("{}", serde_json::to_string_pretty(&reports)?),
                    _ => {
                        let mut any_broken = false;
                        for r in &reports {
                            let symbol = match r.status {
                                health::HealthStatus::Healthy => "pass",
                                health::HealthStatus::Degraded => "warn",
                                health::HealthStatus::Broken => {
                                    any_broken = true;
                                    "FAIL"
                                }
                            };
                            eprintln!("[{}] {}", symbol, r.adapter);
                            for c in &r.checks {
                                eprintln!(
                                    "  {} {}: {}",
                                    if c.passed { "+" } else { "-" },
                                    c.name,
                                    c.message
                                );
                            }
                        }
                        if reports.is_empty() {
                            eprintln!("no adapters with health contracts found");
                        }
                        if any_broken {
                            std::process::exit(1);
                        }
                    }
                }
            } else {
                let site = site.ok_or("site is required (or use --all)")?;
                let name = name.ok_or("name is required (or use --all)")?;
                let ada = adapter::load_adapter(&refs, &site, &name)?;

                let mut args = HashMap::new();
                if let Some(ref defs) = ada.args {
                    for (key, def) in defs {
                        if let Some(ref default) = def.default {
                            args.insert(key.clone(), default.clone());
                        }
                    }
                }
                let cli_args = parse_adapter_args(&adapter_args);
                for (k, v) in cli_args {
                    args.insert(k, v);
                }

                let needs_browser = ada.run.is_some() || ada.browser != Some(false);
                let client = if needs_browser {
                    Some(browser::connect_browser(cli.port, cli.headless).await?)
                } else {
                    None
                };

                let (_, rows) = adapter::run_adapter(client.as_ref(), &site, &name, args, 0)
                    .await
                    .map_err(|e| -> Box<dyn std::error::Error> { e.to_string().into() })?;
                let report = health::validate(&ada, &rows);

                match cli.format.as_str() {
                    "json" => println!("{}", serde_json::to_string_pretty(&report)?),
                    _ => {
                        let symbol = match report.status {
                            health::HealthStatus::Healthy => "pass",
                            health::HealthStatus::Degraded => "warn",
                            health::HealthStatus::Broken => "FAIL",
                        };
                        eprintln!("[{}] {}", symbol, report.adapter);
                        for c in &report.checks {
                            eprintln!(
                                "  {} {}: {}",
                                if c.passed { "+" } else { "-" },
                                c.name,
                                c.message
                            );
                        }
                    }
                }

                if report.status == health::HealthStatus::Broken {
                    std::process::exit(1);
                }
            }
        }

        Command::SaveAdapter { file } => {
            // Parse the adapter to validate and extract site/name
            let content = std::fs::read_to_string(&file)?;
            let ada: adapter::Adapter = serde_yml::from_str(&content)?;

            let home = std::env::var("HOME")?;
            let dir = format!("{}/.claw/adapters/{}", home, ada.site);
            std::fs::create_dir_all(&dir)?;

            let target = format!("{}/{}.yaml", dir, ada.name);

            // Backup existing version if present
            if std::path::Path::new(&target).exists() {
                let backup = format!("{}.bak", target);
                std::fs::copy(&target, &backup)?;
                eprintln!("backed up previous version to {}", backup);
            }

            std::fs::copy(&file, &target)?;
            println!("saved {}/{} to {}", ada.site, ada.name, target);
        }
        Command::RollbackAdapter { site, name } => {
            let home = std::env::var("HOME")?;
            let target = format!("{}/.claw/adapters/{}/{}.yaml", home, site, name);
            let backup = format!("{}.bak", target);

            if !std::path::Path::new(&backup).exists() {
                return Err(format!("no backup found: {}", backup).into());
            }

            std::fs::copy(&backup, &target)?;
            println!("rolled back {}/{} from {}", site, name, backup);
        }

        Command::Grab {
            site,
            name,
            description,
            url,
            select,
            fields,
            limit,
        } => {
            let yaml =
                generate_grab_yaml(&site, &name, &description, &url, &select, &fields, limit);

            // Write to adapters/{site}/{name}.yaml
            let dir = format!("adapters/{}", site);
            std::fs::create_dir_all(&dir)?;
            let path = format!("{}/{}.yaml", dir, name);
            std::fs::write(&path, &yaml)?;

            // Print the generated YAML
            println!("{}", yaml);
            eprintln!("wrote {}", path);
        }

        Command::Login { site } => {
            let client = browser::connect_browser(cli.port, cli.headless).await?;

            let dirs = adapter::adapter_base_dirs();
            let refs: Vec<&str> = dirs.iter().map(|s| s.as_str()).collect();
            let url = adapter::resolve_login_url(&refs, &site);

            client.navigate(&url).await?;
            eprintln!("Opened {} — please log in the browser.", url);
            eprintln!("Press Enter when done...");

            let mut buf = String::new();
            std::io::stdin().read_line(&mut buf)?;
            eprintln!("Login saved. Cookie persists in ~/.claw/chrome-profile/");
        }

        Command::Adapter(raw_args) => {
            if raw_args.len() < 2 {
                return Err("usage: claw <site> <name> [--arg value ...]".into());
            }

            if sync::needs_sync() {
                eprintln!("First run — syncing claws from GitHub...");
                if let Err(e) = sync::sync_claws().await {
                    eprintln!("Warning: sync failed ({}). Continuing with local claws.", e);
                }
            }

            // Extract -f/--format and --each from raw args (clap doesn't parse external subcommand flags)
            let mut format = cli.format.clone();
            let mut each_arg: Option<String> = None;
            let mut filtered_args: Vec<String> = Vec::new();
            let mut i = 0;
            while i < raw_args.len() {
                if (raw_args[i] == "-f" || raw_args[i] == "--format") && i + 1 < raw_args.len() {
                    format = raw_args[i + 1].clone();
                    i += 2;
                } else if raw_args[i] == "--each" && i + 1 < raw_args.len() {
                    each_arg = Some(raw_args[i + 1].clone());
                    i += 2;
                } else {
                    filtered_args.push(raw_args[i].clone());
                    i += 1;
                }
            }
            let raw_args = filtered_args;

            let site = &raw_args[0];
            let name = &raw_args[1];

            let dirs = adapter::adapter_base_dirs();
            let refs: Vec<&str> = dirs.iter().map(|s| s.as_str()).collect();

            // Try Lua adapter first, then YAML
            let lua_path = adapter::find_lua_adapter(&refs, site, name);

            if let Some(lua_path) = lua_path {
                // Lua adapter
                let cli_args = parse_adapter_args(&raw_args[2..]);

                let client = browser::connect_browser(cli.port, cli.headless).await?;

                let (columns, rows) =
                    lua_adapter::execute_lua_adapter(&lua_path, client, cli_args, 0).await?;
                output::print_output(&columns, &rows, &format)?;
            } else {
                // YAML adapter
                let ada = adapter::load_adapter(&refs, site, name)?;

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

                // --each: split a comma-separated arg and run adapter multiple times
                let batch_values: Vec<String> = if let Some(ref each_key) = each_arg {
                    if let Some(val) = args.get(each_key) {
                        val.as_str()
                            .unwrap_or("")
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect()
                    } else {
                        return Err(format!("--each '{}': arg not found", each_key).into());
                    }
                } else {
                    vec![]
                };

                // Determine if browser is needed: explicit browser: true, inline Lua, or default (None = true for backward compat)
                let needs_browser = ada.run.is_some() || ada.browser != Some(false);

                // Build list of arg sets: single run or batch (--each splits comma-separated values)
                let arg_sets: Vec<HashMap<String, Value>> = if !batch_values.is_empty() {
                    let each_key = each_arg.as_ref().unwrap();
                    batch_values
                        .iter()
                        .map(|val| {
                            let mut run_args = args.clone();
                            run_args.insert(each_key.clone(), Value::String(val.clone()));
                            run_args
                        })
                        .collect()
                } else {
                    vec![args]
                };

                if needs_browser {
                    let client = browser::connect_browser(cli.port, cli.headless).await?;

                    let mut all_rows = Vec::new();
                    for run_args in arg_sets {
                        if batch_values.len() > 1 {
                            if let Some(ref ek) = each_arg {
                                eprintln!("  → {} = {}", ek, run_args[ek].as_str().unwrap_or(""));
                            }
                        }
                        if let Some(ref script) = ada.run {
                            let rows = lua_adapter::execute_inline_lua(
                                script,
                                &ada.columns,
                                client.clone(),
                                run_args,
                                0,
                            )
                            .await?;
                            all_rows.extend(rows);
                        } else {
                            let rows = pipeline::execute(&ada.pipeline, Some(&client), run_args, 0)
                                .await?;
                            all_rows.extend(rows);
                        }
                    }
                    output::print_output(&ada.columns, &all_rows, &format)?;
                } else {
                    let mut all_rows = Vec::new();
                    for run_args in arg_sets {
                        if batch_values.len() > 1 {
                            if let Some(ref ek) = each_arg {
                                eprintln!("  → {} = {}", ek, run_args[ek].as_str().unwrap_or(""));
                            }
                        }
                        let rows = pipeline::execute(&ada.pipeline, None, run_args, 0).await?;
                        all_rows.extend(rows);
                    }
                    output::print_output(&ada.columns, &all_rows, &format)?;
                }
            }
        }
    }
    Ok(())
}

/// A single field mapping parsed from --fields input.
struct FieldMapping {
    /// Column name shown in output
    column: String,
    /// Template expression for the map step value
    template: String,
}

/// Parse the --fields string into column names and map entries.
///
/// Format: "output_name:source_path,..." where:
/// - `name` alone means `name:name` (same source and output)
/// - `output:source` maps source to output column
/// - `__index` is special: replaced with array index + 1
/// - `source.method(args)` like `tags.join(', ')` is preserved as-is
///
/// Commas inside parentheses are not treated as field separators.
fn parse_fields(fields: &str) -> Vec<FieldMapping> {
    let entries = split_respecting_parens(fields);

    entries
        .iter()
        .filter(|s| !s.trim().is_empty())
        .map(|entry| {
            let entry = entry.trim();
            if let Some((col, source)) = entry.split_once(':') {
                let col = col.trim();
                let source = source.trim();
                if source == "__index" {
                    FieldMapping {
                        column: col.to_string(),
                        template: "__index".to_string(),
                    }
                } else {
                    FieldMapping {
                        column: col.to_string(),
                        template: format!("${{{{ item.{} }}}}", source),
                    }
                }
            } else {
                FieldMapping {
                    column: entry.to_string(),
                    template: format!("${{{{ item.{} }}}}", entry),
                }
            }
        })
        .collect()
}

/// Split a string on commas, but ignore commas inside parentheses.
fn split_respecting_parens(s: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut depth = 0u32;
    for ch in s.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ',' if depth == 0 => {
                result.push(current.clone());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

/// Generate a claw YAML string for a read-api adapter.
fn generate_grab_yaml(
    site: &str,
    name: &str,
    description: &str,
    url: &str,
    select: &str,
    fields: &str,
    limit: u32,
) -> String {
    let mappings = parse_fields(fields);
    let columns: Vec<&str> = mappings.iter().map(|m| m.column.as_str()).collect();

    // Build the map block lines
    let map_lines: Vec<String> = mappings
        .iter()
        .map(|m| {
            if m.template == "__index" {
                // __index gets resolved at evaluate time, so we wire it as item.__index
                format!("      {}: ${{{{ item.__index }}}}", m.column)
            } else {
                format!("      {}: {}", m.column, m.template)
            }
        })
        .collect();

    // Build the evaluate JS that handles __index injection
    let has_index = mappings.iter().any(|m| m.template == "__index");

    let today = "2026-03-28"; // last_forged timestamp

    let mut yaml = String::new();
    yaml.push_str(&format!("site: {}\n", site));
    yaml.push_str(&format!("name: {}\n", name));
    if !description.is_empty() {
        yaml.push_str(&format!("description: \"{}\"\n", description));
    }
    yaml.push_str("strategy: public\n");
    yaml.push_str("browser: false\n");
    yaml.push_str("version: \"1\"\n");
    yaml.push_str(&format!("last_forged: \"{}\"\n", today));
    yaml.push_str("forged_by: \"claw-forge\"\n");
    yaml.push('\n');
    yaml.push_str("args:\n");
    yaml.push_str("  limit:\n");
    yaml.push_str("    type: int\n");
    yaml.push_str(&format!("    default: {}\n", limit));
    yaml.push('\n');
    yaml.push_str(&format!("columns: [{}]\n", columns.join(", ")));
    yaml.push('\n');
    yaml.push_str("pipeline:\n");
    yaml.push_str(&format!("  - fetch: {}\n", url));

    if !select.is_empty() {
        yaml.push_str(&format!("  - select: {}\n", select));
    }

    if has_index {
        // Inject __index field via a transform step
        yaml.push_str("  - transform: |\n");
        yaml.push_str("      for i, row in ipairs(data) do\n");
        yaml.push_str("        row.__index = i\n");
        yaml.push_str("      end\n");
        yaml.push_str("      return data\n");
    }

    yaml.push_str("  - map:\n");
    for line in &map_lines {
        yaml.push_str(line);
        yaml.push('\n');
    }
    yaml.push_str("  - limit: ${{ args.limit }}\n");

    yaml
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

    #[test]
    fn parse_fields_simple() {
        let fields = parse_fields("title,url");
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].column, "title");
        assert_eq!(fields[0].template, "${{ item.title }}");
        assert_eq!(fields[1].column, "url");
        assert_eq!(fields[1].template, "${{ item.url }}");
    }

    #[test]
    fn parse_fields_with_source_path() {
        let fields = parse_fields("comments:comment_count");
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].column, "comments");
        assert_eq!(fields[0].template, "${{ item.comment_count }}");
    }

    #[test]
    fn parse_fields_with_index() {
        let fields = parse_fields("rank:__index,title");
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].column, "rank");
        assert_eq!(fields[0].template, "__index");
        assert_eq!(fields[1].column, "title");
        assert_eq!(fields[1].template, "${{ item.title }}");
    }

    #[test]
    fn parse_fields_with_dotted_source() {
        let fields = parse_fields("tags:tags.join(', ')");
        assert_eq!(fields[0].column, "tags");
        assert_eq!(fields[0].template, "${{ item.tags.join(', ') }}");
    }

    #[test]
    fn generate_grab_yaml_basic() {
        let yaml = generate_grab_yaml(
            "example",
            "feed",
            "Example feed",
            "https://api.example.com/feed",
            "data.items",
            "title,url",
            10,
        );
        assert!(yaml.contains("site: example"));
        assert!(yaml.contains("name: feed"));
        assert!(yaml.contains("description: \"Example feed\""));
        assert!(yaml.contains("browser: false"));
        assert!(yaml.contains("strategy: public"));
        assert!(yaml.contains("columns: [title, url]"));
        assert!(yaml.contains("- fetch: https://api.example.com/feed"));
        assert!(yaml.contains("- select: data.items"));
        assert!(yaml.contains("      title: ${{ item.title }}"));
        assert!(yaml.contains("      url: ${{ item.url }}"));
        assert!(yaml.contains("- limit: ${{ args.limit }}"));
        assert!(yaml.contains("default: 10"));
        // No transform step when no __index
        assert!(!yaml.contains("transform"));
    }

    #[test]
    fn generate_grab_yaml_no_select() {
        let yaml = generate_grab_yaml(
            "lobsters",
            "hot",
            "",
            "https://lobste.rs/hottest.json",
            "",
            "title,score",
            20,
        );
        // No select step when select is empty
        assert!(!yaml.contains("- select:"));
        // No description when empty
        assert!(!yaml.contains("description:"));
    }

    #[test]
    fn generate_grab_yaml_with_index() {
        let yaml = generate_grab_yaml(
            "lobsters",
            "hot",
            "Lobsters hot posts",
            "https://lobste.rs/hottest.json",
            "",
            "rank:__index,title,score,tags:tags.join(', '),comments:comment_count",
            20,
        );
        assert!(yaml.contains("columns: [rank, title, score, tags, comments]"));
        assert!(yaml.contains("- transform:"));
        assert!(yaml.contains("row.__index = i"));
        assert!(yaml.contains("      rank: ${{ item.__index }}"));
        assert!(yaml.contains("      title: ${{ item.title }}"));
        assert!(yaml.contains("      tags: ${{ item.tags.join(', ') }}"));
        assert!(yaml.contains("      comments: ${{ item.comment_count }}"));
    }
}
