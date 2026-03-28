export default {
  site: "x",
  name: "trending",
  description: "X/Twitter Trending Topics",
  columns: ["rank", "title", "hot"],
  args: { limit: { type: "int", default: 20 } },
  health: { min_rows: 5, non_empty: ["title"] },

  async run(page, args) {
    await page.nav("https://x.com/explore/tabs/trending")
    await page.wait(4000)

    const items = await page.eval(() => {
      const items = []
      const trends = document.querySelectorAll('[data-testid="trend"], [data-testid="cellInnerDiv"]')
      trends.forEach((el, i) => {
        const spans = el.querySelectorAll('span')
        let title = ''
        let hot = ''
        spans.forEach(s => {
          const text = (s.textContent || '').trim()
          if (text.startsWith('#') || (text.length > 1 && text.length < 100 && !text.includes('Trending') && !text.includes('posts'))) {
            if (!title && text.length > 1) title = text
          }
          if (text.includes('posts') || text.includes('K') || text.includes('M')) {
            if (!hot) hot = text
          }
        })
        if (title) {
          items.push({ rank: String(i + 1), title: title, hot: hot || '0' })
        }
      })
      // Deduplicate by title
      const seen = new Set()
      return items.filter(x => {
        if (seen.has(x.title)) return false
        seen.add(x.title)
        return true
      })
    })

    return items.slice(0, args.limit)
  }
}
