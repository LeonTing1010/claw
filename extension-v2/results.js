/**
 * Results page — renders claw output as a table.
 * Data arrives via URL hash: #site/name or via chrome.storage.local
 */

async function render() {
  const hash = location.hash.slice(1) // e.g. "github/trending?limit=5"
  const titleEl = document.getElementById('title')
  const metaEl = document.getElementById('meta')
  const contentEl = document.getElementById('content')

  if (!hash) {
    contentEl.innerHTML = '<div class="empty">No claw specified. Usage: results.html#site/name?args</div>'
    return
  }

  // Parse claw URL from hash
  const [path, queryString] = hash.split('?')
  const [site, name] = path.split('/')
  const args = {}
  if (queryString) {
    for (const pair of queryString.split('&')) {
      const [k, v] = pair.split('=')
      args[decodeURIComponent(k)] = decodeURIComponent(v || '')
    }
  }

  titleEl.textContent = `claw://${site}/${name}`
  metaEl.textContent = 'Running...'

  try {
    const start = Date.now()
    const response = await chrome.runtime.sendMessage({
      action: 'run', site, name, args
    })

    if (response?.error) {
      contentEl.innerHTML = `<div class="error">${response.error}</div>`
      metaEl.textContent = 'Error'
      return
    }

    const elapsed = Date.now() - start
    const { columns, rows, count } = response
    metaEl.textContent = `${count} rows · ${elapsed}ms`

    if (!rows || rows.length === 0) {
      contentEl.innerHTML = '<div class="empty">No results</div>'
      return
    }

    // Build table
    const cols = columns || Object.keys(rows[0])
    let html = '<table><thead><tr>'
    for (const col of cols) {
      html += `<th>${esc(col)}</th>`
    }
    html += '</tr></thead><tbody>'

    for (const row of rows) {
      html += '<tr>'
      for (const col of cols) {
        const val = row[col] ?? ''
        html += `<td title="${esc(String(val))}">${esc(String(val))}</td>`
      }
      html += '</tr>'
    }
    html += '</tbody></table>'
    contentEl.innerHTML = html

  } catch (e) {
    contentEl.innerHTML = `<div class="error">${e.message}</div>`
    metaEl.textContent = 'Error'
  }
}

function esc(s) {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
}

render()
