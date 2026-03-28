export default {
  site: "xiaohongshu",
  name: "post_detail",
  description: "搜索→点击→从弹窗SSR state提取帖子详情+评论内容",
  columns: ["type", "content", "likes", "author"],
  args: {
    keyword: { type: "string" },
    index: { type: "int", default: 1 }
  },
  health: { min_rows: 1, non_empty: ["content"] },

  async run(page, args) {
    // Step 1: search
    await page.nav(`https://www.xiaohongshu.com/search_result?keyword=${encodeURIComponent(args.keyword)}&type=51`)
    await page.waitFor("section.note-item", 10000)
    await page.wait(2000)

    // Step 2: CDP native click to open detail modal
    await page.clickSelector(`section.note-item:nth-child(${args.index}) a.cover`)

    // Step 3: wait for modal and comments to load in SSR state
    await page.wait(5000)

    // Step 4: extract from __INITIAL_STATE__.note.noteDetailMap
    const items = await page.eval(() => {
      const results = []

      let noteMap
      try {
        noteMap = JSON.parse(JSON.stringify(
          window.__INITIAL_STATE__?.note?.noteDetailMap || {}
        ))
      } catch (e) {
        return [{ type: 'error', content: 'parse failed: ' + e.message, likes: '0', author: '' }]
      }

      for (const [noteId, detail] of Object.entries(noteMap)) {
        const note = detail?.note || {}
        if (!note.title && !note.desc && !note.noteId) continue

        const interact = note.interactInfo || {}

        // Note info
        results.push({
          type: 'note',
          content: (note.title || note.displayTitle || '') + '\n' + (note.desc || '').substring(0, 500),
          likes: String(interact.likedCount || 0),
          author: String((note.user || {}).nickname || '')
        })

        // Engagement summary
        results.push({
          type: 'engagement',
          content: 'likes:' + (interact.likedCount || 0) + ' comments:' + (interact.commentCount || 0) + ' collects:' + (interact.collectedCount || 0),
          likes: String(interact.likedCount || 0),
          author: ''
        })

        // Comments
        const commentList = detail?.comments?.list || []
        for (const c of commentList) {
          const text = c.content || ''
          if (text.length < 2) continue
          results.push({
            type: 'comment',
            content: text.substring(0, 300),
            likes: String(c.likeCount || 0),
            author: String((c.userInfo || {}).nickname || '')
          })

          // Sub-comments (replies)
          const subComments = c.subComments || []
          for (const sc of subComments) {
            if ((sc.content || '').length < 2) continue
            results.push({
              type: 'reply',
              content: sc.content.substring(0, 300),
              likes: String(sc.likeCount || 0),
              author: String((sc.userInfo || {}).nickname || '')
            })
          }
        }
      }

      if (results.length === 0) {
        results.push({ type: 'info', content: 'no note detail loaded in state', likes: '0', author: '' })
      }

      return results
    })

    return items
  }
}
