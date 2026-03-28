export default {
  site: "dictionary",
  name: "search",
  description: "English dictionary lookup (Free Dictionary API)",
  columns: ["word", "partOfSpeech", "definition", "example"],
  args: { word: { type: "string" } },
  health: { min_rows: 1, non_empty: ["word", "definition"] },

  async run(page, args) {
    await page.nav("https://dictionaryapi.dev")
    await page.wait(1000)

    const items = await page.eval(async (w) => {
      const res = await fetch('https://api.dictionaryapi.dev/api/v2/entries/en/' + encodeURIComponent(w))
      if (!res.ok) return [{ word: w, partOfSpeech: '-', definition: 'Not found', example: '-' }]
      const data = await res.json()
      const results = []
      for (const entry of data) {
        for (const m of entry.meanings) {
          for (const d of m.definitions.slice(0, 2)) {
            results.push({
              word: String(entry.word),
              partOfSpeech: String(m.partOfSpeech),
              definition: String(d.definition),
              example: String(d.example || '-')
            })
          }
        }
      }
      return results
    }, args.word)

    return items
  }
}
