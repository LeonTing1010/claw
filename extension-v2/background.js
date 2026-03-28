/**
 * Claw v2 — Background Service Worker
 *
 * One extension, three interfaces:
 *   1. WebSocket bridge — Rust MCP server sends CDP commands + claw actions
 *   2. chrome.runtime.onMessage — popup UI
 *   3. chrome.runtime.onMessageExternal — web pages, other extensions
 *
 * Handles both CDP protocol (Page.navigate, Runtime.evaluate, Input.*)
 * and claw protocol (action: "list", action: "run").
 */

import { registerClaw, listClaws, getClaw, runClaw, parseClawURL } from './runtime/executor.js'
import { gatherPageIntelligence } from './runtime/page-intelligence.js'

// --- Claw Registration ---

import hackernewsHot from './claws/hackernews/hot.claw.js'
import weiboHot from './claws/weibo/hot.claw.js'
import xiaohongshuHot from './claws/xiaohongshu/hot.claw.js'
import xiaohongshuSearch from './claws/xiaohongshu/search.claw.js'
import xiaohongshuPublish from './claws/xiaohongshu/publish.claw.js'
import githubTrending from './claws/github/trending.claw.js'

const BUNDLED_CLAWS = [
  hackernewsHot,
  weiboHot,
  xiaohongshuHot,
  xiaohongshuSearch,
  xiaohongshuPublish,
  githubTrending,
]

for (const mod of BUNDLED_CLAWS) {
  registerClaw(mod)
}

console.log(`[claw] registered ${BUNDLED_CLAWS.length} claws`)

// --- State ---

let activeTabId = null

// --- CDP Command Router ---
// Speaks the same protocol as the Rust CdpClient.send(method, params).
// Scripting mode by default, debugger only for Input/DOM/Accessibility.

async function routeCDP(method, params = {}) {
  // Auto-recover: if no tab or tab is gone, create one
  if (activeTabId) {
    try { await chrome.tabs.get(activeTabId) }
    catch { activeTabId = null }
  }
  if (!activeTabId) {
    const tab = await chrome.tabs.create({ url: 'about:blank' })
    activeTabId = tab.id
    console.log(`[claw] created new tab ${tab.id}`)
  }

  switch (method) {
    // --- Scripting mode (undetectable) ---

    case 'Page.navigate': {
      // Can't navigate chrome:// tabs — create a new one
      const current = await chrome.tabs.get(activeTabId)
      if (current.url?.startsWith('chrome://')) {
        const tab = await chrome.tabs.create({ url: params.url })
        activeTabId = tab.id
        console.log(`[claw] created tab ${tab.id} (was on chrome:// page)`)
      } else {
        await chrome.tabs.update(activeTabId, { url: params.url })
      }
      await waitForTabLoad(activeTabId)
      return { frameId: 'main' }
    }

    case 'Runtime.evaluate': {
      // Use debugger for arbitrary expression eval — bypasses page CSP
      return await withDebugger(async () => {
        const result = await chrome.debugger.sendCommand(
          { tabId: activeTabId },
          'Runtime.evaluate',
          {
            expression: params.expression,
            returnByValue: true,
            awaitPromise: true,
          }
        )
        return result
      })
    }

    case 'Page.captureScreenshot': {
      const dataUrl = await chrome.tabs.captureVisibleTab(null, { format: 'png' })
      // Strip data URL prefix, return raw base64 like CDP does
      const base64 = dataUrl.replace(/^data:image\/png;base64,/, '')
      return { data: base64 }
    }

    case 'Network.getCookies': {
      const tab = await chrome.tabs.get(activeTabId)
      const cookies = await chrome.cookies.getAll({ url: tab.url })
      return { cookies }
    }

    // --- No-ops (not needed in real browser) ---

    case 'Page.enable':
    case 'Network.enable':
    case 'Network.disable':
    case 'Network.setUserAgentOverride':
      return {}

    case 'Page.addScriptToEvaluateOnNewDocument':
      return { identifier: 'skipped' }

    // --- Debugger mode (ms-level attach/detach) ---

    default:
      return await withDebugger(async () => {
        return await chrome.debugger.sendCommand({ tabId: activeTabId }, method, params)
      })
  }
}

// --- Bridge Commands ---

async function handleBridgeCommand(method, params = {}) {
  switch (method) {
    case 'Bridge.ping':
      return { pong: true }

    case 'Bridge.getTargets': {
      const tabs = await chrome.tabs.query({})
      return tabs.map(t => ({
        id: String(t.id), type: 'page', title: t.title || '', url: t.url || '',
        webSocketDebuggerUrl: `bridge://tab/${t.id}`
      }))
    }

    case 'Bridge.attach': {
      const tabId = params.tabId
        ? Number(params.tabId)
        : (await chrome.tabs.query({ active: true, currentWindow: true }))[0]?.id

      if (!tabId) return { error: 'No tab to attach' }
      activeTabId = tabId
      console.log(`[claw] attached to tab ${tabId}`)
      return { tabId, attached: true, mode: 'scripting' }
    }

    case 'Bridge.detach': {
      activeTabId = null
      return { detached: true }
    }

    case 'Bridge.newTab': {
      const tab = await chrome.tabs.create({ url: params.url || 'about:blank' })
      return { tabId: tab.id, url: tab.url }
    }

    default:
      return { error: `Unknown bridge command: ${method}` }
  }
}

// --- Claw Protocol Commands (via bridge WebSocket) ---

async function handleClawCommand(method, params = {}) {
  switch (method) {
    case 'Claw.pageIntelligence': {
      const tabId = params.tabId || activeTabId
      if (!tabId) throw new Error('No tab. Call Bridge.attach first.')
      return await gatherPageIntelligence(tabId)
    }

    case 'Claw.run': {
      return await handleClawAction({ action: 'run', ...params })
    }

    case 'Claw.list': {
      return await handleClawAction({ action: 'list' })
    }

    case 'Claw.sync': {
      return await syncClaws()
    }

    default:
      throw new Error(`Unknown Claw command: ${method}`)
  }
}

// --- Claw Action Handler ---

async function handleClawAction(msg) {
  switch (msg.action) {
    case 'list': {
      // Merge bundled claws with synced claws
      const bundled = listClaws()
      const stored = (await chrome.storage.local.get('synced_claws'))?.synced_claws || {}
      const bundledKeys = new Set(bundled.map(c => `${c.site}/${c.name}.claw.js`))
      const synced = Object.keys(stored)
        .filter(k => !bundledKeys.has(k))
        .map(k => {
          const [site, file] = k.split('/')
          const name = file.replace('.claw.js', '')
          return { site, name, description: '(synced)', columns: [] }
        })
      return { claws: [...bundled, ...synced] }
    }

    case 'run': {
      let site, name, args
      if (msg.url) {
        ({ site, name, args } = parseClawURL(msg.url))
        args = { ...args, ...msg.args }
      } else {
        ({ site, name } = msg)
        args = msg.args || {}
      }

      let tabId = msg.tabId || activeTabId
      if (!tabId) {
        const [tab] = await chrome.tabs.query({ active: true, currentWindow: true })
        tabId = tab?.id
      }
      if (!tabId) throw new Error('no tab available')

      // Try bundled first, fall back to synced (dynamic sandbox execution)
      const mod = getClaw(site, name)
      if (mod) {
        return await runClaw(site, name, args, tabId)
      }
      return await runSyncedClaw(site, name, args, tabId)
    }

    case 'ping':
      return { pong: true, claws: listClaws().length }

    default:
      throw new Error(`unknown action: ${msg.action}`)
  }
}

// --- Unified Message Router ---
// Handles both CDP commands and claw actions from any source.

async function handleMessage(msg) {
  const { method, params, action } = msg

  // Claw actions: { action: "list" } or { action: "run", site, name }
  if (action) {
    return await handleClawAction(msg)
  }

  // Bridge meta-commands: { method: "Bridge.attach" }
  if (method && method.startsWith('Bridge.')) {
    return await handleBridgeCommand(method, params || {})
  }

  // Claw commands: { method: "Claw.pageIntelligence" }, { method: "Claw.run" }
  if (method && method.startsWith('Claw.')) {
    return await handleClawCommand(method, params || {})
  }

  // CDP commands: { method: "Page.navigate", params: { url: "..." } }
  if (method) {
    return await routeCDP(method, params || {})
  }

  throw new Error('invalid message: need "action" or "method"')
}

// --- chrome.runtime listeners ---

chrome.runtime.onMessage.addListener((msg, sender, sendResponse) => {
  handleMessage(msg)
    .then(sendResponse)
    .catch(err => sendResponse({ error: err.message }))
  return true
})

chrome.runtime.onMessageExternal.addListener((msg, sender, sendResponse) => {
  handleMessage(msg)
    .then(sendResponse)
    .catch(err => sendResponse({ error: err.message }))
  return true
})

// --- WebSocket Bridge (for Rust MCP server) ---

const BRIDGE_PORT = 9333
let ws = null
let reconnectDelay = 1000

function connectBridge() {
  try {
    ws = new WebSocket(`ws://127.0.0.1:${BRIDGE_PORT}`)
  } catch {
    scheduleBridgeReconnect()
    return
  }

  ws.onopen = () => {
    console.log('[claw] bridge connected')
    reconnectDelay = 1000
  }

  ws.onmessage = async (event) => {
    let msg
    try { msg = JSON.parse(event.data) } catch { return }

    const { id } = msg
    try {
      const result = await handleMessage(msg)
      if (id !== undefined) wsSend({ id, result: result || {} })
    } catch (err) {
      if (id !== undefined) wsSend({ id, error: { code: -32000, message: err.message } })
    }
  }

  ws.onclose = () => {
    console.log('[claw] bridge disconnected')
    scheduleBridgeReconnect()
  }

  ws.onerror = () => scheduleBridgeReconnect()
}

function scheduleBridgeReconnect() {
  setTimeout(() => {
    reconnectDelay = Math.min(reconnectDelay * 2, 30000)
    connectBridge()
  }, reconnectDelay)
}

function wsSend(msg) {
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify(msg))
  }
}

// --- CDP Event Forwarding (when debugger is attached by withDebugger) ---

chrome.debugger.onEvent.addListener((source, method, params) => {
  if (source.tabId === activeTabId) {
    wsSend({ method, params })
  }
})

// --- Helpers ---

function waitForTabLoad(tabId) {
  return new Promise(resolve => {
    const onUpdated = (id, changeInfo) => {
      if (id === tabId && changeInfo.status === 'complete') {
        chrome.tabs.onUpdated.removeListener(onUpdated)
        resolve()
      }
    }
    chrome.tabs.onUpdated.addListener(onUpdated)
    setTimeout(() => { chrome.tabs.onUpdated.removeListener(onUpdated); resolve() }, 30000)
  })
}

// Debugger with delayed detach — consecutive CDP commands share one session.
// Detaches automatically after 500ms of inactivity.
let debuggerTabId = null
let detachTimer = null

async function ensureDebugger() {
  if (detachTimer) { clearTimeout(detachTimer); detachTimer = null }
  if (debuggerTabId !== activeTabId) {
    if (debuggerTabId) await chrome.debugger.detach({ tabId: debuggerTabId }).catch(() => {})
    await chrome.debugger.attach({ tabId: activeTabId }, '1.3')
    await chrome.debugger.sendCommand({ tabId: activeTabId }, 'DOM.enable', {})
    await chrome.debugger.sendCommand({ tabId: activeTabId }, 'Page.enable', {})
    debuggerTabId = activeTabId
    console.log(`[claw] debugger attached to ${activeTabId}`)
  }
  // Schedule auto-detach after 500ms idle
  detachTimer = setTimeout(async () => {
    if (debuggerTabId) {
      await chrome.debugger.detach({ tabId: debuggerTabId }).catch(() => {})
      console.log(`[claw] debugger detached (idle)`)
      debuggerTabId = null
    }
  }, 500)
}

async function withDebugger(fn) {
  await ensureDebugger()
  return await fn()
}

// --- Sync: pull .claw.js from GitHub registry ---

const REGISTRY_URL = 'https://api.github.com/repos/LeonTing1010/claw/git/trees/master?recursive=1'
const RAW_BASE = 'https://raw.githubusercontent.com/LeonTing1010/claw/master/extension-v2/claws'

async function syncClaws() {
  const resp = await fetch(REGISTRY_URL)
  const tree = await resp.json()
  const files = (tree.tree || [])
    .filter(f => f.type === 'blob' && f.path.startsWith('extension-v2/claws/') && f.path.endsWith('.claw.js'))
    .map(f => f.path.replace('extension-v2/claws/', ''))

  const stored = (await chrome.storage.local.get('synced_claws'))?.synced_claws || {}
  let synced = 0, skipped = 0

  for (const file of files) {
    const url = `${RAW_BASE}/${file}`
    try {
      const code = await (await fetch(url)).text()
      if (stored[file] === code) { skipped++; continue }
      stored[file] = code
      synced++
      console.log(`[claw] synced ${file}`)
    } catch (e) {
      console.warn(`[claw] sync failed: ${file}`, e.message)
    }
  }

  await chrome.storage.local.set({ synced_claws: stored })
  console.log(`[claw] sync done: ${synced} updated, ${skipped} unchanged`)
  return { synced, skipped, total: files.length }
}

// --- Sandbox Runner: execute dynamically synced claws ---

let sandboxFrame = null

function getSandbox() {
  if (sandboxFrame) return sandboxFrame
  sandboxFrame = document.createElement('iframe')
  sandboxFrame.src = chrome.runtime.getURL('sandbox.html')
  document.body.appendChild(sandboxFrame)
  return sandboxFrame
}

// Pending sandbox runs
const sandboxRuns = new Map()
let sandboxRunId = 0

// Handle messages from sandbox iframe
self.addEventListener('message', async (event) => {
  const msg = event.data
  if (!msg?.type) return

  if (msg.type === 'page_call') {
    // Sandbox wants to call a page.* API
    const { id, callId, method, args } = msg
    try {
      const result = await executeSandboxPageCall(method, args)
      getSandbox().contentWindow.postMessage({ type: 'page_result', callId, result }, '*')
    } catch (e) {
      getSandbox().contentWindow.postMessage({ type: 'page_result', callId, error: e.message }, '*')
    }
  }

  if (msg.type === 'result') {
    const pending = sandboxRuns.get(msg.id)
    if (pending) {
      sandboxRuns.delete(msg.id)
      if (msg.error) pending.reject(new Error(msg.error))
      else pending.resolve(msg.result)
    }
  }
})

async function executeSandboxPageCall(method, args) {
  const tabId = activeTabId
  if (!tabId && method !== 'wait') throw new Error('no active tab')

  switch (method) {
    case 'nav':
      await chrome.tabs.update(tabId, { url: args[0] })
      await waitForTabLoad(tabId)
      return null

    case 'waitFor': {
      const [selector, timeout = 10000] = args
      const start = Date.now()
      while (Date.now() - start < timeout) {
        const [r] = await chrome.scripting.executeScript({
          target: { tabId }, func: s => !!document.querySelector(s), args: [selector], world: 'MAIN'
        })
        if (r?.result) return null
        await new Promise(r => setTimeout(r, 300))
      }
      throw new Error(`waitFor: "${selector}" timeout`)
    }

    case 'eval': {
      const [fnStr, ...fnArgs] = args
      // Reconstruct function from string and execute via debugger (bypasses CSP)
      const expr = `(${fnStr})(${fnArgs.map(a => JSON.stringify(a)).join(',')})`
      return await withDebugger(async () => {
        const r = await chrome.debugger.sendCommand({ tabId }, 'Runtime.evaluate', {
          expression: expr, returnByValue: true, awaitPromise: true
        })
        if (r.exceptionDetails) throw new Error(r.exceptionDetails.text || 'eval error')
        return r.result?.value
      })
    }

    case 'fetch': {
      const [url, opts = {}] = args
      const [r] = await chrome.scripting.executeScript({
        target: { tabId },
        func: async (u, o) => { const r = await fetch(u, { credentials: 'include', ...o }); return r.json() },
        args: [url, opts], world: 'MAIN'
      })
      return r?.result
    }

    case 'click': {
      const [target] = args
      const [r] = await chrome.scripting.executeScript({
        target: { tabId },
        func: t => {
          let el = document.querySelector(t)
          if (!el) {
            const w = document.createTreeWalker(document.body, NodeFilter.SHOW_ELEMENT)
            while (w.nextNode()) {
              if (w.currentNode.textContent?.trim() === t && w.currentNode.offsetParent !== null) { el = w.currentNode; break }
            }
          }
          if (!el) return null
          const rect = el.getBoundingClientRect()
          return { x: rect.x + rect.width / 2, y: rect.y + rect.height / 2 }
        },
        args: [target], world: 'MAIN'
      })
      if (!r?.result) throw new Error(`click: "${target}" not found`)
      await withDebugger(async () => {
        const p = { x: r.result.x, y: r.result.y, button: 'left', clickCount: 1 }
        await chrome.debugger.sendCommand({ tabId }, 'Input.dispatchMouseEvent', { type: 'mousePressed', ...p })
        await chrome.debugger.sendCommand({ tabId }, 'Input.dispatchMouseEvent', { type: 'mouseReleased', ...p })
      })
      return null
    }

    case 'type': {
      const [selector, text] = args
      await chrome.scripting.executeScript({
        target: { tabId }, func: s => { const el = document.querySelector(s); if (el) { el.focus(); el.click() } },
        args: [selector], world: 'MAIN'
      })
      await new Promise(r => setTimeout(r, 100))
      await withDebugger(async () => {
        for (const c of text) {
          await chrome.debugger.sendCommand({ tabId }, 'Input.dispatchKeyEvent', { type: 'keyDown', text: c, key: c })
          await chrome.debugger.sendCommand({ tabId }, 'Input.dispatchKeyEvent', { type: 'keyUp', key: c })
        }
      })
      return null
    }

    case 'screenshot':
      return await chrome.tabs.captureVisibleTab(null, { format: 'png' })

    case 'cookies': {
      const tab = await chrome.tabs.get(tabId)
      return await chrome.cookies.getAll({ url: tab.url })
    }

    case 'claw': {
      const [site, name, clawArgs = {}] = args
      const result = await handleClawAction({ action: 'run', site, name, args: clawArgs })
      return result?.rows || result
    }

    default:
      throw new Error(`sandbox: unknown page method "${method}"`)
  }
}

async function runSyncedClaw(site, name, args, tabId) {
  const key = `${site}/${name}.claw.js`
  const stored = (await chrome.storage.local.get('synced_claws'))?.synced_claws || {}
  const code = stored[key]
  if (!code) throw new Error(`claw not found: ${site}/${name} (not bundled or synced)`)

  activeTabId = tabId
  const id = ++sandboxRunId
  const sandbox = getSandbox()

  return new Promise((resolve, reject) => {
    sandboxRuns.set(id, { resolve, reject })
    // Wait for sandbox iframe to load, then send
    const send = () => sandbox.contentWindow.postMessage({ type: 'run', id, code, args }, '*')
    if (sandbox.contentDocument?.readyState === 'complete') send()
    else sandbox.addEventListener('load', send, { once: true })

    // Timeout
    setTimeout(() => {
      if (sandboxRuns.has(id)) {
        sandboxRuns.delete(id)
        reject(new Error('sandbox execution timeout (60s)'))
      }
    }, 60000)
  })
}

// --- Omnibox: claw:// protocol via address bar ---

chrome.omnibox.onInputSuggestion = undefined // suppress default

chrome.omnibox.onInputChanged.addListener((text, suggest) => {
  const claws = listClaws()
  const matches = text.trim()
    ? claws.filter(c => `${c.site}/${c.name}`.includes(text.trim()))
    : claws

  suggest(matches.slice(0, 8).map(c => ({
    content: `${c.site}/${c.name}`,
    description: `<match>${c.site}/${c.name}</match> — ${c.description || 'no description'}`
  })))
})

chrome.omnibox.onInputEntered.addListener((text, disposition) => {
  // text is "site/name?args" — open results page
  const resultsUrl = chrome.runtime.getURL(`results.html#${text.trim()}`)

  if (disposition === 'currentTab') {
    chrome.tabs.update({ url: resultsUrl })
  } else {
    chrome.tabs.create({ url: resultsUrl })
  }
})

// --- showResults action (from content script) ---

// Handle showResults in the message router
const originalHandleMessage = handleMessage
// Extend handleClawAction to support showResults
const _origClawAction = handleClawAction
async function handleShowResults(msg) {
  if (msg.action === 'showResults') {
    const hash = msg.url.replace('claw://', '')
    const resultsUrl = chrome.runtime.getURL(`results.html#${hash}`)
    chrome.tabs.create({ url: resultsUrl })
    return { ok: true }
  }
  return null
}

// Patch message listeners to handle showResults
chrome.runtime.onMessage.addListener((msg, sender, sendResponse) => {
  if (msg.action === 'showResults') {
    handleShowResults(msg).then(sendResponse)
    return true
  }
})

// --- Start ---

connectBridge()

chrome.alarms.create('keepalive', { periodInMinutes: 0.4 })
chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === 'keepalive' && (!ws || ws.readyState !== WebSocket.OPEN)) {
    connectBridge()
  }
})

console.log('[claw] v2 ready — claw:// protocol active')
