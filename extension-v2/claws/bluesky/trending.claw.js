export default {
  site: "bluesky",
  name: "trending",
  description: "Bluesky Trending Topics",
  columns: ["topic"],
  args: { limit: { type: "int", default: 20 } },
  health: { min_rows: 3, non_empty: ["topic"] },

  async run(page, args) {
    const data = await page.fetch("https://public.api.bsky.app/xrpc/app.bsky.unspecced.getTrendingTopics")
    const topics = data.topics || []
    return topics.map(item => ({
      topic: item.topic
    })).slice(0, args.limit)
  }
}
