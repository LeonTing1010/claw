export default {
  site: "douyin",
  name: "search",
  description: "搜索抖音视频，返回标题+点赞数+作者",
  columns: ["title", "likes", "author", "url"],
  args: {
    keyword: { type: "string" },
    limit: { type: "int", default: 20 }
  },
  health: { min_rows: 3, non_empty: ["title"] },

  async run(page, args) {
    await page.nav("https://www.douyin.com")
    await page.wait(3000)

    const keyword = args.keyword
    const items = await page.eval(async (kw) => {
      try {
        const params = new URLSearchParams({
          search_channel: 'aweme_general',
          keyword: kw,
          count: '20',
          offset: '0',
          need_filter_settings: '1',
          device_platform: 'webapp',
          aid: '6383'
        })
        const res = await fetch(
          'https://www.douyin.com/aweme/v1/web/general/search/single/?' + params.toString(),
          { credentials: 'include' }
        )
        const data = await res.json()
        if (data.data) {
          return data.data
            .filter(item => item.aweme_info)
            .map(item => {
              const info = item.aweme_info
              const stats = info.statistics || {}
              return {
                title: info.desc || '',
                likes: String(stats.digg_count || 0),
                author: info.author?.nickname || '',
                url: 'https://www.douyin.com/video/' + (info.aweme_id || '')
              }
            })
        }
        return [{ title: 'API requires login — visit douyin.com first', likes: '0', author: '', url: '' }]
      } catch (e) {
        return [{ title: 'Error: ' + e.message, likes: '0', author: '', url: '' }]
      }
    }, keyword)

    return items.slice(0, args.limit)
  }
}
