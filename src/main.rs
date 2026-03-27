mod adapter;
mod cdp;
mod output;
mod pipeline;
mod template;

use std::collections::HashMap;

use clap::{Parser, Subcommand};
use serde_json::Value;

#[derive(Parser)]
#[command(name = "claw", about = "Turn any website into a CLI — with native browser precision")]
#[command(allow_external_subcommands = true)]
struct Cli {
    /// Chrome CDP debugging port
    #[arg(long, default_value_t = 9222, global = true)]
    port: u16,

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
        Command::Adapter(raw_args) => {
            if raw_args.len() < 2 {
                return Err("usage: claw <site> <name> [--arg value ...]".into());
            }
            let site = &raw_args[0];
            let name = &raw_args[1];

            let home = std::env::var("HOME").unwrap_or_default();
            let base_dirs = vec![
                "adapters".to_string(),
                format!("{}/.claw/adapters", home),
            ];
            let base_refs: Vec<&str> = base_dirs.iter().map(|s| s.as_str()).collect();

            let ada = adapter::load_adapter(&base_refs, site, name)?;

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
            output::print_table(&ada.columns, &rows);
        }
    }
    Ok(())
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
