export default {
  site: "steam",
  name: "top-sellers",
  description: "Steam top selling games",
  columns: ["rank", "title", "price", "discount"],
  args: { limit: { type: "int", default: 20 } },
  health: { min_rows: 5, non_empty: ["title"] },

  async run(page, args) {
    await page.nav("https://store.steampowered.com")
    await page.wait(2000)

    const items = await page.eval(async () => {
      const res = await fetch('https://store.steampowered.com/api/featuredcategories?cc=us&l=english')
      const data = await res.json()
      return (data.top_sellers.items || []).map((g, i) => ({
        rank: String(i + 1),
        title: String(g.name),
        price: g.final_price ? '$' + (g.final_price / 100).toFixed(2) : 'Free',
        discount: g.discount_percent ? g.discount_percent + '% off' : '-'
      }))
    })

    return items.slice(0, args.limit)
  }
}
