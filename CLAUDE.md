# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Claw is a precision browser-automation instrument: **agent-forged, machine-executed**. AI agents explore websites and forge deterministic adapters (YAML/Lua); at execution time, Claw runs those adapters with zero AI involvement — no tokens, no latency, no non-determinism. Built in Rust with Chrome DevTools Protocol (CDP) for native browser control, it delivers CDP-level mouse/keyboard/network precision that works on React/Vue/Angular SPAs where JS `dispatchEvent()` fails.

## Build & Development Commands

```bash
cargo build              # Build the project
cargo run                # Run the CLI
cargo test               # Run all tests
cargo test <test_name>   # Run a single test
cargo clippy             # Lint
cargo fmt                # Format code
cargo fmt -- --check     # Check formatting without modifying
```

## Architecture

Two modes, four layers:

- **Forge mode** (development-time): Agent explores sites, tries interactions, observes results, iterates until the adapter is precise. Expensive but one-time.
- **Execute mode** (runtime): Claw loads the forged adapter and runs it deterministically. Zero AI, sub-second, runs 1000x without drift.

Layers:

0. **Layer 0 — Forge Toolkit** (dev-time only): `screenshot`, `read-dom`, `explore`, `record`, `try` — the Agent's eyes and hands during adapter creation.

1. **Layer 1 — CDP Client**: WebSocket connection to Chrome DevTools Protocol. All browser interactions (click, type, upload, network intercept) go through CDP native events, not JavaScript injection.

2. **Layer 2 — Pipeline Engine**: Executes adapter steps sequentially. Supports opencli-compatible steps (`navigate`, `wait`, `evaluate`, `map`, `limit`, `intercept`) plus claw-native steps (`click`, `click_selector`, `type`, `upload`, `screenshot`). Template variables use `${{ args.name }}` syntax.

3. **Layer 3 — Adapters**: Agent-forged, hot-loadable from `~/.claw/adapters/` or `./adapters/`:
   - **YAML adapters** — opencli-compatible format (superset)
   - **Lua adapters** — for complex multi-step UI flows, using `mlua` crate with Lua 5.4

## Key Dependencies

- `tokio` + `tokio-tungstenite` — async runtime and WebSocket for CDP
- `serde` + `serde_json` — CDP message serialization
- `clap` (derive) — CLI argument parsing
- `futures-util` — WebSocket stream splitting (SinkExt/StreamExt)

## Design Decisions

- CDP native events over JS `dispatchEvent()` — React/Vue SPAs ignore synthetic JS events but respond to CDP `Input.dispatchMouseEvent`/`Input.dispatchKeyEvent`
- Lua over TypeScript for complex adapters — embeds natively in Rust via `mlua` (~2MB), sandboxable, hot-reloadable, simpler for AI generation
- Browser connection supports both opencli's Browser Bridge extension (WebSocket) and direct CDP via `--remote-debugging-port`
- Auth uses a 5-tier strategy (public → cookie → header → intercept → ui), opencli-compatible
- Output formats: table (default), json, yaml, csv, md

## Verification Commands

| Gate | Command | Notes |
|------|---------|-------|
| typecheck | `cargo check` | Rust compiler |
| lint | `cargo clippy` | Clippy lints |
| format | `cargo fmt -- --check` | Rustfmt |
| tests | `cargo test` | All unit/integration tests |
| diff-size | (built-in, no command) | Default limits |
| security | (built-in, no command) | Scans for secrets |

## Project Structure

- `src/main.rs` — CLI entry point (clap), subcommand dispatch, adapter arg parsing
- `src/cdp.rs` — CDP WebSocket client: connect, navigate, evaluate, click, type, upload
- `src/adapter.rs` — YAML adapter parser, PipelineStep enum, load_adapter()
- `src/template.rs` — `${{ }}` template engine (args/item resolution)
- `src/pipeline.rs` — Pipeline executor: navigate/evaluate/map/limit/click/type/upload/wait
- `src/output.rs` — Table output formatter (tabled Builder API)
- `adapters/` — YAML adapter definitions (bilibili, xiaohongshu, demo)

## Test Conventions

Rust `#[cfg(test)] mod tests` inside each source file. No external tag system — use module path to filter:

```bash
cargo test cdp::tests                        # Run cdp module tests only
cargo test cdp::tests::pick_page_ws_url      # Run a single test
```

Business logic tests should be named descriptively: `{module}_{what_it_verifies}`, e.g., `pick_page_ws_url_selects_page_not_browser`.

## Status

Phases 1-5 complete (55 tests passing). Full stack: CDP precision execution + forge toolkit (28 scalpels) + meta-tools (try-step, verify-adapter) + adapter expressiveness (conditionals, assertions) + MCP server (`claw mcp`, 26 tools) + adapter versioning (save/rollback) + Lua runtime (mlua, 18 page API methods). 34 CLI subcommands. Remaining: docs + crates.io release. See `.claude/DESIGN.md` for full design and philosophy.
