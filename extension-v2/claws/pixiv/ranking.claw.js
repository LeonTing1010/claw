export default {
  site: "pixiv",
  name: "ranking",
  description: "Pixiv daily illustration ranking",
  columns: ["rank", "title", "author", "views"],
  args: { limit: { type: "int", default: 20 } },
  health: { min_rows: 5, non_empty: ["title"] },

  async run(page, args) {
    await page.nav("https://www.pixiv.net")
    await page.wait(1000)

    const data = await page.fetch("https://www.pixiv.net/ranking.php?mode=daily&content=all&p=1&format=json")
    const contents = data.contents || []
    return contents.slice(0, args.limit).map(item => ({
      rank: String(item.rank),
      title: String(item.title),
      author: String(item.user_name),
      views: String(item.view_count)
    }))
  }
}
