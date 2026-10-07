const { it } = require('node:test')
const assert = require('node:assert/strict')
const { createQuorumBackfill, missingQuorumHash, validateCoreQuorum, prefetchLocalContext } = require('../../src/identityQuorumContext')
const core = require('./mocks/sakura-historical-quorum.json')

it('fills only exact typed quorum-cache misses with strictly matched Core107 identity/key', () => {
  const message = `context provider error: invalid quorum: Quorum not found in cache for hash: ${core.quorumHash}`
  assert.equal(missingQuorumHash({ kind: 19, message }), core.quorumHash)
  assert.equal(missingQuorumHash({ kind: 4, message }), null)
  assert.equal(missingQuorumHash({ kind: 19, message: 'invalid signature' }), null)
  const row = validateCoreQuorum(core, core.quorumHash)
  assert.equal(row.key, core.quorumPublicKey)
  assert.equal(row.height, 56400)
  for (const change of [{ type: 'llmq_test_platform' }, { quorumHash: '00'.repeat(32) }, { quorumPublicKey: 'invalid' }, { height: -1 }]) assert.throws(() => validateCoreQuorum({ ...core, ...change }, core.quorumHash), /Invalid historical/)
})

it('imports unchanged official lists plus matched historical key through real published SDK and closes loopback feed', async () => {
  const m = await import('@dashevo/wasm-sdk')
  await m.default()
  const current = { success: true, data: [], retained: 'current metadata' }
  const previous = { success: true, data: { height: 56496, quorums: [] }, retained: 'previous metadata' }
  const requests = []
  const backfill = createQuorumBackfill('sakura', async (url, options) => {
    requests.push(url)
    assert.equal(options.redirect, 'error')
    assert.ok(options.signal)
    return { ok: true, json: async () => url.includes('quorum/info') ? core : url.endsWith('/previous') ? previous : current }
  })
  let url
  const facade = {
    WasmTrustedContext: {
      async prefetchDevnetWithUrl (base, discovery) {
        url = base
        assert.equal(new URL(base).hostname, '127.0.0.1')
        const list = await (await fetch(base + '/quorums')).json()
        assert.equal(list.retained, current.retained)
        assert.equal(list.data[0].quorum_hash, core.quorumHash)
        assert.deepEqual(await (await fetch(base + '/previous')).json(), previous)
        return m.WasmTrustedContext.prefetchDevnetWithUrl(base, discovery)
      }
    }
  }
  const context = await backfill(facade, core.quorumHash)
  context.free()
  assert.equal(requests.length, 3)
  assert.ok(requests.includes(`http://127.0.0.1:3005/quorum/info?quorumType=107&quorumHash=${core.quorumHash}`))
  await assert.rejects(fetch(url + '/quorums'), /fetch failed/)
})

it('closes temporary loopback feed after SDK rejection without replacing the error', async () => {
  let url
  const m = { WasmTrustedContext: { async prefetchDevnetWithUrl (base) { url = base; throw new Error('SDK rejected context') } } }
  await assert.rejects(prefetchLocalContext(m, { success: true, data: [] }, { success: true, data: { height: 1, quorums: [] } }), /SDK rejected context/)
  await assert.rejects(fetch(url + '/quorums'), /fetch failed/)
})

it('refuses conflicts between Core historical key and either official list', async () => {
  for (const conflictList of ['current', 'previous']) {
    const conflict = { quorum_hash: core.quorumHash, key: '00'.repeat(48), height: core.height, valid_members_count: 4 }
    const backfill = createQuorumBackfill('sakura', async url => ({
      ok: true,
      json: async () => url.includes('quorum/info')
        ? core
        : url.endsWith('/previous')
          ? { success: true, data: { height: 56496, quorums: conflictList === 'previous' ? [conflict] : [] } }
          : { success: true, data: conflictList === 'current' ? [conflict] : [] }
    }))
    await assert.rejects(backfill({}, core.quorumHash), /Conflicting trusted quorum key/)
  }
})
