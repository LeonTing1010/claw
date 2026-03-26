# Claw — Technical Design

## Problem Statement

AI Agents need to operate internet services reliably. Current approaches:

| Approach | Cost | Reliability | Speed |
|----------|------|-------------|-------|
| LLM-driven browsing (Browser-Use, Stagehand) | High (tokens per action) | Low (non-deterministic) | Slow (10-60s) |
| Pre-built adapters (opencli) | Zero | High for reads, **low for writes** | Fast (1-10s) |
| Direct API integration | Zero | High | Fastest |

opencli solved the read problem brilliantly. But write operations (publish, submit, interact) fail on modern React/Vue SPAs because JavaScript `dispatchEvent()` doesn't trigger framework event handlers.

**Claw's thesis: the adapter approach is correct, the execution layer needs native precision.**

---

## Architecture

### Three-Layer Design

```
┌──────────────────────────────────────────────────────┐
│  Layer 3: Adapters                                    │
│  YAML (opencli-compatible) + Lua (complex logic)     │
│  Hot-loadable, user-extensible                        │
├──────────────────────────────────────────────────────┤
│  Layer 2: Pipeline Engine                             │
│  Step execution, template rendering, output formatting│
├──────────────────────────────────────────────────────┤
│  Layer 1: CDP Client                                  │
│  WebSocket → Chrome DevTools Protocol                 │
│  Native mouse/keyboard/network control                │
└──────────────────────────────────────────────────────┘
```

### Layer 1: CDP Client

The core differentiator. All browser interactions go through CDP protocol, not JavaScript injection.

#### Native Mouse Events

```rust
impl CdpClient {
    /// Click at exact coordinates — works with React, Vue, Angular, any framework
    async fn click(&self, x: f64, y: f64) -> Result<()> {
        self.send("Input.dispatchMouseEvent", json!({
            "type": "mousePressed",
            "x": x, "y": y,
            "button": "left",
            "clickCount": 1
        })).await?;
        self.send("Input.dispatchMouseEvent", json!({
            "type": "mouseReleased",
            "x": x, "y": y,
            "button": "left",
            "clickCount": 1
        })).await?;
        Ok(())
    }

    /// Click element matching selector — resolve coordinates, then CDP click
    async fn click_selector(&self, selector: &str) -> Result<()> {
        let rect = self.evaluate(&format!(
            "JSON.stringify(document.querySelector('{}').getBoundingClientRect())",
            selector
        )).await?;
        let r: Rect = serde_json::from_str(&rect)?;
        self.click(r.x + r.width / 2.0, r.y + r.height / 2.0).await
    }

    /// Click element containing specific text
    async fn click_text(&self, text: &str) -> Result<()> {
        let js = format!(r#"
            (() => {{
                const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_ELEMENT);
                while (walker.nextNode()) {{
                    const el = walker.currentNode;
                    if (el.children.length === 0
                        && (el.textContent || '').trim() === '{}'
                        && el.offsetParent !== null) {{
                        const r = el.getBoundingClientRect();
                        return JSON.stringify({{ x: r.x + r.width/2, y: r.y + r.height/2 }});
                    }}
                }}
                return null;
            }})()
        "#, text);
        let coords = self.evaluate(&js).await?;
        let p: Point = serde_json::from_str(&coords)?;
        self.click(p.x, p.y).await
    }
}
```

#### Native Keyboard Input

```rust
impl CdpClient {
    /// Type text character by character — triggers all framework input handlers
    async fn type_text(&self, text: &str) -> Result<()> {
        for ch in text.chars() {
            self.send("Input.dispatchKeyEvent", json!({
                "type": "keyDown",
                "text": ch.to_string(),
            })).await?;
            self.send("Input.dispatchKeyEvent", json!({
                "type": "keyUp",
                "text": ch.to_string(),
            })).await?;
        }
        Ok(())
    }

    /// Type into a specific element — focus first, then type
    async fn type_into(&self, selector: &str, text: &str) -> Result<()> {
        self.click_selector(selector).await?;
        tokio::time::sleep(Duration::from_millis(200)).await;
        // Select all + delete to clear existing content
        self.send("Input.dispatchKeyEvent", json!({
            "type": "keyDown", "key": "a",
            "modifiers": 2 // Ctrl/Cmd
        })).await?;
        self.send("Input.dispatchKeyEvent", json!({
            "type": "keyUp", "key": "a"
        })).await?;
        self.send("Input.dispatchKeyEvent", json!({
            "type": "keyDown", "key": "Backspace"
        })).await?;
        self.send("Input.dispatchKeyEvent", json!({
            "type": "keyUp", "key": "Backspace"
        })).await?;
        tokio::time::sleep(Duration::from_millis(100)).await;
        self.type_text(text).await
    }
}
```

#### File Upload

```rust
impl CdpClient {
    /// Upload files via CDP — no DataTransfer injection needed
    async fn upload_files(&self, selector: &str, paths: &[&str]) -> Result<()> {
        // Get the DOM node ID for the file input
        let doc = self.send("DOM.getDocument", json!({})).await?;
        let node = self.send("DOM.querySelector", json!({
            "nodeId": doc["root"]["nodeId"],
            "selector": selector
        })).await?;

        self.send("DOM.setFileInputFiles", json!({
            "nodeId": node["nodeId"],
            "files": paths
        })).await?;
        Ok(())
    }
}
```

#### Network Interception

```rust
impl CdpClient {
    /// Intercept network requests matching a pattern
    async fn intercept(&self, pattern: &str) -> Result<NetworkCapture> {
        self.send("Fetch.enable", json!({
            "patterns": [{ "urlPattern": pattern }]
        })).await?;
        // ... capture and return matching requests
    }
}
```

### Layer 2: Pipeline Engine

Executes adapter pipeline steps sequentially. Template variables resolved via `${{ args.name }}` syntax (opencli-compatible).

#### Step Types

```rust
enum PipelineStep {
    // opencli-compatible steps
    Navigate(String),           // navigate: <url>
    Wait(f64),                  // wait: <seconds>
    Evaluate(String),           // evaluate: <js_code> — runs in browser via CDP
    Map(HashMap<String, Tmpl>), // map: { key: ${{ item.field }} }
    Limit(Tmpl),                // limit: ${{ args.limit }}
    Tap {                       // tap: { store, action, capture }
        store: String,
        action: String,
        capture: String,
    },
    Intercept(String),          // intercept: <url_pattern>

    // claw-native steps (not in opencli)
    Click(String),              // click: "上传图文" — CDP native click by text
    ClickSelector(String),      // click_selector: "button.submit" — CDP click by CSS
    Type {                      // type: { selector, text }
        selector: String,
        text: Tmpl,
    },
    Upload {                    // upload: { selector, files }
        selector: String,
        files: Tmpl,
    },
    Screenshot(String),         // screenshot: /tmp/debug.png
}
```

#### Template Engine

Minimal template engine compatible with opencli's `${{ }}` syntax:

```rust
fn render_template(tmpl: &str, context: &Context) -> String {
    // ${{ args.prompt }}       → argument value
    // ${{ args.prompt | json }} → JSON-escaped
    // ${{ item.field }}        → current pipeline item field
    regex.replace_all(tmpl, |caps| {
        let expr = caps[1].trim();
        context.resolve(expr)
    })
}
```

#### Output Formatting

Same formats as opencli: `table` (default), `json`, `yaml`, `csv`, `md`.

### Layer 3: Adapters

Two adapter types, both hot-loadable from `~/.claw/adapters/` or `./adapters/`:

#### YAML Adapters (opencli-compatible)

```yaml
# 100% compatible with opencli YAML format
site: bilibili
name: hot
description: B站热门视频
domain: bilibili.com
strategy: cookie
browser: true

args:
  limit:
    type: int
    default: 10

columns: [title, author, views, url]

pipeline:
  - navigate: https://bilibili.com
  - evaluate: |
      (async () => {
        const res = await fetch('/x/web-interface/ranking/v2', { credentials: 'include' });
        const data = await res.json();
        return data.data.list.map(v => ({
          title: v.title, author: v.owner.name,
          views: v.stat.view, url: 'https://bilibili.com/video/' + v.bvid
        }));
      })()
  - map:
      title: ${{ item.title }}
      author: ${{ item.author }}
      views: ${{ item.views }}
      url: ${{ item.url }}
  - limit: ${{ args.limit }}
```

#### Lua Adapters (for complex logic)

When YAML + evaluate JS isn't enough (multi-step UI flows, conditional logic, error recovery):

```lua
-- adapters/xiaohongshu/publish.lua
return {
  site = "xiaohongshu",
  name = "publish",
  description = "小红书发布图文笔记",
  strategy = "cookie",

  args = {
    { name = "content", positional = true, required = true },
    { name = "title",   required = true, help = "笔记标题 (max 20 chars)" },
    { name = "images",  help = "图片路径，逗号分隔" },
    { name = "topics",  help = "话题标签，逗号分隔" },
    { name = "draft",   type = "bool", default = false },
  },

  columns = { "status", "detail" },

  run = function(page, args)
    -- Navigate to publish page
    page:goto("https://creator.xiaohongshu.com/publish/publish?from=menu_left")
    page:wait(3)

    -- CDP native click — React-compatible
    page:click_text("上传图文")
    page:wait(2)

    -- Upload images via CDP DOM.setFileInputFiles
    if args.images then
      local paths = split(args.images, ",")
      page:upload("input[type='file']", paths)
      page:wait_for_not("[class*='uploading']", 30)
    end

    -- Fill title — CDP native type
    page:type_into("input[maxlength='20']", args.title)
    page:wait(0.5)

    -- Fill content
    page:type_into("[contenteditable='true']", args.content)
    page:wait(0.5)

    -- Add topics
    if args.topics then
      for _, topic in ipairs(split(args.topics, ",")) do
        page:click_text("添加话题")
        page:wait(1)
        page:type_into("input[placeholder*='搜索话题']", topic)
        page:wait(1.5)
        page:click_selector("[class*='topic-item']:first-child")
        page:wait(0.5)
      end
    end

    -- Publish or save draft
    local btn_text = args.draft and "暂存" or "发布"
    page:click_text(btn_text)
    page:wait(4)

    local url = page:evaluate("() => location.href")
    local success = not url:find("/publish/publish")

    return {{
      status = success and "published" or "check-browser",
      detail = string.format('"%s" · %s', args.title, success and "OK" or url)
    }}
  end
}
```

**Why Lua over TypeScript:**

| | TypeScript (opencli) | Lua (claw) |
|---|---|---|
| Embedding in Rust | Impossible without V8/Deno | Native via `mlua` crate, ~2MB |
| Sandbox | Full page access in CDP | Controlled API surface |
| Syntax | Complex (async/await, closures) | Minimal, readable |
| Generation by AI | Harder to get right | Simpler grammar, fewer errors |
| Hot-reload | Requires rebuild | Instant |

---

## Auth Strategy (5-Tier, opencli-compatible)

```
Tier 1: public   — Direct HTTP, no browser needed
Tier 2: cookie   — fetch() with credentials:'include'
Tier 3: header   — Cookie + CSRF/Bearer token
Tier 4: intercept — Pinia/Vuex Store Action + XHR capture
Tier 5: ui       — Full UI automation (CDP native in claw)
```

Claw's advantage is in Tier 5: where opencli falls back to JS DOM manipulation and frequently fails, claw uses CDP native events that work identically to real user input.

---

## Browser Connection

### Option A: Reuse opencli's Browser Bridge extension

The extension exposes a WebSocket endpoint. Claw connects to the same endpoint — zero migration for existing opencli users.

### Option B: Direct CDP connection

Chrome's `--remote-debugging-port` flag exposes CDP directly. No extension needed, but requires Chrome launch with the flag.

### Recommended: Support both

```rust
enum BrowserConnection {
    BridgeExtension { ws_url: String },  // Reuse opencli extension
    DirectCdp { port: u16 },             // Chrome --remote-debugging-port
}
```

---

## Compatibility with opencli

### What works out of the box

- All YAML adapters (~200+) — parsed and executed identically
- `cli-manifest.json` — read directly for pre-built adapters
- Output formats — table/json/yaml/csv/md
- Template syntax — `${{ args.x }}`, `${{ item.y }}`, `${{ args.x | json }}`
- Auth strategies — 5-tier system

### What's different

- `.ts` adapters → need migration to `.lua` (incremental, per-adapter)
- Browser click/type → CDP native instead of JS injection
- New pipeline steps: `click`, `click_selector`, `type`, `upload`, `screenshot`
- No Node.js dependency

---

## Distribution

```
opencli:  npm install -g @jackwener/opencli
          → requires Node.js 20+ (~800MB runtime)
          → ~500ms cold start

claw:     single binary (~15MB)
          → zero dependencies
          → ~10ms cold start
          → cross-platform: macOS (arm64/x64), Linux, Windows
```

---

## Implementation Roadmap

### Phase 1: Core (Week 1)

- [ ] CDP WebSocket client (connect, evaluate, navigate)
- [ ] CDP native mouse/keyboard events
- [ ] YAML adapter parser (serde)
- [ ] Pipeline engine (6 step types: navigate/wait/evaluate/map/limit/intercept)
- [ ] Template engine (`${{ }}` syntax)
- [ ] Output formatting (table/json)
- [ ] CLI framework (clap)
- [ ] Browser Bridge extension compatibility

**Milestone: `claw bilibili hot --limit 5` works**

### Phase 2: Lua + High-Value Adapters (Week 2)

- [ ] Lua runtime integration (mlua)
- [ ] Page API bindings for Lua (goto/wait/click_text/type_into/upload/evaluate)
- [ ] Lua adapter: xiaohongshu/publish
- [ ] Lua adapter: jimeng/generate + history
- [ ] CDP file upload (DOM.setFileInputFiles)
- [ ] click / click_selector / type pipeline steps

**Milestone: `claw xiaohongshu publish` works end-to-end**

### Phase 3: Discovery + Ecosystem (Week 3)

- [ ] `claw explore <url>` — API discovery
- [ ] `claw cascade <url>` — auth strategy detection
- [ ] `claw record <url>` — request recording
- [ ] Adapter hot-loading from `~/.claw/adapters/`
- [ ] `claw install <name>` — install community adapters

**Milestone: users can discover and create adapters**

### Phase 4: Polish + Release (Week 4)

- [ ] Cross-platform builds (macOS arm64/x64, Linux x64)
- [ ] `claw doctor` — connectivity diagnosis
- [ ] Shell completions (bash/zsh/fish)
- [ ] Documentation site
- [ ] Publish to crates.io + GitHub releases

**Milestone: public release**

---

## Crate Dependencies

```toml
[dependencies]
tokio = { version = "1", features = ["full"] }          # async runtime
tokio-tungstenite = "0.24"                               # WebSocket (CDP)
serde = { version = "1", features = ["derive"] }         # serialization
serde_json = "1"                                         # JSON
serde_yaml = "0.9"                                       # YAML adapter parsing
clap = { version = "4", features = ["derive"] }          # CLI framework
mlua = { version = "0.10", features = ["lua54","async"] }# Lua scripting
tabled = "0.17"                                          # table output
reqwest = { version = "0.12", features = ["json"] }      # HTTP (public APIs)
regex = "1"                                              # template engine
```

---

## References

- [opencli](https://github.com/jackwener/opencli) — The project that proved the concept
- [Chrome DevTools Protocol](https://chromedevtools.github.io/devtools-protocol/) — CDP specification
- [CDP Input domain](https://chromedevtools.github.io/devtools-protocol/tot/Input/) — Mouse/keyboard events
- [CDP DOM domain](https://chromedevtools.github.io/devtools-protocol/tot/DOM/) — File upload, DOM queries
- [mlua](https://github.com/mlua-rs/mlua) — Lua bindings for Rust
