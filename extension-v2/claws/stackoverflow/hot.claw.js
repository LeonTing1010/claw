export default {
  site: "stackoverflow",
  name: "hot",
  description: "StackOverflow hot questions",
  columns: ["rank", "title", "score", "answers", "tags"],
  args: { limit: { type: "int", default: 20 } },
  health: { min_rows: 5, non_empty: ["title"] },

  async run(page, args) {
    await page.nav("https://stackoverflow.com")
    await page.wait(2000)

    const items = await page.eval(() => {
      return (async () => {
        const res = await fetch('https://api.stackexchange.com/2.3/questions?order=desc&sort=hot&site=stackoverflow&pagesize=50')
        const data = await res.json()
        return data.items.map((q, i) => ({
          rank: String(i + 1),
          title: q.title,
          score: String(q.score),
          answers: String(q.answer_count),
          tags: q.tags.join(', ')
        }))
      })()
    })

    return items.slice(0, args.limit)
  }
}
