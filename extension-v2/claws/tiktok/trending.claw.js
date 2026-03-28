export default {
  site: "tiktok",
  name: "trending",
  description: "TikTok Trending Videos",
  columns: ["rank", "author", "views", "url"],
  args: { limit: { type: "int", default: 20 } },
  health: { min_rows: 3, non_empty: ["author"] },

  async run(page, args) {
    await page.nav("https://www.tiktok.com/explore")
    await page.wait(5000)

    const items = await page.eval(() => {
      const links = document.querySelectorAll('a[href*="/video/"]')
      const items = []
      const seen = new Set()
      for (const a of links) {
        const href = a.href
        if (seen.has(href)) continue
        seen.add(href)
        const match = href.match(/@([^/]+)/)
        const author = match ? match[1] : ''
        const views = a.textContent.trim()
        items.push({
          rank: String(items.length + 1),
          author: author,
          views: views || '0',
          url: href
        })
      }
      return items
    })

    return items.slice(0, args.limit)
  }
}
