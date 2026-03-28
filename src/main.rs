#![recursion_limit = "256"]
mod adapter;
mod bridge;
mod cdp;
mod health;
mod mcp;
mod output;
mod sync;

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
    /// Output format: table, json, csv
    #[arg(short = 'f', long, default_value = "table", global = true)]
    format: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List available claws (website API specs)
    List,
    /// Download/update claws from GitHub
    Sync,
    /// Generate shell completions
    Completions {
        /// Shell: bash, zsh, fish, powershell, elvish
        shell: Shell,
    },

    /// Health check all claws via extension bridge
    Check,

    // ---- MCP SERVER (primary interface for AI agents) ----
    /// Run as MCP server (stdin/stdout JSON-RPC) for AI agent integration
    Mcp,

    /// Run a claw via extension bridge (claw <site> <name> [--arg value ...])
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
        Command::Mcp => {
            mcp::serve().await?;
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
        Command::Check => {
            // Connect to extension bridge
            let client = bridge::try_extension_bridge().await?;

            // Get claw list from extension
            let list_result = client
                .send("Claw.list", Some(serde_json::json!({})))
                .await?;

            let claws = list_result
                .get("claws")
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_default();

            if claws.is_empty() {
                println!("No claws registered in extension.");
                return Ok(());
            }

            let mut healthy = 0;
            let mut degraded = 0;
            let mut broken = 0;
            let mut errors = 0;

            for claw in &claws {
                let site = claw["site"].as_str().unwrap_or("?");
                let name = claw["name"].as_str().unwrap_or("?");
                let adapter_name = format!("{}/{}", site, name);

                // Run the claw
                let run_result = client
                    .send(
                        "Claw.run",
                        Some(serde_json::json!({
                            "site": site,
                            "name": name,
                            "args": {}
                        })),
                    )
                    .await;

                match run_result {
                    Err(e) => {
                        println!("{} — Error: {}", adapter_name, e);
                        errors += 1;
                    }
                    Ok(result) => {
                        if let Some(err) = result.get("error") {
                            println!("{} — Error: {}", adapter_name, err);
                            errors += 1;
                            continue;
                        }

                        let rows = result
                            .get("rows")
                            .and_then(|r| r.as_array())
                            .cloned()
                            .unwrap_or_default();

                        // Try to get health contract from result
                        let health_contract = result
                            .get("health")
                            .and_then(adapter::parse_health_contract);

                        if let Some(contract) = health_contract {
                            let report = health::validate(&adapter_name, &contract, &rows);
                            let status_str = match report.status {
                                health::HealthStatus::Healthy => {
                                    healthy += 1;
                                    "Healthy"
                                }
                                health::HealthStatus::Degraded => {
                                    degraded += 1;
                                    "Degraded"
                                }
                                health::HealthStatus::Broken => {
                                    broken += 1;
                                    "Broken"
                                }
                            };
                            let failures: Vec<&str> = report
                                .checks
                                .iter()
                                .filter(|c| !c.passed)
                                .map(|c| c.message.as_str())
                                .collect();
                            if failures.is_empty() {
                                println!("{} — {} ({} rows)", adapter_name, status_str, rows.len());
                            } else {
                                println!(
                                    "{} — {} ({})",
                                    adapter_name,
                                    status_str,
                                    failures.join("; ")
                                );
                            }
                        } else {
                            // No health contract — just report row count
                            healthy += 1;
                            println!(
                                "{} — OK ({} rows, no health contract)",
                                adapter_name,
                                rows.len()
                            );
                        }
                    }
                }
            }

            println!(
                "\n{} claws: {} healthy, {} degraded, {} broken, {} errors",
                claws.len(),
                healthy,
                degraded,
                broken,
                errors
            );

            // Exit code: 0 = all healthy, 1 = degraded, 2 = broken/errors
            if broken > 0 || errors > 0 {
                std::process::exit(2);
            } else if degraded > 0 {
                std::process::exit(1);
            }
        }
        Command::Completions { shell } => {
            let mut cmd = Cli::command();
            generate(shell, &mut cmd, "claw", &mut std::io::stdout());
        }

        Command::Adapter(raw_args) => {
            if raw_args.len() < 2 {
                return Err("usage: claw <site> <name> [--arg value ...]".into());
            }

            let site = &raw_args[0];
            let name = &raw_args[1];
            let args = parse_adapter_args(&raw_args[2..]);

            // Run via Chrome extension bridge
            let client = bridge::try_extension_bridge().await?;
            let result = client
                .send(
                    "Claw.run",
                    Some(serde_json::json!({
                        "site": site,
                        "name": name,
                        "args": args
                    })),
                )
                .await?;

            println!("{}", serde_json::to_string_pretty(&result)?);
        }
    }
    Ok(())
}

/// Parse --key value pairs from raw CLI args into a HashMap.
fn parse_adapter_args(raw: &[String]) -> std::collections::HashMap<String, Value> {
    let mut args = std::collections::HashMap::new();
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
