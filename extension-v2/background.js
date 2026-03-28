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

import { registerClaw, listClaws, runClaw, parseClawURL } from './runtime/executor.js'
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

    default:
      throw new Error(`Unknown Claw command: ${method}`)
  }
}

// --- Claw Action Handler ---

async function handleClawAction(msg) {
  switch (msg.action) {
    case 'list':
      return { claws: listClaws() }

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

      return await runClaw(site, name, args, tabId)
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
