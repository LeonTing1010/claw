/**
 * page API — the 10 system calls a .claw.js can use.
 *
 * Scripting mode (undetectable): nav, wait, waitFor, eval, fetch, screenshot, cookies
 * Debugger mode (ms-level attach/detach): click, type, upload
 */

/**
 * Create a page API bound to a specific tab.
 * @param {number} tabId - Chrome tab ID
 * @returns {object} page API object
 */
export function createPageAPI(tabId) {
  let currentUrl = ''

  const page = {
    /** Navigate to URL. Uses chrome.tabs (undetectable). */
    async nav(url) {
      await chrome.tabs.update(tabId, { url })
      await waitForTabLoad(tabId)
      const tab = await chrome.tabs.get(tabId)
      currentUrl = tab.url || url
    },

    /** Wait fixed milliseconds. */
    async wait(ms) {
      return new Promise(resolve => setTimeout(resolve, ms))
    },

    /** Wait for a CSS selector to appear in the page. Polls via chrome.scripting. */
    async waitFor(selector, timeoutMs = 10000) {
      const start = Date.now()
      while (Date.now() - start < timeoutMs) {
        const results = await chrome.scripting.executeScript({
          target: { tabId },
          func: (sel) => !!document.querySelector(sel),
          args: [selector],
          world: 'MAIN'
        })
        if (results?.[0]?.result) return
        await new Promise(r => setTimeout(r, 300))
      }
      throw new Error(`waitFor: "${selector}" not found within ${timeoutMs}ms`)
    },

    /**
     * Click an element. Accepts CSS selector or visible text.
     * Uses chrome.debugger for CDP native Input.dispatchMouseEvent (isTrusted=true).
     * Attaches debugger, clicks, detaches — millisecond exposure.
     */
    async click(target) {
      // Find element coordinates via scripting (undetectable)
      const results = await chrome.scripting.executeScript({
        target: { tabId },
        func: (t) => {
          // Try as CSS selector first
          let el = document.querySelector(t)
          // Fallback: find by visible text
          if (!el) {
            const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_ELEMENT)
            while (walker.nextNode()) {
              if (walker.currentNode.textContent?.trim() === t && walker.currentNode.offsetParent !== null) {
                el = walker.currentNode
                break
              }
            }
          }
          if (!el) return null
          const rect = el.getBoundingClientRect()
          return { x: rect.x + rect.width / 2, y: rect.y + rect.height / 2 }
        },
        args: [target],
        world: 'MAIN'
      })

      const pos = results?.[0]?.result
      if (!pos) throw new Error(`click: target "${target}" not found`)

      // Debugger: attach → click → detach
      await withDebugger(tabId, async () => {
        const params = { x: pos.x, y: pos.y, button: 'left', clickCount: 1 }
        await chrome.debugger.sendCommand({ tabId }, 'Input.dispatchMouseEvent', { type: 'mousePressed', ...params })
        await chrome.debugger.sendCommand({ tabId }, 'Input.dispatchMouseEvent', { type: 'mouseReleased', ...params })
      })
    },

    /**
     * Type text into an element (by CSS selector).
     * Uses chrome.debugger for CDP native keyboard events.
     */
    async type(selector, text) {
      // Focus the element via scripting
      await chrome.scripting.executeScript({
        target: { tabId },
        func: (sel) => {
          const el = document.querySelector(sel)
          if (el) { el.focus(); el.click() }
        },
        args: [selector],
        world: 'MAIN'
      })
      await new Promise(r => setTimeout(r, 100))

      // Type via debugger
      await withDebugger(tabId, async () => {
        for (const char of text) {
          await chrome.debugger.sendCommand({ tabId }, 'Input.dispatchKeyEvent', {
            type: 'keyDown', text: char, key: char, code: `Key${char.toUpperCase()}`
          })
          await chrome.debugger.sendCommand({ tabId }, 'Input.dispatchKeyEvent', {
            type: 'keyUp', key: char, code: `Key${char.toUpperCase()}`
          })
        }
      })
    },

    /**
     * Upload files to a file input element.
     * Uses chrome.debugger for CDP DOM.setFileInputFiles.
     */
    async upload(selector, files) {
      const fileList = typeof files === 'string' ? files.split(',').map(f => f.trim()) : files

      await withDebugger(tabId, async () => {
        // Get node ID
        const doc = await chrome.debugger.sendCommand({ tabId }, 'DOM.getDocument', {})
        const node = await chrome.debugger.sendCommand({ tabId }, 'DOM.querySelector', {
          nodeId: doc.root.nodeId, selector
        })
        await chrome.debugger.sendCommand({ tabId }, 'DOM.setFileInputFiles', {
          nodeId: node.nodeId, files: fileList
        })
      })
    },

    /**
     * Execute a function in the page's JS context. Uses chrome.scripting (undetectable).
     * @param {Function} fn - Function to execute in page context
     * @param {...any} args - Arguments to pass to the function
     * @returns {any} Return value from the function
     */
    async eval(fn, ...args) {
      const results = await chrome.scripting.executeScript({
        target: { tabId },
        func: fn,
        args,
        world: 'MAIN'
      })
      return results?.[0]?.result
    },

    /**
     * Fetch a URL with the page's cookies. Uses chrome.scripting → page fetch() (undetectable).
     * @returns {any} Parsed JSON response
     */
    async fetch(url, opts = {}) {
      const results = await chrome.scripting.executeScript({
        target: { tabId },
        func: async (u, o) => {
          const res = await fetch(u, { credentials: 'include', ...o })
          return res.json()
        },
        args: [url, opts],
        world: 'MAIN'
      })
      return results?.[0]?.result
    },

    /** Capture screenshot of visible area. Returns base64 data URL. */
    async screenshot() {
      return await chrome.tabs.captureVisibleTab(null, { format: 'png' })
    },

    /** Read cookies for the current page's domain. */
    async cookies() {
      return await chrome.cookies.getAll({ url: currentUrl })
    },

    /**
     * Run another claw (composition).
     * @param {string} site - Site name
     * @param {string} name - Claw name
     * @param {object} args - Arguments
     * @returns {Array} Rows from the claw
     */
    async claw(site, name, args = {}) {
      // This is wired up by the executor — placeholder here
      throw new Error('page.claw() not wired — must be set by executor')
    }
  }

  return page
}

// --- Helpers ---

/** Wait for a tab to finish loading. */
function waitForTabLoad(tabId) {
  return new Promise((resolve) => {
    const onUpdated = (id, changeInfo) => {
      if (id === tabId && changeInfo.status === 'complete') {
        chrome.tabs.onUpdated.removeListener(onUpdated)
        resolve()
      }
    }
    chrome.tabs.onUpdated.addListener(onUpdated)
    setTimeout(() => {
      chrome.tabs.onUpdated.removeListener(onUpdated)
      resolve()
    }, 30000)
  })
}

/**
 * Attach debugger, run callback, detach debugger.
 * Minimizes detection window to milliseconds.
 */
async function withDebugger(tabId, fn) {
  await chrome.debugger.attach({ tabId }, '1.3')
  try {
    await fn()
  } finally {
    await chrome.debugger.detach({ tabId }).catch(() => {})
  }
}
