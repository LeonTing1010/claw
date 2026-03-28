export default {
  site: "jimeng",
  name: "history",
  description: "即梦AI 查看最近生成的作品",
  columns: ["prompt", "status", "image_url", "created_at"],
  args: {
    limit: { type: "int", default: 5 }
  },

  async run(page, args) {
    await page.nav("https://jimeng.jianying.com/ai-tool/home")
    await page.wait(2000)

    const items = await page.eval(async (limit) => {
      const res = await fetch('/mweb/v1/get_history?aid=513695&device_platform=web&region=cn', {
        method: 'POST',
        credentials: 'include',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          cursor: '',
          count: limit,
          need_page_item: true,
          need_aigc_data: true,
          aigc_mode_list: ['workbench']
        })
      })
      const data = await res.json()
      const records = data?.data?.records_list || []
      const rows = []
      for (const rec of records.slice(0, limit)) {
        const items = rec.item_list || []
        const created = new Date((rec.created_time || 0) * 1000).toLocaleString('zh-CN')
        for (const item of items) {
          const attr = item.common_attr || {}
          const urlMap = attr.cover_url_map || {}
          const url = urlMap['1080'] || urlMap['720'] || attr.cover_url || ''
          rows.push({
            prompt: attr.description || 'N/A',
            status: url ? 'completed' : 'pending',
            image_url: String(url),
            created_at: String(created)
          })
        }
      }
      return rows
    }, args.limit)

    return items.slice(0, args.limit)
  }
}
