export default {
  site: "v2ex",
  name: "hot",
  description: "V2EX 热门话题",
  columns: ["title", "node", "replies"],
  args: { limit: { type: "int", default: 20 } },
  health: { min_rows: 5, non_empty: ["title"] },

  async run(page, args) {
    const data = await page.fetch("https://www.v2ex.com/api/topics/hot.json")

    return data.slice(0, args.limit).map(item => ({
      title: String(item.title || ''),
      node: String((item.node && item.node.title) || ''),
      replies: String(item.replies || 0)
    }))
  }
}
