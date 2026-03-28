/**
 * Page Intelligence — one-shot page analysis for claw forging.
 *
 * Replaces 5-8 MCP round-trips (screenshot + ax_tree + global_names + api_log + page_info)
 * with a single call that returns everything an agent needs to decide strategy and write a claw.
 */

/**
 * Gather complete page intelligence in one call.
 * @param {number} tabId - Chrome tab ID
 * @returns {object} Full intelligence report
 */
export async function gatherPageIntelligence(tabId) {
  const tab = await chrome.tabs.get(tabId)

  // Single executeScript call gathers everything from the page context
  const [analysisResult] = await chrome.scripting.executeScript({
    target: { tabId },
    func: analyzePageContext,
    world: 'MAIN'
  })
  const analysis = analysisResult?.result || {}

  // Screenshot (extension API, not page context)
  let screenshot = null
  try {
    screenshot = await chrome.tabs.captureVisibleTab(null, { format: 'png' })
  } catch { /* tab not visible */ }

  // Cookies for auth detection (extension API)
  let cookies = []
  try {
    cookies = await chrome.cookies.getAll({ url: tab.url })
  } catch {}

  const auth = {
    cookies_count: cookies.length,
    has_session: cookies.some(c =>
      /sess|token|auth|login|user|sid/i.test(c.name)
    ),
    session_cookies: cookies
      .filter(c => /sess|token|auth|login|user|sid/i.test(c.name))
      .map(c => c.name)
      .slice(0, 10),
    storage_keys: analysis.storage_keys || []
  }

  const strategies = recommendStrategies(analysis, tab.url)

  return {
    url: tab.url,
    title: tab.title,
    framework: analysis.framework,
    ssr_state: analysis.ssr_state,
    api_hints: analysis.api_hints,
    interactive: analysis.interactive,
    auth,
    meta: analysis.meta,
    strategies,
    screenshot
  }
}

// ---------------------------------------------------------------------------
// Page-context analysis (runs inside the page via chrome.scripting)
// Must be fully self-contained — no closures, no imports, no extension APIs.
// ---------------------------------------------------------------------------

function analyzePageContext() {
  const result = {
    framework: null,
    ssr_state: {},
    api_hints: [],
    interactive: { forms: [], inputs: [], buttons: [], links_count: 0 },
    storage_keys: [],
    meta: {}
  }

  // --- Framework Detection ---

  if (window.__NEXT_DATA__) {
    result.framework = { name: 'next', evidence: '__NEXT_DATA__' }
  } else if (window.__NUXT__ || window.__NUXT_DATA__) {
    result.framework = { name: 'nuxt', evidence: window.__NUXT__ ? '__NUXT__' : '__NUXT_DATA__' }
  } else if (window.__vue_app__) {
    result.framework = { name: 'vue3', evidence: '__vue_app__' }
  } else if (document.querySelector('[data-v-]')) {
    result.framework = { name: 'vue2', evidence: 'data-v- attributes' }
  } else if (document.querySelector('[data-reactroot]') || document.querySelector('#__next')) {
    result.framework = { name: 'react', evidence: 'data-reactroot or #__next' }
  } else if (window.angular || document.querySelector('[ng-app]') || document.querySelector('[ng-version]')) {
    result.framework = { name: 'angular', evidence: 'angular globals or ng- attributes' }
  } else if (document.querySelector('[class*="svelte-"]')) {
    result.framework = { name: 'svelte', evidence: 'svelte- class prefixes' }
  } else {
    result.framework = { name: 'unknown', evidence: 'no framework markers detected' }
  }

  // --- SSR State Extraction ---

  const SSR_GLOBALS = [
    '__NEXT_DATA__',
    '__NUXT__',
    '__NUXT_DATA__',
    '__INITIAL_STATE__',
    '__INITIAL_SSR_STATE__',
    '__pinia',
    '__PRELOADED_STATE__',
    '__APP_DATA__',
    '__SSR_DATA__',
    '__APOLLO_STATE__',
    '__RELAY_STORE__',
  ]

  for (const key of SSR_GLOBALS) {
    const val = window[key]
    if (val == null) continue
    try {
      const serialized = JSON.stringify(val)
      const topKeys = typeof val === 'object' && val !== null
        ? Object.keys(val).slice(0, 30)
        : []
      result.ssr_state[key] = {
        size: serialized.length,
        keys: topKeys,
        sample: serialized.substring(0, 3000)
      }
    } catch {
      result.ssr_state[key] = { size: -1, keys: [], sample: '[unserializable]' }
    }
  }

  // Also scan window for any obvious state objects we missed
  try {
    for (const key of Object.getOwnPropertyNames(window)) {
      if (SSR_GLOBALS.includes(key)) continue
      if (!/^__[A-Z]/.test(key)) continue // only __UPPER_CASE patterns
      try {
        const val = window[key]
        if (val && typeof val === 'object' && !Array.isArray(val)) {
          const s = JSON.stringify(val)
          if (s.length > 500) { // only interesting if substantial
            result.ssr_state[key] = {
              size: s.length,
              keys: Object.keys(val).slice(0, 20),
              sample: s.substring(0, 1500)
            }
          }
        }
      } catch {}
    }
  } catch {}

  // --- API Hints from Performance Entries ---

  try {
    const entries = performance.getEntriesByType('resource')
    const apiEntries = entries.filter(e =>
      e.initiatorType === 'fetch' || e.initiatorType === 'xmlhttprequest'
    )
    result.api_hints = apiEntries.slice(0, 50).map(e => {
      let pathname = ''
      try { pathname = new URL(e.name).pathname } catch {}
      return {
        url: e.name,
        pathname,
        type: e.initiatorType,
        duration_ms: Math.round(e.duration),
        size: e.transferSize || e.decodedBodySize || 0
      }
    })
  } catch {}

  // --- Interactive Elements ---

  // Forms
  try {
    const forms = document.querySelectorAll('form')
    result.interactive.forms = Array.from(forms).slice(0, 10).map(f => ({
      action: f.action || '',
      method: (f.method || 'GET').toUpperCase(),
      id: f.id || null,
      inputs: Array.from(f.querySelectorAll('input,textarea,select')).slice(0, 15).map(i => ({
        tag: i.tagName.toLowerCase(),
        type: i.type || '',
        name: i.name || '',
        placeholder: i.placeholder || '',
        required: i.required || false
      }))
    }))
  } catch {}

  // Standalone inputs (not inside forms)
  try {
    const inputs = document.querySelectorAll(
      'input:not(form input), textarea:not(form textarea), [contenteditable="true"], [role="textbox"]'
    )
    result.interactive.inputs = Array.from(inputs).slice(0, 20).map(i => ({
      tag: i.tagName.toLowerCase(),
      type: i.type || i.getAttribute('role') || '',
      name: i.name || '',
      placeholder: i.placeholder || i.getAttribute('aria-label') || '',
      selector: quickSelector(i)
    }))
  } catch {}

  // Buttons
  try {
    const buttons = document.querySelectorAll(
      'button, [role="button"], input[type="submit"], a.btn, a.button'
    )
    result.interactive.buttons = Array.from(buttons)
      .filter(b => b.offsetParent !== null) // visible only
      .slice(0, 30)
      .map(b => ({
        text: (b.textContent || '').trim().substring(0, 60),
        type: b.type || b.tagName.toLowerCase(),
        selector: quickSelector(b)
      }))
  } catch {}

  // Link count
  try {
    result.interactive.links_count = document.querySelectorAll('a[href]').length
  } catch {}

  // --- Storage Keys ---

  try {
    result.storage_keys = Object.keys(localStorage).slice(0, 30)
  } catch {}

  // --- Page Meta ---

  result.meta = {
    charset: document.characterSet || '',
    lang: document.documentElement.lang || '',
    description: (document.querySelector('meta[name="description"]') || {}).content || '',
    og_title: (document.querySelector('meta[property="og:title"]') || {}).content || '',
    canonical: (document.querySelector('link[rel="canonical"]') || {}).href || '',
    ready_state: document.readyState,
    scroll_height: document.documentElement.scrollHeight,
    viewport_height: window.innerHeight,
  }

  return result

  // --- Helper ---

  function quickSelector(el) {
    if (el.id) return '#' + el.id
    const testId = el.getAttribute('data-testid') || el.getAttribute('data-test-id')
    if (testId) return `[data-testid="${testId}"]`
    if (el.name) return `${el.tagName.toLowerCase()}[name="${el.name}"]`
    const cls = Array.from(el.classList || [])
      .filter(c => !/^(svelte-|css-|_|sc-)/.test(c))
      .slice(0, 2)
    if (cls.length) return `${el.tagName.toLowerCase()}.${cls.join('.')}`
    return el.tagName.toLowerCase()
  }
}

// ---------------------------------------------------------------------------
// Strategy recommendation (runs in extension context)
// ---------------------------------------------------------------------------

function recommendStrategies(analysis, url) {
  const strategies = []
  let hostname = ''
  try { hostname = new URL(url).hostname.replace(/^www\./, '') } catch {}
  const site = hostname.split('.')[0] || 'example'

  // 1. SSR state — best: zero network requests
  const ssrKeys = Object.keys(analysis.ssr_state || {})
  if (ssrKeys.length > 0) {
    const totalSize = ssrKeys.reduce((s, k) => s + (analysis.ssr_state[k].size || 0), 0)
    const primaryGlobal = ssrKeys[0]
    strategies.push({
      rank: 1,
      type: 'ssr',
      reason: `SSR state found: ${ssrKeys.join(', ')} (${(totalSize / 1024).toFixed(0)}KB). Extract from window globals — zero requests, instant.`,
      globals: ssrKeys,
      template: `export default {
  site: "${site}",
  name: "TODO_name",
  description: "TODO",
  columns: [/* TODO: fill column names */],
  args: { limit: { type: "int", default: 20 } },

  async run(page, args) {
    await page.nav("${url}")
    await page.wait(1000)

    const data = await page.eval(() => {
      const state = window.${primaryGlobal}
      // TODO: navigate state to find your array
      // Keys: ${(analysis.ssr_state[primaryGlobal]?.keys || []).slice(0, 10).join(', ')}
      return Array.isArray(state) ? state : Object.values(state)
    })

    return data.slice(0, args.limit).map(item => ({
      // TODO: map item fields to columns
    }))
  }
}`
    })
  }

  // 2. API endpoints — good: one fetch, reliable
  const apiHints = analysis.api_hints || []
  const jsonAPIs = apiHints.filter(h =>
    /\/(api|graphql|v[0-9]|ajax|rpc|data|feed)\b/i.test(h.pathname) ||
    /\.json(\?|$)/.test(h.url)
  )
  if (jsonAPIs.length > 0) {
    const bestAPI = jsonAPIs.sort((a, b) => b.size - a.size)[0]
    strategies.push({
      rank: 2,
      type: 'api',
      reason: `Found ${jsonAPIs.length} API endpoint(s). Replay with page cookies for reliable structured data.`,
      endpoints: jsonAPIs.slice(0, 8).map(a => ({
        url: a.url,
        pathname: a.pathname,
        size: a.size
      })),
      template: `export default {
  site: "${site}",
  name: "TODO_name",
  description: "TODO",
  columns: [/* TODO: fill column names */],
  args: { limit: { type: "int", default: 20 } },

  async run(page, args) {
    await page.nav("${url}")

    const data = await page.fetch("${bestAPI.url}")
    // If data is nested: const items = data.data.list || data.items || data

    return data.slice(0, args.limit).map(item => ({
      // TODO: map item fields to columns
    }))
  }
}`
    })
  }

  // 3. DOM extraction — fallback
  strategies.push({
    rank: strategies.length + 1,
    type: 'dom',
    reason: 'Extract from DOM. Less reliable but always available.',
    template: `export default {
  site: "${site}",
  name: "TODO_name",
  description: "TODO",
  columns: [/* TODO: fill column names */],
  args: { limit: { type: "int", default: 20 } },

  async run(page, args) {
    await page.nav("${url}")
    await page.waitFor("TODO_selector", 10000)
    await page.wait(2000)

    const items = await page.eval(() => {
      return Array.from(document.querySelectorAll("TODO_selector"))
        .map(el => ({
          // TODO: extract from each element
        }))
        .filter(item => /* TODO: filter empty */ true)
    })

    return items.slice(0, args.limit)
  }
}`
  })

  return strategies
}
