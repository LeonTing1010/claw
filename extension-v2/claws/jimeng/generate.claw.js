export default {
  site: "jimeng",
  name: "generate",
  description: "即梦AI 文生图 — 提交 prompt 触发生成",
  columns: ["status", "prompt"],
  args: {
    prompt: { type: "string" }
  },

  async run(page, args) {
    await page.nav("https://jimeng.jianying.com/ai-tool/home")
    await page.waitFor(".tiptap", 20000)
    await page.wait(1000)

    // 切换到图片生成模式
    await page.click("图片生成")
    await page.wait(500)
    await page.click("图片生成")
    await page.wait(1000)

    // 找到主输入框 (最大的textbox) and type prompt
    const target = await page.eval(() => {
      const inputs = document.querySelectorAll('[role="textbox"], .tiptap')
      let best = '.tiptap'
      let maxArea = 0
      inputs.forEach(el => {
        const rect = el.getBoundingClientRect()
        const area = rect.width * rect.height
        if (area > maxArea) {
          maxArea = area
          best = el.className ? '.' + el.className.split(' ').join('.') : el.tagName.toLowerCase()
        }
      })
      return best
    })

    await page.type(target, args.prompt)
    await page.wait(500)

    // 找最近的按钮点击
    await page.eval(() => {
      const buttons = document.querySelectorAll('button')
      if (buttons.length > 0) {
        buttons[buttons.length - 1].click()
      }
    })
    await page.wait(3000)

    return [{
      status: "submitted",
      prompt: args.prompt.substring(0, 80)
    }]
  }
}
