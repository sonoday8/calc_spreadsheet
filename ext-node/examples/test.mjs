/**
 * Quick demo + assertions (mirrors ext-php/examples/test.php).
 *
 *   npm run example
 *   # or: npm run build && node examples/test.mjs
 */

import { calcSpreadsheet } from '../index.js'
import { inspect } from 'node:util'

function show(label, value) {
  process.stdout.write(`${label} = ${inspect(value, { depth: 1 })}\n`)
}

console.log('=== placeholder replace ===')
const replaced = calcSpreadsheet(
  {
    A1: '__NAME__',
    B1: 'NAME',
    C1: '__UNKNOWN__',
    D1: '=__RATE__*2',
    E1: '=F1+1',
    F1: '=__A1__',
  },
  {
    __NAME__: 'Alice',
    __RATE__: 10,
    __A1__: 7,
  },
)
for (const cell of ['A1', 'B1', 'C1', 'D1', 'E1', 'F1']) {
  show(cell, replaced[cell] ?? null)
}

const unknown = calcSpreadsheet({ Z1: '=__UNKNOWN__*2', Z2: '=__UNKNOWN__+1' })
show('unknown in formula Z1', unknown.Z1 ?? null) // 0 (missing name like =Z99)
show('unknown in formula Z2', unknown.Z2 ?? null) // 1

console.log('=== invalid replacement keys warn ===')
const warnings = []
const originalWarn = console.warn
console.warn = (...args) => {
  warnings.push(args.map(String).join(' '))
}
const warned = calcSpreadsheet(
  { A1: '=__OK__+1' },
  {
    __OK__: 3,
    bad: 9,
    __name__: 'x',
  },
)
console.warn = originalWarn
show('A1', warned.A1 ?? null) // 4
console.log(`warnings: ${warnings.length}`)
for (const w of warnings) {
  console.log(w)
}
if (warnings.length !== 1) {
  console.error('expected 1 console.warn')
  process.exit(1)
}
if (
  !warnings[0].includes('bad') ||
  !warnings[0].includes('__name__') ||
  !warnings[0].includes('ignored invalid replacement key')
) {
  console.error('warning text missing expected keys')
  process.exit(1)
}
if (warned.A1 !== 4) {
  console.error('expected A1 == 4 after ignoring invalid keys')
  process.exit(1)
}
console.log()

console.log('=== mixed sheet bench ===')

const LEAF_COUNT = 256
const LIGHT_WIDTH = 256
const LIGHT_REFS = 3
const LIGHT_DEPTH = 16
const HEAVY_WIDTH = 4096
const HEAVY_REFS = 80

const buildStart = performance.now()
const cells = {}

for (let i = 0; i < LEAF_COUNT; i++) {
  cells[`L0_${i}`] = String(i % 97)
}

for (let depth = 1; depth <= LIGHT_DEPTH; depth++) {
  const prev = depth - 1
  const prevCount = prev === 0 ? LEAF_COUNT : LIGHT_WIDTH
  for (let i = 0; i < LIGHT_WIDTH; i++) {
    const args = []
    for (let j = 0; j < LIGHT_REFS; j++) {
      args.push(`L${prev}_${(i + j) % prevCount}`)
    }
    cells[`L${depth}_${i}`] = `=SUM(${args.join(', ')})`
  }
}

const heavyDepth = LIGHT_DEPTH + 1
for (let i = 0; i < HEAVY_WIDTH; i++) {
  const args = []
  for (let j = 0; j < HEAVY_REFS; j++) {
    args.push(`L${LIGHT_DEPTH}_${(i + j) % LIGHT_WIDTH}`)
  }
  cells[`L${heavyDepth}_${i}`] = `=SUM(${args.join(', ')})`
}

const buildMs = performance.now() - buildStart

const calcStart = performance.now()
const result = calcSpreadsheet(cells)
const calcMs = performance.now() - calcStart

const sampleKey = `L${heavyDepth}_0`

console.log(`cells in:  ${Object.keys(cells).length}`)
console.log(`cells out: ${Object.keys(result).length}`)
console.log(`build:     ${buildMs.toFixed(3)} ms`)
console.log(`calc:      ${calcMs.toFixed(3)} ms`)
show(`sample ${sampleKey}`, result[sampleKey] ?? null)
