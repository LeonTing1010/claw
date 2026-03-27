mod adapter;
mod cdp;
mod output;
mod pipeline;
mod template;

use std::collections::HashMap;

use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use serde_json::Value;

#[derive(Parser)]
#[command(name = "claw", about = "Turn any website into a CLI — with native browser precision")]
#[command(allow_external_subcommands = true)]
struct Cli {
    /// Chrome CDP debugging port
    #[arg(long, default_value_t = 9222, global = true)]
    port: u16,

    /// Output format: table, json, csv
    #[arg(short = 'f', long, default_value = "table", global = true)]
    format: String,

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
            let ws_url = cdp::CdpClient::discover_ws_url(cli.port).await?;
            println!("{}", ws_url);
        }
        Command::Evaluate { expression } => {
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
                let rows: Vec<std::collections::HashMap<String, String>> = adapters.iter().map(|a| {
                    let mut row = std::collections::HashMap::new();
                    row.insert("site".into(), a.site.clone());
                    row.insert("name".into(), a.name.clone());
                    row.insert("description".into(), a.description.clone());
                    row
                }).collect();
                output::print_output(&columns, &rows, &cli.format)?;
            }
        }
        Command::Completions { shell } => {
            let mut cmd = Cli::command();
            generate(shell, &mut cmd, "claw", &mut std::io::stdout());
        }
        Command::Doctor => {
            // 1. TCP connectivity
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
                    let targets: Vec<serde_json::Value> = serde_json::from_str(&body).unwrap_or_default();
                    let pages = targets.iter().filter(|t| t["type"].as_str() == Some("page")).count();
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
                Ok(ws_url) => {
                    match cdp::CdpClient::connect(&ws_url).await {
                        Ok(client) => {
                            match client.evaluate("1+1").await {
                                Ok(val) if val == 2 => println!("[ok] JavaScript evaluation working"),
                                Ok(val) => println!("[warn] Unexpected eval result: {}", val),
                                Err(e) => println!("[fail] JS evaluation failed: {}", e),
                            }
                        }
                        Err(e) => println!("[fail] WebSocket connection failed: {}", e),
                    }
                }
                Err(e) => println!("[fail] Cannot discover page: {}", e),
            }
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
        let raw: Vec<String> = vec!["--limit", "5"]
            .into_iter()
            .map(String::from)
            .collect();
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
        let raw: Vec<String> = vec!["--verbose"]
            .into_iter()
            .map(String::from)
            .collect();
        let args = parse_adapter_args(&raw);
        assert_eq!(args.get("verbose"), Some(&json!(true)));
    }
}
