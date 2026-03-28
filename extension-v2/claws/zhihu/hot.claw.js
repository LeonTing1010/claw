export default {
  site: "zhihu",
  name: "hot",
  description: "知乎热榜",
  columns: ["rank", "title", "heat"],
  args: { limit: { type: "int", default: 20 } },
  health: { min_rows: 5, non_empty: ["title"] },

  async run(page, args) {
    await page.nav("https://www.zhihu.com")
    await page.wait(3000)

    const items = await page.eval(async () => {
      const res = await fetch('https://www.zhihu.com/api/v3/feed/topstory/hot-lists/total?limit=50')
      const data = await res.json()
      return data.data.map((item, i) => ({
        rank: String(i + 1),
        title: item.target.title,
        heat: item.detail_text || ''
      }))
    })

    return items.slice(0, args.limit)
  }
}
