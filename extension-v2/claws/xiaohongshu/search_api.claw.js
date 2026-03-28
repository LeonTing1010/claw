export default {
  site: "xiaohongshu",
  name: "search_api",
  description: "从 SSR state 提取小红书搜索完整数据（含评论数、收藏数、作者）",
  columns: ["title", "likes", "comments", "collects", "author", "note_id"],
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
      const feeds = window.__INITIAL_STATE__?.search?.feeds?._value
      if (!feeds) return []
      const arr = Array.isArray(feeds) ? feeds : Object.values(feeds)
      return arr.map(item => {
        const nc = item?.noteCard || {}
        const interact = nc?.interactInfo || {}
        return {
          title: String(nc?.displayTitle || ''),
          likes: String(interact?.likedCount || '0'),
          comments: String(interact?.commentCount || '0'),
          collects: String(interact?.collectedCount || '0'),
          author: String(nc?.user?.nickname || ''),
          note_id: String(item?.id || '')
        }
      }).filter(item => item.title || item.likes !== '0')
    })

    return items.slice(0, args.limit)
  }
}
