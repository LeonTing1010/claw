export default {
  site: "lobsters",
  name: "hot",
  description: "Lobsters hot posts",
  columns: ["rank", "title", "score", "tags", "comments"],
  args: { limit: { type: "int", default: 20 } },
  health: { min_rows: 5, non_empty: ["title"] },

  async run(page, args) {
    await page.nav("https://lobste.rs")
    await page.wait(2000)

    const items = await page.eval(() => {
      return (async () => {
        const res = await fetch('https://lobste.rs/hottest.json')
        const data = await res.json()
        return data.map((t, i) => ({
          rank: String(i + 1),
          title: t.title,
          score: String(t.score),
          tags: t.tags.join(', '),
          comments: String(t.comment_count)
        }))
      })()
    })

    return items.slice(0, args.limit)
  }
}
