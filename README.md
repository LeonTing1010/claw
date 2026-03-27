# Claw

> Turn any website into a CLI — with native browser precision.

Single binary, zero dependencies. Transforms websites into deterministic command-line interfaces via Chrome DevTools Protocol.

## Install

Download from [GitHub Releases](https://github.com/LeonTing1010/claw/releases), or build from source:

```bash
cargo install --git https://github.com/LeonTing1010/claw
```

## Usage

```bash
claw list                              # See available adapters
claw bilibili hot --limit 5            # B站热门视频
claw bilibili hot --limit 3 -f json    # JSON output (pipe to jq)
claw bilibili hot -f csv > hot.csv     # CSV export

claw xiaohongshu publish \
  --title "标题" \
  --content "正文" \
  --images "/path/to/image.webp"       # Publish to Xiaohongshu
```

Chrome launches automatically on first run. No manual setup needed.

## Why Claw

opencli proved that pre-built adapters beat LLM-driven browsing. But its JavaScript event simulation fails on React/Vue SPAs:

```
opencli:  JS dispatchEvent → React ignores it → silent failure
claw:     CDP Input.dispatchMouseEvent → browser-native → works everywhere
```

Claw keeps the YAML adapter ecosystem and fixes the execution layer with CDP-level precision.

## Architecture

```
┌─────────────────────────────────────┐
│  Layer 3: Adapters (YAML)           │  Hot-loadable, opencli-compatible
├─────────────────────────────────────┤
│  Layer 2: Pipeline Engine (Rust)    │  navigate/wait/evaluate/map/click/type
├─────────────────────────────────────┤
│  Layer 1: CDP Client (Rust)         │  Native browser control
└─────────────────────────────────────┘
```

## Writing Adapters

Adapters are YAML files in `./adapters/` or `~/.claw/adapters/`:

```yaml
site: example
name: search
description: Search example.com
browser: true
args:
  query:
    type: string
columns: [title, url]
pipeline:
  - navigate: https://example.com
  - wait: 2
  - type:
      selector: "input[name=q]"
      text: "${{ args.query }}"
  - click_selector: "button[type=submit]"
  - wait: 2
  - evaluate: |
      Array.from(document.querySelectorAll('.result')).map(el => ({
        title: el.querySelector('h3').textContent,
        url: el.querySelector('a').href
      }))
  - map:
      title: ${{ item.title }}
      url: ${{ item.url }}
```

### Pipeline Steps

| Step | Syntax | Description |
|------|--------|-------------|
| navigate | `navigate: <url>` | Load a URL |
| wait | `wait: <seconds>` | Pause execution |
| evaluate | `evaluate: <js>` | Run JS, capture result |
| map | `map: { key: ${{ item.field }} }` | Transform each item |
| limit | `limit: <n>` | Truncate results |
| click | `click: "Button Text"` | CDP native click by text |
| click_selector | `click_selector: "css"` | CDP native click by selector |
| type | `type: { selector, text }` | CDP native keyboard input |
| upload | `upload: { selector, files }` | CDP file upload |

## Commands

```bash
claw list                    # List available adapters
claw doctor                  # Diagnose Chrome connection
claw completions zsh         # Generate shell completions
claw evaluate "1+1"          # Run JS in browser
claw navigate <url>          # Navigate browser
```

## License

MIT
