/**
 * Claw Content Script — injects claw:// protocol support into every page.
 *
 * 1. window.claw("site/name", {args}) — programmatic API for any webpage/script
 * 2. <a href="claw://site/name?args"> — clickable claw links
 */

// --- 1. window.claw() API ---

const script = document.createElement('script')
script.textContent = `
(function() {
  const EXTENSION_ID = '${chrome.runtime.id}';

  /**
   * Run a claw and return structured data.
   * @param {string} path - "site/name" (e.g. "github/trending")
   * @param {object} args - Arguments (e.g. {limit: 5})
   * @returns {Promise<{columns: string[], rows: object[], count: number}>}
   *
   * Usage:
   *   const result = await claw("github/trending", {limit: 5})
   *   console.table(result.rows)
   */
  window.claw = function(path, args = {}) {
    return new Promise((resolve, reject) => {
      const [site, name] = path.split('/')
      if (!site || !name) {
        reject(new Error('claw: usage: claw("site/name", {args})'))
        return
      }
      chrome.runtime.sendMessage(EXTENSION_ID, {
        action: 'run', site, name, args
      }, response => {
        if (chrome.runtime.lastError) {
          reject(new Error('claw: extension not available — ' + chrome.runtime.lastError.message))
        } else if (response?.error) {
          reject(new Error('claw: ' + response.error))
        } else {
          resolve(response)
        }
      })
    })
  }

  /**
   * List all available claws.
   * @returns {Promise<Array<{site, name, description, columns}>>}
   */
  window.claw.list = function() {
    return new Promise((resolve, reject) => {
      chrome.runtime.sendMessage(EXTENSION_ID, {
        action: 'list'
      }, response => {
        if (chrome.runtime.lastError) {
          reject(new Error('claw: ' + chrome.runtime.lastError.message))
        } else {
          resolve(response?.claws || response)
        }
      })
    })
  }

  // Mark protocol as available
  window.claw.version = '2.0.0'

  console.log('[claw] protocol ready — try: claw("github/trending", {limit: 5})')
})()
`
document.documentElement.appendChild(script)
script.remove()

// --- 2. Intercept claw:// link clicks ---

document.addEventListener('click', (e) => {
  const link = e.target.closest('a[href^="claw://"]')
  if (!link) return

  e.preventDefault()
  const url = link.getAttribute('href')

  chrome.runtime.sendMessage({ action: 'run', url }, (response) => {
    if (response?.error) {
      console.error('[claw]', response.error)
    } else {
      // Open results in extension page
      chrome.runtime.sendMessage({
        action: 'showResults',
        url,
        data: response
      })
    }
  })
}, true)
