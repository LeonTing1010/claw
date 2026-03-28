export default {
  site: "pypi",
  name: "top",
  description: "PyPI top Python packages",
  columns: ["project", "download_count"],
  args: { limit: { type: "int", default: 20 } },
  health: { min_rows: 10, non_empty: ["project"] },

  async run(page, args) {
    const data = await page.fetch("https://hugovk.github.io/top-pypi-packages/top-pypi-packages-30-days.min.json")
    return data.rows.map(item => ({
      project: item.project,
      download_count: String(item.download_count)
    })).slice(0, args.limit)
  }
}
