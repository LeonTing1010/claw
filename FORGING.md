# Grab Spec

How to create claws. For AI agents, not humans.

## 1. Decision Tree

```
GOAL: Read data?
  Public API in network log?    -> read-api    (fetch, no browser)
  API needs cookies?            -> read-browser (navigate + evaluate w/ credentials)
  API uses signed/CSRF params?  -> intercept   (trigger + capture response)
  No API, data in DOM?          -> read-browser (navigate + evaluate DOM query)
GOAL: Write/interact?           -> write-ui    (Lua run: with page:* calls)
GOAL: Compose claws?            -> Lua run: with claw.run() calls
```

Recon: `network_log_start` -> `navigate` -> interact -> `network_log_dump_bodies`. API first, DOM fallback.

## 2. Claw Anatomy

```yaml
# Required
site: weibo              # Site identifier (directory name)
name: hot                # Command name (file stem)
columns: [rank, title]   # Output column names
# Plus one of: pipeline: (YAML steps) | run: (Lua script)

# Optional
description: "What this does"
domain: example.com                 # Cookie scoping
strategy: public|cookie|intercept   # Auth tier
browser: true                       # Needs Chrome
version: "1"                        # Bump on re-forge
last_forged: "2026-03-28"
forged_by: "claude-opus-4"
args:
  limit: { type: int, default: 20 }
```

## 3. Four Archetypes

### A. Read via API (no browser)

```yaml
site: example
name: feed
columns: [title, url]
args:
  limit: { type: int, default: 10 }
pipeline:
  - fetch: https://api.example.com/feed
  - select: data.items
  - map:
      title: ${{ item.title }}
      url: ${{ item.url }}
  - limit: ${{ args.limit }}
```

### B. Read via Browser (DOM or cookie-auth API)

```yaml
site: hackernews
name: hot
domain: news.ycombinator.com
strategy: public
browser: true
columns: [rank, title, hot]
pipeline:
  - navigate: https://news.ycombinator.com/
  - wait: 2
  - evaluate: |
      (() => {
        const rows = document.querySelectorAll('.athing');
        return [...rows].map((row, i) => ({
          rank: i + 1,
          title: row.querySelector('.titleline > a')?.textContent || '',
          hot: row.nextElementSibling?.querySelector('.score')?.textContent?.replace(/\D/g,'') || '0'
        }));
      })()
  - map:
      rank: ${{ item.rank }}
      title: ${{ item.title }}
      hot: ${{ item.hot }}
  - limit: ${{ args.limit }}
```

For cookie-auth, add `strategy: cookie` and use `fetch(url, {credentials:'include'})` inside evaluate.

### C. Network Intercept

```yaml
site: douyin
name: hot
domain: douyin.com
strategy: intercept
browser: true
columns: [rank, title, hot]
pipeline:
  - intercept:
      trigger: "navigate: https://www.douyin.com/hot"
      capture: "/aweme/v1/hot/search/list"
      timeout: 10
      select: data.word_list
  - map:
      rank: ${{ item.position }}
      title: ${{ item.word }}
      hot: ${{ item.hot_value }}
  - limit: ${{ args.limit }}
```

Use intercept when the API uses signed params, CSRF tokens, or encrypted headers that can't be replayed.

### D. Write via Lua

```yaml
site: xiaohongshu
name: publish
domain: creator.xiaohongshu.com
strategy: cookie
browser: true
columns: [status, detail]
args:
  title: { type: string, default: "" }
  images: { type: string }
run: |
  page:goto("https://creator.xiaohongshu.com/publish/publish")
  page:wait_for_selector(".creator-tab", 10)
  page:click_text("Upload")
  page:wait(2)
  page:upload("input.upload-input", args.images)
  page:wait(20)
  if args.title ~= "" then
    page:type_into("input.d-text", args.title)
  end
  page:click_text("Publish")
  page:wait(5)
  return { status = "submitted", detail = page:page_info().url or "" }
```

Lua API: `page:goto`, `page:wait`, `page:wait_for_selector`, `page:click_text`, `page:click_selector`, `page:type_into`, `page:upload`, `page:find`, `page:page_info`, `page:evaluate`, `page:screenshot`, `page:wait_for_url`. For composition: `claw.run(site, adapter, {args})`.

## 4. Pitfalls

| Problem | Cause | Fix |
|---------|-------|-----|
| Empty data | SPA not loaded when evaluate runs | Add `wait: 2-3` or `wait_for_selector` before evaluate |
| Click does nothing | React/Vue ignores JS dispatchEvent | Use `click:` or `click_selector:` (CDP native), never JS `.click()` |
| Stale selectors | Site redesign | Prefer stable selectors: `[data-testid]`, `.rank`, semantic class names. Avoid generated classes like `.css-1a2b3c` |
| Cookie auth fails | Domain mismatch | Set `domain:` to match cookie scope. `navigate` first to establish cookie context |
| Upload hangs | Wrong input selector | Target `input[type='file']` directly, even if hidden. `page:upload` uses CDP DOM.setFileInputFiles |
| Intercept misses | Timing: request fires before listener | `intercept` handles this -- it sets up capture before executing trigger |
| Template not resolved | Typo in `${{ }}` | Must be exactly `${{ args.name }}` or `${{ item.field }}` with spaces inside braces |
| Lua returns nothing | Missing `return` | Lua `run:` must end with `return {field=val}` or `return {{row1}, {row2}}` |
| evaluate returns undefined | Async without await | Wrap in `(async () => { ... })()` for any fetch/Promise code |
| Modal/popup blocks flow | Overlay appears on load | Use `if page:find(".modal")` (Lua) or `if_selector:` (YAML) to dismiss |

## 5. Quality Checklist

- [ ] `columns:` matches keys in `map:` or Lua return
- [ ] `wait:` or `wait_for_selector:` before any DOM read
- [ ] `limit:` step present and wired to `${{ args.limit }}`
- [ ] No hardcoded generated CSS classes (`.css-xxx`, `._abc123`)
- [ ] evaluate returns an array of objects, not raw HTML
- [ ] Lua scripts have explicit `return`
- [ ] `strategy:` matches actual auth requirement
- [ ] `browser: true` set when pipeline uses navigate/click/evaluate
- [ ] Version and last_forged metadata present
- [ ] Tested with `verify_adapter` -- returns rows, no step errors

## 6. Transform Helpers

Available in `transform:` steps (Lua). Data arrives as `data` (array of objects), `args` as global.

```yaml
# Sort by field
- transform: "return sort_by(data, 'views', 'desc')"    # or 'asc' (default)

# Limit results
- transform: "return limit(data, 10)"

# Keep only specific fields
- transform: "return pick(data, 'title', 'url')"

# Group into {key, items, count} objects
- transform: "return group_by(data, 'platform')"

# Deduplicate by field (keeps first occurrence)
- transform: "return unique_by(data, 'title')"

# Chain: sort then limit
- transform: "return limit(sort_by(data, 'hot', 'desc'), 20)"
```

Helpers: `sort_by(tbl, field, order)`, `limit(tbl, n)`, `pick(tbl, ...)`, `group_by(tbl, field)`, `unique_by(tbl, field)`.
