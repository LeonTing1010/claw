export default {
  site: "github",
  name: "trending",
  description: "GitHub Trending repositories",
  columns: ["repo", "description", "stars", "language"],
  args: { limit: { type: "int", default: 20 } },
  health: { min_rows: 5, non_empty: ["repo"] },

  async run(page, args) {
    await page.nav("https://github.com/trending")
    await page.waitFor("article.Box-row", 10000)
    await page.wait(2000)

    const items = await page.eval(() => {
      return Array.from(document.querySelectorAll('article.Box-row')).map(el => {
        const repo = el.querySelector('h2 a')?.textContent?.trim().replace(/\s+/g, '') || ''
        const descEl = el.querySelector('p.col-9')
        const description = descEl ? descEl.textContent.trim() : ''
        const stars = el.querySelector('[href$="/stargazers"]')?.textContent?.trim() || ''
        const language = el.querySelector('[itemprop="programmingLanguage"]')?.textContent?.trim() || ''
        return { repo, description, stars, language }
      }).filter(item => item.repo.length > 0)
    })

    return items.slice(0, args.limit)
  }
}
