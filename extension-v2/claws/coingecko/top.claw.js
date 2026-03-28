export default {
  site: "coingecko",
  name: "top",
  description: "Top cryptocurrencies by market cap",
  columns: ["name", "symbol", "price", "change_24h"],
  args: { limit: { type: "int", default: 20 } },
  health: { min_rows: 5, non_empty: ["name", "symbol"] },

  async run(page, args) {
    await page.nav("https://www.coingecko.com")
    await page.wait(2000)

    const items = await page.eval(() => {
      return (async () => {
        const res = await fetch('https://api.coingecko.com/api/v3/coins/markets?vs_currency=usd&order=market_cap_desc&per_page=50')
        const data = await res.json()
        return data.map(c => ({
          name: c.name,
          symbol: c.symbol.toUpperCase(),
          price: '$' + (c.current_price || 0).toLocaleString(),
          change_24h: (c.price_change_percentage_24h >= 0 ? '+' : '') + (c.price_change_percentage_24h || 0).toFixed(2) + '%'
        }))
      })()
    })

    return items.slice(0, args.limit)
  }
}
