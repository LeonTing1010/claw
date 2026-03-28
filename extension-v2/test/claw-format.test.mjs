/**
 * Constraint: .claw.js format contract
 * Classification: safety / what — invalid format = runtime crash
 *
 * Why: .claw.js is the only artifact format. If it's malformed,
 * nothing works. These constraints define the executable contract.
 *
 * Run: node extension-v2/test/claw-format.test.mjs
 */

import { strict as assert } from 'node:assert'
import { readdir } from 'node:fs/promises'
import { join, basename } from 'node:path'
import { pathToFileURL } from 'node:url'

const CLAWS_DIR = new URL('../claws/', import.meta.url).pathname
const REQUIRED_FIELDS = ['site', 'name', 'columns', 'run']
const VALID_ARG_TYPES = ['string', 'int', 'float', 'boolean']
const PAGE_API_METHODS = ['nav', 'wait', 'waitFor', 'click', 'type', 'upload', 'eval', 'fetch', 'screenshot', 'cookies', 'claw']

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

async function testAsync(name, fn) {
  try {
    await fn()
    passed++
    console.log(`  \x1b[32m✓\x1b[0m ${name}`)
  } catch (e) {
    failed++
    console.log(`  \x1b[31m✗\x1b[0m ${name}`)
    console.log(`    ${e.message}`)
  }
}

// Collect all .claw.js files
async function findClawFiles(dir) {
  const files = []
  for (const site of await readdir(dir)) {
    const sitePath = join(dir, site)
    try {
      for (const file of await readdir(sitePath)) {
        if (file.endsWith('.claw.js')) {
          files.push({ site, name: basename(file, '.claw.js'), path: join(sitePath, file) })
        }
      }
    } catch { /* not a directory */ }
  }
  return files
}

// --- Constraints ---

console.log('\n.claw.js format constraints\n')

const clawFiles = await findClawFiles(CLAWS_DIR)

test('at least one .claw.js file exists', () => {
  assert(clawFiles.length > 0, `no .claw.js files found in ${CLAWS_DIR}`)
})

for (const { site, name, path } of clawFiles) {
  console.log(`\n  ${site}/${name}.claw.js`)

  let mod
  await testAsync(`  loads as ES module`, async () => {
    mod = await import(pathToFileURL(path))
    assert(mod.default, 'must have default export')
  })

  if (!mod?.default) continue
  const claw = mod.default

  // Required fields
  test(`  has required fields: ${REQUIRED_FIELDS.join(', ')}`, () => {
    for (const field of REQUIRED_FIELDS) {
      assert(claw[field] !== undefined, `missing required field: ${field}`)
    }
  })

  // site/name match directory
  test(`  site matches directory (${claw.site} === ${site})`, () => {
    assert.equal(claw.site, site)
  })

  test(`  name matches filename (${claw.name} === ${name})`, () => {
    assert.equal(claw.name, name)
  })

  // columns is non-empty string array
  test(`  columns is non-empty string array`, () => {
    assert(Array.isArray(claw.columns), 'columns must be an array')
    assert(claw.columns.length > 0, 'columns must not be empty')
    for (const col of claw.columns) {
      assert.equal(typeof col, 'string', `column must be string, got ${typeof col}`)
    }
  })

  // run is async function with 2 params
  test(`  run is a function`, () => {
    assert.equal(typeof claw.run, 'function', 'run must be a function')
  })

  // description is optional but must be string if present
  if (claw.description !== undefined) {
    test(`  description is string`, () => {
      assert.equal(typeof claw.description, 'string')
    })
  }

  // args validation
  if (claw.args) {
    test(`  args have valid types`, () => {
      for (const [key, spec] of Object.entries(claw.args)) {
        assert(spec.type, `arg '${key}' missing type`)
        assert(VALID_ARG_TYPES.includes(spec.type), `arg '${key}' has invalid type '${spec.type}'`)
      }
    })
  }

  // health validation
  if (claw.health) {
    test(`  health contract is valid`, () => {
      if (claw.health.min_rows !== undefined) {
        assert.equal(typeof claw.health.min_rows, 'number')
        assert(claw.health.min_rows > 0, 'min_rows must be > 0')
      }
      if (claw.health.non_empty !== undefined) {
        assert(Array.isArray(claw.health.non_empty))
        for (const field of claw.health.non_empty) {
          assert(claw.columns.includes(field), `health.non_empty field '${field}' not in columns`)
        }
      }
    })
  }

  // run() must not use forbidden globals (basic static check)
  test(`  run() body does not reference chrome.* directly`, () => {
    const src = claw.run.toString()
    assert(!src.includes('chrome.tabs'), 'run() must not reference chrome.tabs directly — use page API')
    assert(!src.includes('chrome.scripting'), 'run() must not reference chrome.scripting directly — use page API')
    assert(!src.includes('chrome.debugger'), 'run() must not reference chrome.debugger directly — use page API')
  })
}

// --- Summary ---
console.log(`\n${passed + failed} constraints, ${passed} passed, ${failed} failed\n`)
process.exit(failed > 0 ? 1 : 0)
