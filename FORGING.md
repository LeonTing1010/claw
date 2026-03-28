# Forging Spec (v2)

How to create .claw.js files. For AI agents, not humans.

## 1. Forge Workflow

```
page_intelligence(url)     → framework, SSR state, APIs, strategies + templates
                           ↓
pick strategy template     → SSR > API > DOM (in priority order)
                           ↓
forge_verify(url, expr)    → test extraction logic, see sample data
                           ↓
iterate if needed          → tweak expression, re-verify
                           ↓
forge_save(site, name, code) → persist to ~/.claw/claws/
```

3-4 MCP calls total. `page_intelligence` returns ready-to-use templates — pick one, fill TODOs, verify, save.

## 2. .claw.js Format

```js
export default {
  // Required
  site: "weibo",                       // Site identifier
  name: "hot",                         // Claw name
  description: "微博热搜榜",            // What it does
  columns: ["rank", "title", "hot"],   // Output column names

  // Optional
  args: {
    limit: { type: "int", default: 20 },
    keyword: { type: "string" }        // No default = required arg
  },
  health: {
    min_rows: 5,                       // Minimum expected rows
    non_empty: ["title"]               // Columns that must have values
  },

  // Required: the extraction function
  async run(page, args) {
    // ... use page API to extract data ...
    return [{ rank: "1", title: "...", hot: "999" }]
  }
}
```

## 3. Page API — 10 Methods

| Method | Mode | Description |
|--------|------|-------------|
| `page.nav(url)` | scripting | Navigate to URL |
| `page.wait(ms)` | scripting | Fixed delay |
| `page.waitFor(selector, timeoutMs)` | scripting | Wait for CSS selector to appear |
| `page.eval(fn, ...args)` | scripting | Execute function in page context |
| `page.fetch(url, opts)` | scripting | Fetch with page cookies |
| `page.screenshot()` | scripting | Capture visible area |
| `page.cookies()` | scripting | Read cookies |
| `page.click(target)` | debugger | Click by selector or visible text |
| `page.type(selector, text)` | debugger | Type into input |
| `page.upload(selector, files)` | debugger | Upload files |

Scripting mode = undetectable. Debugger mode = ms-level attach/detach.

## 4. Strategy Decision Tree

```
page_intelligence returned strategies?
  Has SSR state (__NEXT_DATA__, __pinia, etc.)?
    → SSR strategy: page.eval(() => window.__STATE__) — zero requests
  Has API endpoints in api_hints?
    → API strategy: page.fetch(apiUrl) — one request, reliable
  Neither?
    → DOM strategy: page.eval(() => querySelectorAll(...)) — always works
```

## 5. Three Archetypes

### A. SSR State Extraction (fastest, most reliable)

```js
export default {
  site: "xiaohongshu",
  name: "hot",
  description: "小红书热门",
  columns: ["title", "likes", "url"],
  args: { limit: { type: "int", default: 20 } },

  async run(page, args) {
    await page.nav("https://www.xiaohongshu.com/explore")
    await page.wait(2000)

    const items = await page.eval(() => {
      // SSR state — look for __INITIAL_SSR_STATE__, __pinia, etc.
      const state = window.__INITIAL_SSR_STATE__
      return state.feed.items.map(item => ({
        title: item.title,
        likes: String(item.likes_count),
        url: 'https://www.xiaohongshu.com/explore/' + item.id
      }))
    })

    return items.slice(0, args.limit)
  }
}
```

### B. API Replay (reliable, structured)

```js
export default {
  site: "hackernews",
  name: "hot",
  description: "Hacker News top stories",
  columns: ["rank", "title", "score", "url"],
  args: { limit: { type: "int", default: 20 } },
  health: { min_rows: 5, non_empty: ["title"] },

  async run(page, args) {
    // API doesn't need cookies — fetch directly
    const ids = await page.fetch("https://hacker-news.firebaseio.com/v0/topstories.json")
    const items = []

    for (const id of ids.slice(0, args.limit)) {
      const item = await page.fetch(`https://hacker-news.firebaseio.com/v0/item/${id}.json`)
      items.push({
        rank: String(items.length + 1),
        title: item.title,
        score: String(item.score),
        url: item.url || `https://news.ycombinator.com/item?id=${id}`
      })
    }

    return items
  }
}
```

For cookie-auth APIs, `page.nav(domain)` first to establish session, then `page.fetch(apiUrl)`.

### C. DOM Extraction (fallback)

```js
export default {
  site: "hackernews",
  name: "hot",
  description: "HN top stories",
  columns: ["rank", "title", "hot"],
  args: { limit: { type: "int", default: 20 } },
  health: { min_rows: 5, non_empty: ["title"] },

  async run(page, args) {
    await page.nav("https://news.ycombinator.com/")
    await page.waitFor(".athing", 10000)
    await page.wait(2000)

    const items = await page.eval(() => {
      return Array.from(document.querySelectorAll('.athing')).map(row => {
        const title = row.querySelector('.titleline > a')?.textContent || ''
        const score = row.nextElementSibling?.querySelector('.score')?.textContent?.replace(/\D/g, '') || '0'
        const rank = row.querySelector('.rank')?.textContent?.replace('.', '').trim() || ''
        return { rank, title, hot: score }
      }).filter(item => item.title.length > 0)
    })

    return items.slice(0, args.limit)
  }
}
```

### D. Interactive (write/action)

```js
export default {
  site: "xiaohongshu",
  name: "publish",
  description: "发布小红书笔记",
  columns: ["status", "url"],
  args: {
    title: { type: "string" },
    images: { type: "string" }  // comma-separated paths
  },

  async run(page, args) {
    await page.nav("https://creator.xiaohongshu.com/publish/publish")
    await page.waitFor(".creator-tab", 10000)

    await page.upload("input[type='file']", args.images)
    await page.wait(5000)

    if (args.title) {
      await page.type("input.d-text", args.title)
    }

    await page.click("发布")
    await page.wait(3000)

    const url = await page.eval(() => location.href)
    return [{ status: "submitted", url }]
  }
}
```

## 6. Pitfalls

| Problem | Fix |
|---------|-----|
| Empty data | Add `page.waitFor(selector)` or `page.wait(2000)` before eval |
| Click does nothing | Use `page.click()` (CDP native), never JS `.click()` in eval |
| Stale selectors | Prefer `[data-testid]`, semantic classes. Avoid `.css-1a2b3c` |
| Cookie auth fails | `page.nav(domain)` first, then `page.fetch(api)` |
| evaluate returns undefined | Wrap async code: `page.eval(async () => { ... })` |
| fetch returns HTML | API URL wrong, or needs specific headers/cookies |

## 7. Quality Checklist

- [ ] `columns` matches keys in returned objects
- [ ] `waitFor` or `wait` before DOM reads
- [ ] `args.limit` used to cap results
- [ ] No hardcoded generated CSS classes
- [ ] `eval` returns array of objects, not raw HTML
- [ ] `health` contract defined (min_rows, non_empty)
- [ ] Tested with `forge_verify` — returns rows, correct columns
