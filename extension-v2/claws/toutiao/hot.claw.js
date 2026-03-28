export default {
  site: "toutiao",
  name: "hot",
  description: "头条热榜",
  columns: ["rank", "title", "hot"],
  args: { limit: { type: "int", default: 20 } },
  health: { min_rows: 5, non_empty: ["title"] },

  async run(page, args) {
    await page.nav("https://www.toutiao.com")
    await page.wait(3000)

    const items = await page.eval(async () => {
      const res = await fetch('https://www.toutiao.com/hot-event/hot-board/?origin=toutiao_pc')
      const data = await res.json()
      return (data.data || []).map((item, i) => ({
        rank: String(i + 1),
        title: item.Title || '',
        hot: String(item.HotValue || 0)
      }))
    })

    return items.slice(0, args.limit)
  }
}
