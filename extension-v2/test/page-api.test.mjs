/**
 * Constraint: page API contract
 * Classification: safety / what — missing method = claw runtime crash
 *
 * Why: page API is the only interface between .claw.js and the browser.
 * If a method is missing or misnamed, claws fail silently.
 *
 * Run: node extension-v2/test/page-api.test.mjs
 */

import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'

const REQUIRED_METHODS = ['nav', 'wait', 'waitFor', 'click', 'type', 'upload', 'eval', 'fetch', 'screenshot', 'cookies', 'claw']

let passed = 0
let failed = 0

function test(name, fn) {
  try {
    fn()
    passed++
    console.log(`  \x1b[32m✓\x1b[0m ${name}`)
  } catch (e) {
    failed++
    console.log(`  \x1b[31m✗\x1b[0m ${name}`)
    console.log(`    ${e.message}`)
  }
}

console.log('\npage API constraints\n')

// Read page-api.js source (can't import because it uses chrome.* which doesn't exist in Node)
const src = readFileSync(new URL('../runtime/page-api.js', import.meta.url), 'utf-8')

test('page-api.js exists and is non-empty', () => {
  assert(src.length > 0)
})

test('exports createPageAPI function', () => {
  assert(src.includes('export function createPageAPI'))
})

// Check that all required methods are defined in the page object
for (const method of REQUIRED_METHODS) {
  test(`page.${method} is defined`, () => {
    // Match patterns like: async nav(, async wait(, nav:, wait:, etc.
    const patterns = [
      `async ${method}(`,     // async method(
      `${method}(`,           // method(
      `${method}:`,           // property shorthand
    ]
    const found = patterns.some(p => src.includes(p))
    assert(found, `page.${method} not found in page-api.js`)
  })
}

// Constraint: scripting-mode methods must NOT use chrome.debugger
const SCRIPTING_METHODS = ['nav', 'wait', 'waitFor', 'eval', 'fetch', 'screenshot', 'cookies']

test('createPageAPI returns an object (structural check)', () => {
  assert(src.includes('const page = {') || src.includes('const page={'))
  assert(src.includes('return page'))
})

// Constraint: withDebugger helper exists for ms-level attach/detach
test('withDebugger helper exists for attach/detach pattern', () => {
  assert(src.includes('withDebugger'), 'must have withDebugger helper for ms-level debugger usage')
  assert(src.includes('debugger.attach'), 'withDebugger must attach')
  assert(src.includes('debugger.detach'), 'withDebugger must detach')
})

// Constraint: no method count drift
test(`exactly ${REQUIRED_METHODS.length} methods in page API`, () => {
  // Count 'async' method definitions inside the page object
  const methodDefs = src.match(/async \w+\(/g) || []
  // Filter to only those in the page object (rough heuristic: between 'const page = {' and 'return page')
  const pageSection = src.substring(src.indexOf('const page = {'), src.indexOf('return page'))
  const pageMethods = pageSection.match(/async \w+\(/g) || []
  assert.equal(pageMethods.length, REQUIRED_METHODS.length,
    `expected ${REQUIRED_METHODS.length} page methods, found ${pageMethods.length}: ${pageMethods.join(', ')}`)
})

console.log(`\n${passed + failed} constraints, ${passed} passed, ${failed} failed\n`)
process.exit(failed > 0 ? 1 : 0)
