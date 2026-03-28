export default {
  site: "wikipedia",
  name: "most-read",
  description: "Wikipedia most read articles today",
  columns: ["title", "description", "views"],
  args: { limit: { type: "int", default: 20 } },
  health: { min_rows: 5, non_empty: ["title"] },

  async run(page, args) {
    await page.nav("https://en.wikipedia.org")
    await page.wait(1000)

    const items = await page.eval(async () => {
      const d = new Date()
      const y = d.getFullYear()
      const m = d.getMonth() + 1
      const day = d.getDate() - 1
      const url = `https://en.wikipedia.org/api/rest_v1/feed/featured/${y}/${String(m).padStart(2, '0')}/${String(day).padStart(2, '0')}`
      const res = await fetch(url)
      const data = await res.json()
      return (data.mostread?.articles || []).map(a => ({
        title: String(a.titles?.normalized || a.title),
        description: String(a.description || '-'),
        views: String(a.views || 0)
      }))
    })

    return items.slice(0, args.limit)
  }
}
