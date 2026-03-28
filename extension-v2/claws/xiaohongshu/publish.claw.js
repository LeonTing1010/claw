export default {
  site: "xiaohongshu",
  name: "publish",
  description: "发布小红书图文笔记",
  columns: ["status", "url"],
  args: {
    title: { type: "string", default: "" },
    content: { type: "string", default: "" },
    images: { type: "string" }
  },

  async run(page, args) {
    await page.nav("https://creator.xiaohongshu.com/publish/publish")
    await page.waitFor(".creator-tab", 10000)

    await page.click("上传图文")
    await page.wait(2000)

    await page.upload("input.upload-input", args.images)
    await page.wait(20000)

    if (args.title) {
      await page.type("input.d-text", args.title)
      await page.wait(500)
    }

    if (args.content) {
      await page.type(".tiptap.ProseMirror", args.content)
      await page.wait(500)
    }

    await page.click("发布")
    await page.wait(5000)

    const url = await page.eval(() => location.href)
    return [{
      status: url.includes('/publish/publish') ? 'check-browser' : 'published',
      url
    }]
  }
}
