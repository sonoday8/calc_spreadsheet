import assert from 'node:assert/strict'
import test from 'node:test'
import { calcSpreadsheet } from '../index.js'

test('basic arithmetic and SUM', () => {
  const result = calcSpreadsheet({
    A1: '2',
    A2: '3',
    B1: '10',
    B2: '20',
    C1: '=SUM(A1:A2*B1:B2)',
  })
  assert.equal(result.C1, 80)
})

test('placeholder replace', () => {
  const result = calcSpreadsheet(
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
  assert.equal(result.A1, 'Alice')
  assert.equal(result.B1, 'NAME')
  assert.equal(result.C1, '__UNKNOWN__')
  assert.equal(result.D1, 20)
  assert.equal(result.E1, 8)
  assert.equal(result.F1, 7)
})

test('unknown placeholder in formula behaves like missing cell', () => {
  const result = calcSpreadsheet({
    Z1: '=__UNKNOWN__*2',
    Z2: '=__UNKNOWN__+1',
  })
  assert.equal(result.Z1, 0)
  assert.equal(result.Z2, 1)
})

test('invalid replacement keys warn and are ignored', () => {
  const warnings = []
  const originalWarn = console.warn
  console.warn = (...args) => {
    warnings.push(args.map(String).join(' '))
  }
  try {
    const result = calcSpreadsheet(
      { A1: '=__OK__+1' },
      {
        __OK__: 3,
        bad: 9,
        __name__: 'x',
      },
    )
    assert.equal(result.A1, 4)
    assert.equal(warnings.length, 1)
    assert.match(warnings[0], /bad/)
    assert.match(warnings[0], /__name__/)
    assert.match(warnings[0], /ignored invalid replacement key/)
  } finally {
    console.warn = originalWarn
  }
})

test('numeric cell and replacement inputs', () => {
  const result = calcSpreadsheet(
    { A1: 5, B1: '=A1*__RATE__' },
    { __RATE__: 2 },
  )
  assert.equal(result.A1, 5)
  assert.equal(result.B1, 10)
})

test('negative thresholds throw', () => {
  assert.throws(
    () => calcSpreadsheet({ A1: '1' }, {}, -1, null),
    /min_layer_width must be >= 0/,
  )
  assert.throws(
    () => calcSpreadsheet({ A1: '1' }, {}, null, -1),
    /min_layer_work must be >= 0/,
  )
})

test('formula error surfaces as Error', () => {
  assert.throws(() => calcSpreadsheet({ A1: '=1/0' }), Error)
})
