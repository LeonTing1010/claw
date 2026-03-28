export default {
  site: "instagram",
  name: "explore",
  description: "Instagram Explore",
  columns: ["rank", "caption", "author", "likes"],
  args: { limit: { type: "int", default: 20 } },
  health: { min_rows: 3, non_empty: ["caption"] },

  async run(page, args) {
    await page.nav("https://www.instagram.com/explore/")
    await page.wait(5000)

    const items = await page.eval(() => {
      const items = []
      const posts = document.querySelectorAll('a[href*="/p/"], a[href*="/reel/"]')
      const seen = new Set()
      for (const a of posts) {
        const href = a.href
        if (seen.has(href)) continue
        seen.add(href)
        const img = a.querySelector('img')
        const caption = img ? (img.alt || '').trim() : ''
        const likeEl = a.querySelector('[class*="like"], span')
        const likes = likeEl ? likeEl.textContent.trim() : '0'
        items.push({
          rank: String(items.length + 1),
          caption: caption.substring(0, 200),
          author: '',
          likes: likes
        })
      }
      return items
    })

    return items.slice(0, args.limit)
  }
}
