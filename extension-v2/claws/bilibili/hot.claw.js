export default {
  site: "bilibili",
  name: "hot",
  description: "B站热门视频",
  columns: ["title", "author", "views", "url"],
  args: { limit: { type: "int", default: 10 } },
  health: { min_rows: 5, non_empty: ["title"] },

  async run(page, args) {
    await page.nav("https://www.bilibili.com")
    await page.wait(2000)

    const items = await page.eval(async () => {
      const res = await fetch('https://api.bilibili.com/x/web-interface/ranking/v2', { credentials: 'include' })
      const data = await res.json()
      return data.data.list.map(v => ({
        title: v.title,
        author: v.owner.name,
        views: String(v.stat.view),
        url: 'https://bilibili.com/video/' + v.bvid
      }))
    })

    return items.slice(0, args.limit)
  }
}
