export default {
  site: "douban",
  name: "hot",
  description: "豆瓣热门电影",
  columns: ["rank", "title", "rate"],
  args: { limit: { type: "int", default: 20 } },
  health: { min_rows: 5, non_empty: ["title"] },

  async run(page, args) {
    await page.nav("https://movie.douban.com")
    await page.wait(2000)

    const items = await page.eval(async () => {
      const res = await fetch('https://movie.douban.com/j/search_subjects?type=movie&tag=%E7%83%AD%E9%97%A8&page_limit=50&page_start=0')
      const data = await res.json()
      return data.subjects.map((m, i) => ({
        rank: String(i + 1),
        title: m.title,
        rate: m.rate || '-'
      }))
    })

    return items.slice(0, args.limit)
  }
}
