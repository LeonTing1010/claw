export default {
  site: "xiaohongshu",
  name: "search",
  description: "搜索小红书笔记，返回标题+点赞数+链接",
  columns: ["title", "likes", "url", "author"],
  args: {
    keyword: { type: "string" },
    limit: { type: "int", default: 20 }
  },
  health: { min_rows: 3, non_empty: ["title"] },

  async run(page, args) {
    await page.nav(`https://www.xiaohongshu.com/search_result?keyword=${encodeURIComponent(args.keyword)}&type=51`)
    await page.waitFor("section.note-item", 10000)
    await page.wait(2000)

    const items = await page.eval(() => {
      return Array.from(document.querySelectorAll('section.note-item')).map(el => {
        const linkEl = el.querySelector('a')
        const href = linkEl ? linkEl.getAttribute('href') : ''
        const title = el.querySelector('.title span')?.innerText
          || el.querySelector('a.title')?.innerText
          || ''
        const likes = el.querySelector('.like-wrapper .count')?.innerText || '0'
        const author = el.querySelector('.author-wrapper .name')?.innerText
          || el.querySelector('.nickname')?.innerText
          || ''
        return {
          title: title.trim(),
          likes: likes.trim(),
          url: href ? 'https://www.xiaohongshu.com' + href : '',
          author: author.trim()
        }
      }).filter(item => item.title.length > 0)
    })

    return items.slice(0, args.limit)
  }
}
