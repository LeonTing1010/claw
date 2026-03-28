/**
 * Claw Sandbox — executes dynamically synced .claw.js code.
 *
 * Runs in a sandboxed iframe (MV3 sandbox page). Can use eval().
 * Communicates with background.js via postMessage for page.* API calls.
 *
 * Protocol:
 *   background → sandbox: { type: "run", id, code, args }
 *   sandbox → background: { type: "page_call", id, callId, method, args }
 *   background → sandbox: { type: "page_result", callId, result?, error? }
 *   sandbox → background: { type: "result", id, result?, error? }
 */

let callIdCounter = 0
const pendingCalls = new Map()

window.addEventListener('message', async (event) => {
  const msg = event.data
  if (!msg || !msg.type) return

  switch (msg.type) {
    case 'run':
      await handleRun(msg)
      break

    case 'page_result':
      // Response from background for a page.* API call
      const pending = pendingCalls.get(msg.callId)
      if (pending) {
        pendingCalls.delete(msg.callId)
        if (msg.error) {
          pending.reject(new Error(msg.error))
        } else {
          pending.resolve(msg.result)
        }
      }
      break
  }
})

async function handleRun({ id, code, args }) {
  try {
    // Parse the .claw.js module — eval in sandbox (allowed by MV3 sandbox CSP)
    const mod = eval(`(function() {
      const exports = {};
      const module = { exports };
      ${code.replace(/export\s+default\s+/, 'module.exports = ')}
      return module.exports;
    })()`)

    // Create page API proxy — each call sends postMessage to background
    const page = createPageProxy(id)

    // Execute
    const rows = await mod.run(page, args || {})

    // Return result
    parent.postMessage({ type: 'result', id, result: {
      columns: mod.columns,
      rows: Array.isArray(rows) ? rows : [],
      count: Array.isArray(rows) ? rows.length : 0,
      site: mod.site,
      name: mod.name
    }}, '*')

  } catch (e) {
    parent.postMessage({ type: 'result', id, error: e.message }, '*')
  }
}

function createPageProxy(runId) {
  function callPage(method, ...args) {
    return new Promise((resolve, reject) => {
      const callId = ++callIdCounter
      pendingCalls.set(callId, { resolve, reject })
      parent.postMessage({
        type: 'page_call',
        id: runId,
        callId,
        method,
        args
      }, '*')
    })
  }

  return {
    nav:        (url)              => callPage('nav', url),
    wait:       (ms)               => new Promise(r => setTimeout(r, ms)),
    waitFor:    (sel, timeout)     => callPage('waitFor', sel, timeout),
    click:      (target)           => callPage('click', target),
    type:       (sel, text)        => callPage('type', sel, text),
    upload:     (sel, files)       => callPage('upload', sel, files),
    eval:       (fn, ...args)      => callPage('eval', fn.toString(), ...args),
    fetch:      (url, opts)        => callPage('fetch', url, opts),
    screenshot: ()                 => callPage('screenshot'),
    cookies:    ()                 => callPage('cookies'),
    claw:       (site, name, args) => callPage('claw', site, name, args),
  }
}
