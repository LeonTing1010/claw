export default {
  site: "google",
  name: "trends",
  description: "Google Trends trending searches",
  columns: ["rank", "title", "hot"],
  args: {
    limit: { type: "int", default: 20 },
    geo: { type: "string", default: "US" }
  },
  health: { min_rows: 5, non_empty: ["title"] },

  async run(page, args) {
    await page.nav("https://trends.google.com/trending?geo=" + args.geo)
    await page.wait(4000)

    const items = await page.eval(() => {
      const items = []
      const rows = document.querySelectorAll('[jsname] .mZ3RIc, tr[jsaction], .feed-item, [data-hveid]')
      if (rows.length > 0) {
        rows.forEach((row, i) => {
          const titleEl = row.querySelector('.mZ3RIc, .title, a') || row
          const title = (titleEl.textContent || '').trim().split('\n')[0].trim()
          const hotEl = row.querySelector('.lqv0Cb, .search-count-title, [aria-label*="search"]')
          const hot = hotEl ? hotEl.textContent.replace(/[^0-9KMB+.]/g, '') : ''
          if (title && title.length > 1 && title.length < 200) {
            items.push({ rank: String(i + 1), title: title, hot: hot || '0' })
          }
        })
      }
      // Fallback: extract from any visible trending list
      if (items.length === 0) {
        const allLinks = document.querySelectorAll('a[href*="/trending"]')
        allLinks.forEach((a, i) => {
          const text = (a.textContent || '').trim()
          if (text && text.length > 1 && text.length < 200) {
            items.push({ rank: String(i + 1), title: text, hot: '0' })
          }
        })
      }
      return items
    })

    return items.slice(0, args.limit)
  }
}
