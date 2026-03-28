# Claw

> **The web capability cache for AI agents.** Forge once, execute forever.

AI agents explore websites and forge deterministic adapters — machine-generated API specs for sites that never built an API. At runtime, Claw executes them with zero AI: no tokens, no latency, no drift.

```
AI agent ──forge──→ adapter (YAML/Lua) ──execute──→ structured data
          (once)                         (1000x, free)
```

Single Rust binary. Zero dependencies. CDP-native precision that works on React/Vue/Angular SPAs.

## Install

```bash
# From source
cargo install --git https://github.com/LeonTing1010/claw

# Or download from GitHub Releases
# https://github.com/LeonTing1010/claw/releases
```

## Quick Start

```bash
claw list                              # See available adapters
claw weibo hot                         # 微博热搜
claw bilibili hot --limit 5 -f json    # B站热门 → JSON
claw trending scan                     # 13 平台热搜聚合，标记新词

# Write operations (CDP native — works on SPAs)
claw xiaohongshu publish \
  --title "标题" --content "正文" \
  --images "/path/to/image.webp"
```

Chrome launches automatically. No manual setup needed.

## How It Works

### Two Phases

**Forge** (one-time, AI-driven): Agent uses 28 MCP tools to explore a website — screenshot, read DOM, try interactions, verify results — then outputs a YAML/Lua adapter.

**Execute** (every time, zero AI): Claw loads the adapter and runs it deterministically via Chrome DevTools Protocol.

### Why CDP Native

```
JS dispatchEvent()         → React/Vue ignore it → silent failure
CDP Input.dispatchMouseEvent → browser-native     → works everywhere
```

This is the difference between "works on static sites" and "works on any website."

## Architecture

```
┌──────────────────────────────────────────────────┐
│  Layer 3: Adapters                               │
│  YAML + Lua, hot-loadable, composable            │
├──────────────────────────────────────────────────┤
│  Layer 2: Pipeline Engine                        │
│  34 step types, Lua transform, template engine   │
├──────────────────────────────────────────────────┤
│  Layer 1: CDP Client                             │
│  Native mouse/keyboard/network/upload            │
└──────────────────────────────────────────────────┘
```

## Adapters

An adapter is an API spec for a website. YAML for reads, Lua for complex interactions:

```yaml
site: weibo
name: hot
description: 微博热搜榜
browser: true
args:
  limit: { type: int, default: 20 }
columns: [rank, title, hot]
pipeline:
  - navigate: https://weibo.com
  - evaluate: |
      (async () => {
        const res = await fetch('/ajax/side/hotSearch');
        const data = await res.json();
        return data.data.realtime.map((item, i) => ({
          rank: i + 1, title: item.note, hot: item.num || 0
        }));
      })()
  - map:
      rank: ${{ item.rank }}
      title: ${{ item.title }}
      hot: ${{ item.hot }}
  - limit: ${{ args.limit }}
```

### Pipeline Steps

| Category | Steps |
|----------|-------|
| **Extract** | `evaluate`, `fetch`, `intercept`, `select` (path) |
| **Transform** | `map`, `filter`, `limit`, `transform` (Lua: sort_by, group_by, unique_by, pick) |
| **Browser** | `navigate`, `click`, `click_selector`, `type`, `upload`, `hover`, `scroll`, `press_key`, `select` (dropdown), `dismiss_dialog` |
| **Wait** | `wait`, `wait_for` (selector/text/url/network_idle) |
| **Control** | `if_selector`, `if_text`, `if_url`, `use` (compose adapters) |
| **Assert** | `assert_selector`, `assert_text`, `assert_url`, `assert_not_selector` |

### Lua Transform

For data operations that YAML can't express:

```yaml
- transform: |
    data = sort_by(data, "views", "desc")
    data = unique_by(data, "title")
    return limit(data, 10)
```

Built-in helpers: `sort_by`, `limit`, `pick`, `group_by`, `unique_by`.

### Lua Adapters

For complex multi-step UI flows:

```yaml
site: telegraph
name: publish
columns: [status, url]
run: |
  page:goto("https://telegra.ph")
  page:type_into(".tl_article_edit #_tl_editor", args.content)
  page:click_text("Publish")
  page:wait(3)
  local url = page:evaluate("location.href")
  return {{ status = "published", url = url }}
```

## For AI Agents (MCP)

Claw exposes all 28 forge tools as an MCP server:

```bash
claw mcp    # Start MCP server (stdin/stdout JSON-RPC)
```

Tools: `screenshot`, `ax_tree`, `read_dom`, `explore`, `find`, `element_info`, `click`, `type_text`, `navigate`, `evaluate`, `try_step`, `verify_adapter`, and more.

Configure in your AI client:

```json
{
  "mcpServers": {
    "claw": {
      "command": "claw",
      "args": ["mcp"]
    }
  }
}
```

## Commands

```bash
# Run adapters
claw <site> <name> [--args] [-f json|csv|yaml|md]
claw trending scan --platforms "weibo,bilibili,hackernews"

# Forge tools (for AI agents or manual exploration)
claw screenshot /tmp/page.png
claw ax-tree
claw read-dom --selector "main"
claw explore https://example.com
claw find "Submit" --role button
claw evaluate "document.title"

# Adapter management
claw list
claw verify-adapter weibo hot
claw save-adapter ./my-adapter.yaml
claw rollback-adapter weibo hot

# System
claw doctor
claw login weibo
claw completions zsh
```

## Output Formats

```bash
claw weibo hot                    # Table (default)
claw weibo hot -f json            # JSON (pipe to jq)
claw weibo hot -f csv > hot.csv   # CSV
claw weibo hot -f yaml            # YAML
claw weibo hot -f md              # Markdown
```

## Building

```bash
cargo build              # Build
cargo test               # 86 tests
cargo clippy             # Lint
cargo fmt -- --check     # Format check
```

## License

MIT
