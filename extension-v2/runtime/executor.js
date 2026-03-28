/**
 * Claw executor — loads .claw.js files and runs them.
 *
 * The executor's only job:
 *   1. Load a .claw.js module
 *   2. Create a page API for the active tab
 *   3. Call run(page, args) and return structured data
 */

import { createPageAPI } from './page-api.js'

/** Registry of loaded claw modules, keyed by "site/name" */
const clawRegistry = new Map()

/**
 * Register a claw module in the registry.
 * Called during extension startup to load all bundled claws.
 */
export function registerClaw(mod) {
  const key = `${mod.site}/${mod.name}`
  clawRegistry.set(key, mod)
}

/** List all registered claws. */
export function listClaws() {
  return Array.from(clawRegistry.values()).map(({ site, name, description, columns, args }) => ({
    site, name, description: description || '', columns, args: args || {}
  }))
}

/** Get a claw module by site/name. */
export function getClaw(site, name) {
  return clawRegistry.get(`${site}/${name}`)
}

/**
 * Execute a claw.
 * @param {string} site - Site identifier
 * @param {string} name - Claw name
 * @param {object} userArgs - User-provided arguments
 * @param {number} tabId - Chrome tab ID to operate on
 * @returns {{ columns: string[], rows: object[] }}
 */
export async function runClaw(site, name, userArgs = {}, tabId) {
  const mod = getClaw(site, name)
  if (!mod) throw new Error(`claw not found: ${site}/${name}`)

  // Resolve args with defaults
  const args = {}
  if (mod.args) {
    for (const [key, spec] of Object.entries(mod.args)) {
      args[key] = userArgs[key] !== undefined ? coerceArg(userArgs[key], spec.type) : spec.default
    }
  }
  // Pass through any extra args
  for (const [key, val] of Object.entries(userArgs)) {
    if (!(key in args)) args[key] = val
  }

  // Create page API for this tab
  const page = createPageAPI(tabId)

  // Wire up page.claw() for composition
  page.claw = async (s, n, a = {}) => {
    const result = await runClaw(s, n, a, tabId)
    return result.rows
  }

  // Execute
  const rows = await mod.run(page, args)

  // Validate output
  if (!Array.isArray(rows)) {
    throw new Error(`claw ${site}/${name} run() must return an array, got ${typeof rows}`)
  }

  return { columns: mod.columns, rows, count: rows.length }
}

/** Coerce argument to declared type. */
function coerceArg(value, type) {
  switch (type) {
    case 'int': return parseInt(value, 10)
    case 'float': return parseFloat(value)
    case 'boolean': return value === true || value === 'true'
    case 'string': return String(value)
    default: return value
  }
}

/**
 * Parse a claw:// URL into { site, name, args }.
 * Format: claw://site/name?arg1=val1&arg2=val2
 */
export function parseClawURL(url) {
  const match = url.match(/^claw:\/\/([^/]+)\/([^?]+)(?:\?(.*))?$/)
  if (!match) throw new Error(`invalid claw URL: ${url}`)

  const [, site, name, queryString] = match
  const args = {}
  if (queryString) {
    for (const pair of queryString.split('&')) {
      const [k, v] = pair.split('=')
      args[decodeURIComponent(k)] = decodeURIComponent(v || '')
    }
  }

  return { site, name, args }
}
