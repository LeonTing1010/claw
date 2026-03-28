export default {
  site: "facebook",
  name: "feed",
  description: "Facebook News Feed",
  columns: ["rank", "author", "text", "url"],
  args: { limit: { type: "int", default: 20 } },
  health: { min_rows: 3, non_empty: ["author"] },

  async run(page, args) {
    await page.nav("https://www.facebook.com/")
    await page.wait(5000)

    const items = await page.eval(() => {
      const items = []
      const articles = document.querySelectorAll('[role="article"]')
      articles.forEach((el, i) => {
        const authorEl = el.querySelector('h3 a, h4 a, [data-ad-preview="headline"] a, strong a')
        const author = authorEl ? authorEl.textContent.trim() : ''
        const textEls = el.querySelectorAll('[dir="auto"]')
        let text = ''
        for (const t of textEls) {
          const content = t.textContent.trim()
          if (content.length > text.length && content.length < 500) text = content
        }
        const linkEl = el.querySelector('a[href*="/posts/"], a[href*="/photos/"], a[href*="/videos/"], a[href*="permalink"]')
        const url = linkEl ? linkEl.href : ''
        if (author || text) {
          items.push({
            rank: String(i + 1),
            author: author,
            text: text.substring(0, 200),
            url: url
          })
        }
      })
      return items
    })

    return items.slice(0, args.limit)
  }
}
