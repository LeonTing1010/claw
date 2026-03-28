export default {
  site: "sspai",
  name: "hot",
  description: "少数派热门文章",
  columns: ["title", "likes", "author"],
  args: { limit: { type: "int", default: 20 } },
  health: { min_rows: 5, non_empty: ["title"] },

  async run(page, args) {
    await page.nav("https://sspai.com")
    await page.wait(2000)

    const items = await page.eval(async () => {
      const res = await fetch('https://sspai.com/api/v1/articles?offset=0&limit=50&sort=hottest_daily')
      const data = await res.json()
      return (data.list || data.data || []).map(a => ({
        title: String(a.title || ''),
        likes: String(a.like_count || a.likes || 0),
        author: String(a.author?.nickname || a.nickname || '-')
      }))
    })

    return items.slice(0, args.limit)
  }
}
