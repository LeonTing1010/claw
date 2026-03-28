export default {
  site: "youtube",
  name: "trending",
  description: "YouTube Trending Videos",
  columns: ["rank", "title", "hot"],
  args: { limit: { type: "int", default: 20 } },
  health: { min_rows: 5, non_empty: ["title"] },

  async run(page, args) {
    await page.nav("https://www.youtube.com/feed/trending")
    await page.wait(4000)

    const items = await page.eval(() => {
      const items = []
      const videos = document.querySelectorAll('ytd-video-renderer, ytd-rich-item-renderer')
      videos.forEach((el, i) => {
        const titleEl = el.querySelector('#video-title, h3 a, a#video-title-link')
        const viewsEl = el.querySelector('.inline-metadata-item, #metadata-line span, .ytd-video-meta-block')
        const title = titleEl ? titleEl.textContent.trim() : ''
        const views = viewsEl ? viewsEl.textContent.trim() : '0'
        if (title) {
          items.push({ rank: String(i + 1), title: title, hot: views })
        }
      })
      // Fallback: generic video title extraction
      if (items.length === 0) {
        document.querySelectorAll('a[href*="/watch"]').forEach((a, i) => {
          const text = (a.getAttribute('title') || a.textContent || '').trim()
          if (text && text.length > 5 && text.length < 200 && !items.some(x => x.title === text)) {
            items.push({ rank: String(i + 1), title: text, hot: '0' })
          }
        })
      }
      return items
    })

    return items.slice(0, args.limit)
  }
}
