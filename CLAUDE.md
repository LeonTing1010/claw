# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Claw is the **web capability cache for AI agents**: forge once, execute forever. AI agents explore websites and forge deterministic claws (YAML/Lua) — machine-generated API specs for websites that never built an API. At runtime, Claw executes claws with zero AI: no tokens, no latency, no non-determinism. The primary user is the AI agent, not the human. See `.claude/DESIGN.md` for full design.

## Development Workflow

All code changes **must** follow the Constraint-Driven Development (CDD) flow:

1. **RED** — Write failing tests first that define the expected behavior
2. **GREEN** — Write the minimal implementation to make tests pass
3. **Why** — Review: does the implementation match the intent? Refactor if needed

Use the `/constraint-driven-development` skill for any feature, bug fix, or business logic change. Do not skip straight to implementation.

## Best Practices

### Code

- **CDP native events, never JS injection.** Click/type/hover go through `Input.dispatchMouseEvent`/`Input.dispatchKeyEvent`. Never `element.click()` or `dispatchEvent()` — React/Vue ignore them.
- **Don't add dependencies for what existing tools already do.** Claw has Lua (mlua) for data transforms — don't add jq/jaq/etc. Check what's already in the binary before proposing new crates.
- **Pipeline steps are the API surface.** Adding a step means: enum variant + YAML deserializer + pipeline executor + step_label + FORGING.md update. Don't add steps lightly.
- **Template rendering happens per-step.** Each step renders its own `${{ }}` templates. This is intentionally repetitive — don't extract a shared pre-render pass (steps need different contexts).

### Claws (Adapters)

- **API > UI. Always.** A fetch-based claw is faster, more reliable, and won't break on UI redesigns. Only use browser DOM when there's no API. See `FORGING.md` for the full decision tree.
- **One claw = one capability.** `weibo/hot` returns hot search. `xiaohongshu/publish` publishes a post. Don't bundle unrelated operations.
- **Use `claw.run()` for composition, not inline duplication.** If claw A needs data from claw B, call `claw.run("site", "name")` — don't copy B's JS into A.
- **Test with `verify-adapter`.** Every claw must pass `claw verify-adapter site name` before shipping.

### Architecture

- **User-facing text says "claw", code says "adapter".** The brand name is "claw" (as in: "forge a claw for weibo"). Internal variables, function names, file paths keep `adapter` to avoid a massive rename. Don't rename code internals.
- **MCP tools are the primary AI interface.** `list_adapters` and `run_adapter` for execution, 28 forge tools for creation. CLI is secondary (for humans).
- **Don't optimize mechanical repetition.** The 25 `TemplateContext` blocks in pipeline.rs, the deserialization match branches — these are intentionally verbose. They don't drift, don't cause bugs, and are easier to read than macros/helpers. See engineering philosophy §8 Core Warning.

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

## Verification Commands

| Gate | Command | Notes |
|------|---------|-------|
| typecheck | `cargo check` | Rust compiler |
| lint | `cargo clippy` | Clippy lints |
| format | `cargo fmt -- --check` | Rustfmt |
| tests | `cargo test` | All unit/integration tests |

## Project Structure

- `src/main.rs` — CLI entry point (clap), subcommand dispatch
- `src/cdp.rs` — CDP WebSocket client: connect, navigate, evaluate, click, type, upload
- `src/adapter.rs` — YAML adapter parser, PipelineStep enum, load/list/run adapters
- `src/pipeline.rs` — Pipeline executor: all step types + Lua transform
- `src/lua_adapter.rs` — Lua runtime: page API, transform helpers, JSON↔Lua conversion
- `src/mcp.rs` — MCP server: 30 tools over stdin/stdout JSON-RPC
- `src/sync.rs` — GitHub sync: download claws to ~/.claw/adapters/
- `src/template.rs` — `${{ }}` template engine (args/item resolution)
- `src/output.rs` — Output formatter (table/json/csv/yaml/md)
- `src/browser.rs` — Chrome launch/detection (macOS + Linux)
- `adapters/` — Claw YAML definitions
- `FORGING.md` — Claw authoring spec for AI agents

## Test Conventions

Rust `#[cfg(test)] mod tests` inside each source file. Use module path to filter:

```bash
cargo test cdp::tests                        # Run cdp module tests only
cargo test cdp::tests::pick_page_ws_url      # Run a single test
```

Test names: `{module}_{what_it_verifies}`, e.g., `pick_page_ws_url_selects_page_not_browser`.
