export default {
  site: "xiaohongshu",
  name: "hot",
  description: "小红书热搜话题",
  columns: ["rank", "title", "hot"],
  args: { limit: { type: "int", default: 20 } },
  health: { min_rows: 5, non_empty: ["title"] },

  async run(page, args) {
    await page.nav("https://www.xiaohongshu.com/explore")
    await page.wait(4000)

    // CDP native click to reveal trending searches
    await page.click("input[placeholder*='搜索']")
    await page.wait(2000)

    const items = await page.eval(() => {
      const items = []

      // Trending/hot items from search suggestions
      const suggestions = document.querySelectorAll('.trending-item, .hot-item, [class*="hot"] a, [class*="trend"] a, .search-trending-item')
      suggestions.forEach((el, i) => {
        const text = (el.textContent || '').trim()
        if (text && text.length > 1) {
          items.push({ rank: i + 1, title: text, hot: '0' })
        }
      })

      // Fallback — explore feed note titles
      if (items.length === 0) {
        document.querySelectorAll('.note-item a, [class*="note"] .title, a.title, span.title').forEach((el, i) => {
          const text = (el.textContent || '').trim()
          if (text && text.length > 2 && text.length < 200) {
            items.push({ rank: i + 1, title: text, hot: '0' })
          }
        })
      }

      return items
    })

    return items.slice(0, args.limit)
  }
}
