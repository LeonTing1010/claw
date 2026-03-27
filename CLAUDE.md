# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Claw is a Rust CLI tool that turns websites into deterministic command-line interfaces using Chrome DevTools Protocol (CDP) for native browser control. It's compatible with [opencli](https://github.com/jackwener/opencli) YAML adapters but replaces JS event simulation with CDP-level mouse/keyboard/network events, solving SPA framework compatibility issues.

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

Three-layer design:

1. **Layer 1 — CDP Client**: WebSocket connection to Chrome DevTools Protocol. All browser interactions (click, type, upload, network intercept) go through CDP native events, not JavaScript injection. This is the core differentiator over opencli.

2. **Layer 2 — Pipeline Engine**: Executes adapter steps sequentially. Supports opencli-compatible steps (`navigate`, `wait`, `evaluate`, `map`, `limit`, `intercept`) plus claw-native steps (`click`, `click_selector`, `type`, `upload`, `screenshot`). Template variables use `${{ args.name }}` syntax.

3. **Layer 3 — Adapters**: Two types, both hot-loadable from `~/.claw/adapters/` or `./adapters/`:
   - **YAML adapters** — 100% opencli-compatible format
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

- `src/main.rs` — CLI entry point (clap), subcommand dispatch
- `src/cdp.rs` — CDP WebSocket client, browser discovery, high-level operations (navigate, evaluate)

## Test Conventions

Rust `#[cfg(test)] mod tests` inside each source file. No external tag system — use module path to filter:

```bash
cargo test cdp::tests                        # Run cdp module tests only
cargo test cdp::tests::pick_page_ws_url      # Run a single test
```

Business logic tests should be named descriptively: `{module}_{what_it_verifies}`, e.g., `pick_page_ws_url_selects_page_not_browser`.

## Status

Phase 1 core implemented: CDP WebSocket client, browser discovery, evaluate/navigate commands. See `DESIGN.md` for the full technical design.
