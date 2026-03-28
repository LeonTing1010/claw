export default {
  site: "weibo",
  name: "hot",
  description: "微博热搜榜",
  columns: ["rank", "title", "hot"],
  args: { limit: { type: "int", default: 20 } },
  health: { min_rows: 10, non_empty: ["title"] },

  async run(page, args) {
    await page.nav("https://weibo.com")
    await page.wait(3000)

    const items = await page.eval(async () => {
      const res = await fetch('https://weibo.com/ajax/side/hotSearch')
      const data = await res.json()
      return data.data.realtime.map((item, i) => ({
        rank: i + 1,
        title: item.note,
        hot: item.num || 0
      }))
    })

    return items.slice(0, args.limit)
  }
}
