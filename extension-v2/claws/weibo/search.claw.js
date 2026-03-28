export default {
  site: "weibo",
  name: "search",
  description: "搜索微博内容，返回标题+互动数+作者",
  columns: ["title", "likes", "author", "url"],
  args: {
    keyword: { type: "string" },
    limit: { type: "int", default: 20 }
  },
  health: { min_rows: 3, non_empty: ["title"] },

  async run(page, args) {
    await page.nav("https://weibo.com")
    await page.wait(3000)

    const items = await page.eval(async (keyword) => {
      try {
        const encoded = encodeURIComponent(keyword)
        const containerid = '100103type=1&q=' + encoded
        const res = await fetch(
          'https://m.weibo.cn/api/container/getIndex?containerid=' + encodeURIComponent(containerid),
          { credentials: 'include' }
        )
        const data = await res.json()
        const cards = data.data?.cards || []
        const results = []

        for (const card of cards) {
          if (card.card_type === 9) {
            const mblog = card.mblog
            if (!mblog) continue
            const text = (mblog.text || '').replace(/<[^>]+>/g, '').trim()
            results.push({
              title: text.substring(0, 120),
              likes: String(mblog.attitudes_count || 0),
              author: String(mblog.user?.screen_name || ''),
              url: 'https://weibo.com/' + (mblog.user?.id || '') + '/' + (mblog.bid || '')
            })
          } else if (card.card_group) {
            for (const sub of card.card_group) {
              if (sub.card_type === 9 && sub.mblog) {
                const text = (sub.mblog.text || '').replace(/<[^>]+>/g, '').trim()
                results.push({
                  title: text.substring(0, 120),
                  likes: String(sub.mblog.attitudes_count || 0),
                  author: String(sub.mblog.user?.screen_name || ''),
                  url: 'https://weibo.com/' + (sub.mblog.user?.id || '') + '/' + (sub.mblog.bid || '')
                })
              }
            }
          }
        }

        return results.length > 0 ? results
          : [{ title: 'No results - try visiting weibo.com first', likes: '0', author: '', url: '' }]
      } catch (e) {
        return [{ title: 'Error: ' + e.message, likes: '0', author: '', url: '' }]
      }
    }, args.keyword)

    return items.slice(0, args.limit)
  }
}
