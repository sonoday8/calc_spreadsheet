/**
 * Load bench matching examples/bench_replace_load.rs / ext-php bench (large profile).
 *
 *   npm run bench
 *   # or: npm run build && node examples/bench_replace_load.mjs
 */

import { calcSpreadsheet } from '../index.js'

const LEAF_COUNT = 1024
const MID_WIDTH = 8192
const MID_REFS = 8
const HEAVY_WIDTH = 8192
const HEAVY_REFS = 40
const REPLACE_KEYS = 512
const PLACEHOLDERS_PER_FORMULA = 6
const WARMUP = 2
const ITERATIONS = 5

function buildSheetAndReplacements() {
  const replacements = {}
  for (let i = 0; i < REPLACE_KEYS; i++) {
    const key = `__R${i}__`
    if (i % 17 === 0) {
      replacements[key] = `T${i}`
    } else {
      replacements[key] = (i % 97) + 1
    }
  }

  const cells = {}
  for (let i = 0; i < LEAF_COUNT; i++) {
    cells[`L${i}`] = String(i % 97)
  }

  for (let i = 0; i < MID_WIDTH; i++) {
    const args = []
    for (let j = 0; j < MID_REFS; j++) {
      args.push(`L${(i + j) % LEAF_COUNT}`)
    }
    const ph = []
    for (let j = 0; j < PLACEHOLDERS_PER_FORMULA; j++) {
      let k = (i + j * 3) % REPLACE_KEYS
      if (k % 17 === 0) {
        k = (k + 1) % REPLACE_KEYS
      }
      ph.push(`__R${k}__`)
    }
    cells[`M${i}`] = `=SUM(${args.join(',')},${ph.join(',')})`
  }

  for (let i = 0; i < 32; i++) {
    const k = (i * 17) % REPLACE_KEYS
    cells[`C${i}`] = `="id="&__R${k}__&M${i}`
  }

  for (let i = 0; i < HEAVY_WIDTH; i++) {
    const args = []
    for (let j = 0; j < HEAVY_REFS; j++) {
      args.push(`M${(i + j) % MID_WIDTH}`)
    }
    for (let j = 0; j < PLACEHOLDERS_PER_FORMULA; j++) {
      let k = (i * 5 + j) % REPLACE_KEYS
      if (k % 17 === 0) {
        k = (k + 1) % REPLACE_KEYS
      }
      args.push(`__R${k}__`)
    }
    cells[`H${i}`] = `=SUM(${args.join(',')})`
  }

  return [cells, replacements]
}

function measure(n, fn) {
  let totalMs = 0
  let minMs = Number.POSITIVE_INFINITY
  let last = null
  for (let i = 0; i < n; i++) {
    const t0 = performance.now()
    last = fn()
    const elapsed = performance.now() - t0
    totalMs += elapsed
    if (elapsed < minMs) {
      minMs = elapsed
    }
  }
  return {
    avg_ms: totalMs / n,
    min_ms: minMs,
    total_ms: totalMs,
    last,
  }
}

console.log('=== bench_replace_load (Node) ===')
console.log(`Node ${process.version}`)

const buildStart = performance.now()
const [cells, replacements] = buildSheetAndReplacements()
const buildMs = performance.now() - buildStart

let placeholderHits = 0
for (const expr of Object.values(cells)) {
  placeholderHits += (String(expr).match(/__R/g) || []).length
}

console.log(`cells in:           ${Object.keys(cells).length}`)
console.log(`replacement keys:   ${Object.keys(replacements).length}`)
console.log(`placeholder tokens: ~${placeholderHits}`)
console.log(`sheet build:        ${buildMs.toFixed(3)} ms`)
console.log(`warmup=${WARMUP}, iterations=${ITERATIONS}`)
console.log()

for (let i = 0; i < WARMUP; i++) {
  calcSpreadsheet(cells, replacements)
  calcSpreadsheet(cells, {})
}

const withRep = measure(ITERATIONS, () => calcSpreadsheet(cells, replacements))
const without = measure(ITERATIONS, () => calcSpreadsheet(cells, {}))

const sample = withRep.last.H0
if (typeof sample !== 'number') {
  console.error(`expected numeric H0, got: ${sample}`)
  process.exit(1)
}

console.log('--- with replacements ---')
console.log(
  `  avg ${withRep.avg_ms.toFixed(3)} ms | min ${withRep.min_ms.toFixed(3)} ms | total ${withRep.total_ms.toFixed(3)} ms (n=${ITERATIONS})`,
)
console.log(`  cells out: ${Object.keys(withRep.last).length}`)
console.log(`  sample H0 = ${sample}`)
console.log()

console.log('--- without replacements ---')
console.log(
  `  avg ${without.avg_ms.toFixed(3)} ms | min ${without.min_ms.toFixed(3)} ms | total ${without.total_ms.toFixed(3)} ms (n=${ITERATIONS})`,
)
console.log()

const ratio = withRep.avg_ms / Math.max(without.avg_ms, 1e-9)
console.log(
  `replace path / empty path (avg): ${ratio.toFixed(2)}x (${withRep.avg_ms.toFixed(1)} ms vs ${without.avg_ms.toFixed(1)} ms)`,
)
