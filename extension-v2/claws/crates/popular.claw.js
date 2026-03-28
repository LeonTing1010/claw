export default {
  site: "crates",
  name: "popular",
  description: "crates.io popular Rust packages",
  columns: ["name", "downloads", "description"],
  args: { limit: { type: "int", default: 20 } },
  health: { min_rows: 5, non_empty: ["name"] },

  async run(page, args) {
    await page.nav("https://crates.io")
    await page.wait(2000)

    const items = await page.eval(() => {
      return (async () => {
        const res = await fetch('/api/v1/crates?page=1&per_page=50&sort=downloads')
        const data = await res.json()
        return data.crates.map(c => ({
          name: c.name,
          downloads: String(c.downloads),
          description: (c.description || '').substring(0, 60)
        }))
      })()
    })

    return items.slice(0, args.limit)
  }
}
