export default {
  site: "xiaohongshu",
  name: "search_fast",
  description: "纯 HTTP 搜索小红书（无浏览器，从 SSR HTML 提取 __INITIAL_STATE__）",
  columns: ["title", "likes", "comments", "collects", "author", "note_id"],
  args: {
    keyword: { type: "string" },
    limit: { type: "int", default: 20 }
  },
  health: { min_rows: 3, non_empty: ["title"] },

  async run(page, args) {
    const url = `https://www.xiaohongshu.com/search_result?keyword=${encodeURIComponent(args.keyword)}&type=51`
    const html = await page.fetch(url, {
      method: 'GET',
      headers: {
        'user-agent': 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36',
        'accept': 'text/html,application/xhtml+xml'
      },
      raw: true
    })

    // Extract __INITIAL_STATE__ JSON from the SSR HTML
    const stateMatch = html.match(/__INITIAL_STATE__\s*=\s*({[\s\S]*?})\s*<\/script>/)
    if (!stateMatch) {
      return [{ title: 'ERROR: no __INITIAL_STATE__ found', likes: '0', comments: '0', collects: '0', author: '', note_id: '' }]
    }

    let raw = stateMatch[1]
    // Fix JS-specific values that aren't valid JSON
    raw = raw.replace(/:undefined/g, ':null').replace(/,undefined/g, ',null')

    let state
    try {
      state = JSON.parse(raw)
    } catch (e) {
      // Greedy extraction fallback: find matching braces manually
      const start = html.indexOf('__INITIAL_STATE__')
      if (start === -1) {
        return [{ title: 'ERROR: no __INITIAL_STATE__ found', likes: '0', comments: '0', collects: '0', author: '', note_id: '' }]
      }
      const jsonStart = html.indexOf('{', start)
      if (jsonStart === -1) {
        return [{ title: 'ERROR: no JSON start', likes: '0', comments: '0', collects: '0', author: '', note_id: '' }]
      }
      let depth = 0
      let jsonEnd = jsonStart
      for (let i = jsonStart; i < Math.min(jsonStart + 500000, html.length); i++) {
        if (html[i] === '{') depth++
        else if (html[i] === '}') {
          depth--
          if (depth === 0) { jsonEnd = i; break }
        }
      }
      const rawBlock = html.substring(jsonStart, jsonEnd + 1)
        .replace(/:undefined/g, ':null')
        .replace(/,undefined/g, ',null')
      try {
        state = JSON.parse(rawBlock)
      } catch (e2) {
        return [{ title: 'ERROR: JSON parse failed: ' + e2.message, likes: '0', comments: '0', collects: '0', author: '', note_id: '' }]
      }
    }

    const search = state && state.search
    if (!search) {
      return [{ title: 'ERROR: no search in state', likes: '0', comments: '0', collects: '0', author: '', note_id: '' }]
    }

    // feeds might be a Vue ref wrapper with _rawValue/_value, or direct array
    let feeds = search.feeds
    if (feeds && feeds._rawValue) feeds = feeds._rawValue
    if (feeds && feeds._value) feeds = feeds._value

    if (!Array.isArray(feeds)) {
      return [{ title: 'ERROR: feeds not array', likes: '0', comments: '0', collects: '0', author: '', note_id: '' }]
    }

    const results = feeds.map(item => {
      const nc = item.noteCard || item.note_card || {}
      const interact = nc.interactInfo || nc.interact_info || {}
      return {
        title: String(nc.displayTitle || nc.display_title || ''),
        likes: String(interact.likedCount || interact.liked_count || 0),
        comments: String(interact.commentCount || interact.comment_count || 0),
        collects: String(interact.collectedCount || interact.collected_count || 0),
        author: String((nc.user || {}).nickname || ''),
        note_id: String(item.id || '')
      }
    })

    return results.slice(0, args.limit)
  }
}
