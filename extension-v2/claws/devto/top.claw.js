export default {
  site: "devto",
  name: "top",
  description: "Dev.to top articles",
  columns: ["title", "reactions", "comments", "author"],
  args: { limit: { type: "int", default: 20 } },
  health: { min_rows: 5, non_empty: ["title"] },

  async run(page, args) {
    const data = await page.fetch("https://dev.to/api/articles?per_page=50&state=rising")
    return data.map(item => ({
      title: item.title,
      reactions: String(item.positive_reactions_count),
      comments: String(item.comments_count),
      author: item.user.name
    })).slice(0, args.limit)
  }
}
