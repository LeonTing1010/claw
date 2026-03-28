export default {
  site: "reddit",
  name: "hot",
  description: "Reddit Hot Posts",
  columns: ["rank", "title", "subreddit", "score"],
  args: { limit: { type: "int", default: 20 } },
  health: { min_rows: 5, non_empty: ["title"] },

  async run(page, args) {
    await page.nav("https://www.reddit.com/")
    await page.wait(2000)

    const items = await page.eval(async () => {
      const res = await fetch('https://www.reddit.com/r/popular/hot.json?limit=50&raw_json=1')
      const data = await res.json()
      return data.data.children.map((child, i) => ({
        rank: String(i + 1),
        title: child.data.title,
        subreddit: child.data.subreddit_name_prefixed || child.data.subreddit,
        score: String(child.data.score || 0)
      }))
    })

    return items.slice(0, args.limit)
  }
}
