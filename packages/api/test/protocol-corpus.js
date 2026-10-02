// Read-only, opt-in gate over an immutable Tenderdash snapshot. No SDK client,
// RPC, database, transaction broadcast or shared build cache is used.
const fs = require('node:fs')
const readline = require('node:readline')
const crypto = require('node:crypto')
const assert = require('node:assert/strict')
const { decodeStateTransition } = require('../src/utils')
const { decodeProtocolStateTransition } = require('../src/protocolDecoder')
const { StateTransitionWASM } = require('pshenmic-dpp')

const existingFields = (expected, actual, path = '') => {
  if (expected && typeof expected === 'object') {
    assert.ok(actual && typeof actual === 'object', path)
    for (const [key, value] of Object.entries(expected)) existingFields(value, actual[key], `${path}.${key}`)
  } else assert.equal(actual, expected, path)
}
const json = value => JSON.parse(JSON.stringify(value))

async function main () {
  const path = process.env.EXPLORER_PROTOCOL_SNAPSHOT
  assert.ok(path, 'Set EXPLORER_PROTOCOL_SNAPSHOT to the retained NDJSON snapshot')
  const input = fs.createReadStream(path)
  const digest = crypto.createHash('sha256')
  input.on('data', chunk => digest.update(chunk))
  let manifest
  let height = 0
  let transactions = 0
  let compatibleComparisons = 0
  const types = {}
  for await (const line of readline.createInterface({ input })) {
    const record = JSON.parse(line)
    if (record.kind === 'manifest') { manifest = record; continue }
    if (record.kind !== 'block') continue
    assert.equal(record.height, height + 1)
    height = record.height
    for (const tx of record.block.block.data.txs ?? []) {
      const decoded = await decodeStateTransition(tx)
      assert.equal(decoded.raw, Buffer.from(tx, 'base64').toString('hex'), `height ${height}`)
      json(decoded)
      transactions++
      types[decoded.type] = (types[decoded.type] ?? 0) + 1
      let legacy
      try { legacy = StateTransitionWASM.fromBase64(tx) } catch { continue }
      if ([0, 1, 5].includes(legacy.getActionTypeNumber())) {
        existingFields(json(decoded), json(await decodeProtocolStateTransition(tx)), `height ${height}`)
        compatibleComparisons++
      }
    }
  }
  assert.ok(manifest)
  assert.equal(height, manifest.end)
  console.log(JSON.stringify({ snapshotSha256: digest.digest('hex'), chain: manifest.chain, capturedEnd: height, observedTip: manifest.observedTip, transactions, compatibleComparisons, types, scope: 'captured prefix decode/presentation only; not full-tip or database replay' }, null, 2))
}

main().catch(error => { console.error(error); process.exitCode = 1 })
