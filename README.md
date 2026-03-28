# Claw

> **Make every website programmable by AI.**

Websites are closing their APIs. AI agents need them more than ever. Claw bridges the gap.

AI agents grab websites and turn them into **claws** — deterministic, machine-generated API specs that run with zero AI. One agent grabs a website, every agent benefits.

```
AI agent ──grab──→ claw (YAML/Lua) ──run──→ structured data
         (once)                     (1000x, free)
```

The long-term value: even as AI agents get stronger, Claw eliminates redundant discovery across all agents. One grab, shared by all — like Wikipedia for web APIs.

## Install

```bash
# One-line install
curl -fsSL https://raw.githubusercontent.com/LeonTing1010/claw/master/install.sh | sh

# Or from source
cargo install --git https://github.com/LeonTing1010/claw

# Or download binary from GitHub Releases
# https://github.com/LeonTing1010/claw/releases
```

## Quick Start

```bash
claw list                              # See available claws (auto-syncs on first run)
claw weibo hot                         # 微博热搜
claw bilibili hot --limit 5 -f json    # B站热门 → JSON
claw trending scan                     # 13+ 平台热搜聚合
claw v2ex hot                          # V2EX 热门话题

# Grab a new website (API-first)
claw grab --site mysite --name feed \
  --url "https://api.example.com/feed" \
  --fields "title,author,score"

# Write operations (CDP native — works on SPAs)
claw xiaohongshu publish \
  --title "标题" --content "正文" \
  --images "/path/to/image.webp"
```

Chrome launches automatically. No manual setup needed.

## How It Works

### Grab and Run

**Grab** (one-time): AI agent uses 28 MCP tools to explore a website — screenshot, read DOM, try interactions, discover APIs — then outputs a claw (YAML/Lua).

**Run** (every time, zero AI): Claw loads the claw and executes it deterministically. No tokens, sub-second, works 1000x without drift.

### Why CDP Native

```
JS dispatchEvent()          → React/Vue ignore it → silent failure
CDP Input.dispatchMouseEvent → browser-native      → works everywhere
```

This is the difference between "works on static sites" and "works on any website."

### API-first Claws

For websites with public APIs, claws run without a browser at all:

```yaml
browser: false
pipeline:
  - fetch: https://lobste.rs/hottest.json
  - map:
      title: ${{ item.title }}
      score: ${{ item.score }}
  - limit: ${{ args.limit }}
```

No Chrome, no navigation — pure HTTP, sub-100ms execution.

## Claws

A claw is an API spec for a website. YAML for reads, Lua for complex interactions:

```yaml
site: weibo
name: hot
description: 微博热搜榜
strategy: public
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
| **Control** | `if_selector`, `if_text`, `if_url`, `use` (compose claws) |
| **Assert** | `assert_selector`, `assert_text`, `assert_url`, `assert_not_selector` |

### Lua Transform

```yaml
- transform: |
    data = sort_by(data, "views", "desc")
    data = unique_by(data, "title")
    return limit(data, 10)
```

Helpers: `sort_by`, `limit`, `pick`, `group_by`, `unique_by`.

### Lua Claws

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

Claw exposes 30 tools as an MCP server — the primary interface for AI agents:

```bash
claw mcp    # Start MCP server (stdin/stdout JSON-RPC)
```

**Discover and run claws:**
- `list_adapters` — what websites are available
- `run_adapter` — execute a claw, get structured JSON

**Grab toolkit (28 tools):**
- See: `screenshot`, `ax_tree`, `read_dom`, `explore`, `page_info`
- Probe: `find`, `element_info`, `network_log_start/dump`, `cookies`
- Try: `click`, `type_text`, `navigate`, `evaluate`, `hover`, `scroll`
- Verify: `try_step`, `verify_adapter`

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
# Run claws
claw <site> <name> [--args] [-f json|csv|yaml|md]
claw trending scan --platforms "weibo,bilibili,hackernews"

# Grab new claws
claw grab --site X --name Y --url "API_URL" --fields "title,score"

# Sync shared claws from GitHub
claw sync

# Grab toolkit (for AI agents or manual exploration)
claw screenshot /tmp/page.png
claw ax-tree
claw explore https://example.com
claw find "Submit" --role button

# Claw management
claw list
claw verify-adapter weibo hot
claw save-adapter ./my-claw.yaml
claw rollback-adapter weibo hot

# System
claw doctor
claw login weibo
claw completions zsh
```

## Output Formats

```bash
claw weibo hot                    # Table (default)
claw weibo hot -f json            # JSON
claw weibo hot -f csv > hot.csv   # CSV
claw weibo hot -f yaml            # YAML
claw weibo hot -f md              # Markdown
```

## Building

```bash
cargo build              # Build
cargo test               # 108 tests
cargo clippy             # Lint
cargo fmt -- --check     # Format check
```

## License

MIT
