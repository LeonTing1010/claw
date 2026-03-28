export default {
  site: "douyin",
  name: "hot",
  description: "抖音热搜榜",
  columns: ["rank", "title", "hot"],
  args: { limit: { type: "int", default: 20 } },
  health: { min_rows: 5, non_empty: ["title"] },

  async run(page, args) {
    await page.nav("https://www.douyin.com")
    await page.wait(3000)

    const items = await page.eval(async () => {
      const res = await fetch('https://www.douyin.com/aweme/v1/web/hot/search/list/', { credentials: 'include' })
      const data = await res.json()
      if (data.data && data.data.word_list) {
        return data.data.word_list.map((item, i) => ({
          rank: String(i + 1),
          title: item.word,
          hot: String(item.hot_value || 0)
        }))
      }
      return [{ rank: '1', title: 'API requires login — visit douyin.com first', hot: '0' }]
    })

    return items.slice(0, args.limit)
  }
}
