export default {
  site: "xueqiu",
  name: "hot-stock",
  description: "雪球热门股票",
  columns: ["rank", "title", "code", "percent"],
  args: { limit: { type: "int", default: 20 } },
  health: { min_rows: 5, non_empty: ["title", "code"] },

  async run(page, args) {
    await page.nav("https://xueqiu.com")
    await page.wait(2000)

    const items = await page.eval(async () => {
      const res = await fetch('https://stock.xueqiu.com/v5/stock/hot_stock/list.json?size=50&_type=10&type=10', { credentials: 'include' })
      const data = await res.json()
      return (data.data.items || []).map((s, i) => ({
        rank: String(i + 1),
        title: String(s.name || ''),
        code: String(s.code || ''),
        percent: (s.percent >= 0 ? '+' : '') + s.percent.toFixed(2) + '%'
      }))
    })

    return items.slice(0, args.limit)
  }
}
