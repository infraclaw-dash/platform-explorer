const { describe, it } = require('node:test')
const assert = require('node:assert/strict')
const { createIdentityReader, formatKey, readAliasInfo } = require('../../src/verifiedIdentity')
const { getAliasStateByVote } = require('../../src/utils')
const fixture = require('./mocks/sakura-identity-key-v1.json')

describe('verified protocol-14 identity reads', () => {
  it('preserves contract-bound key identifiers with the modern SDK accessor', async () => {
    const m = await import('@dashevo/wasm-sdk')
    await m.default()
    const bounds = m.ContractBounds.SingleContractDocumentType(fixture.identifier, 'domain')
    const key = new m.IdentityPublicKey({ keyId: 20, purpose: 0, securityLevel: 2, keyType: 2, data: new Uint8Array(20), contractBounds: bounds, totalBudget: 20000000000n })
    try {
      assert.deepEqual(formatKey(key).contractBounds, { identifier: fixture.identifier, documentTypeName: 'domain' })
      assert.equal(formatKey(key).totalBudget, '20000000000')
    } finally { key.free(); bounds.free() }
  })

  it('retains alias classification and won/pending/locked/unknown statuses without hiding RPC failures', async () => {
    const m = await import('@dashevo/wasm-sdk')
    await m.default()
    let calls = 0
    let mode = 'pending'
    let frees = 0
    const sdk = {
      async getContestedResourceVoteState (query) {
        calls++
        assert.deepEqual(query.indexValues, ['dash', 'b01'])
        assert.equal(query.resultType, 'documentsAndVoteTally')
        if (mode === 'error') throw new Error('Document proof invalid')
        return {
          contenders: mode === 'empty' ? [] : [{ serializedDocument: new Uint8Array([3]), free () {} }],
          winner: mode === 'pending' ? undefined : { identityId: mode === 'won' ? new m.Identifier(fixture.identifier) : undefined, free () {} },
          free () { frees++ }
        }
      }
    }
    const alias = { alias: 'BoI.dash', timestamp: 1700000000000, tx: 'abc' }
    for (const [next, expected] of [['pending', 'pending'], ['won', 'ok'], ['locked', 'locked'], ['empty', 'unknown']]) {
      mode = next
      const info = await readAliasInfo(sdk, alias.alias)
      const result = getAliasStateByVote(info, alias, fixture.identifier)
      assert.equal(result.status, expected)
      assert.equal(result.alias, alias.alias)
      assert.equal(result.contested, true)
      assert.equal(result.txHash, alias.tx)
      assert.equal(result.timestamp.getTime(), alias.timestamp)
    }
    assert.equal(frees, 4)
    const unContested = await readAliasInfo(sdk, 'long-label-012345678901234567890.dash')
    assert.equal(getAliasStateByVote(unContested, { alias: 'long-label-012345678901234567890.dash' }, fixture.identifier).contested, false)
    assert.equal(calls, 4)
    mode = 'error'
    await assert.rejects(readAliasInfo(sdk, alias.alias), /Document proof invalid/)
  })

  it('preserves all 19 actual keys, legacy fields and the budget/expiry key without truncation', async () => {
    const m = await import('@dashevo/wasm-sdk')
    await m.default()
    const { IdentityPublicKeyWASM } = require('pshenmic-dpp')
    for (const f of fixture.keys) {
      const key = m.IdentityPublicKey.fromHex(f.raw)
      const result = formatKey(key)
      assert.equal(result.raw, f.raw)
      assert.equal(result.data, f.data)
      assert.equal(result.publicKeyHash, f.hash)
      assert.equal(result.keyType, f.keyType)
      assert.equal(result.purpose, f.purpose)
      assert.equal(result.securityLevel, f.securityLevel)
      assert.deepEqual(result.protocolFields, f.json)
      assert.equal(result.totalBudget, f.json.totalBudget == null ? null : String(f.json.totalBudget))
      assert.equal(result.expiresAt, f.json.expiresAt == null ? null : String(f.json.expiresAt))
      if (f.json.$formatVersion === '0') {
        const old = IdentityPublicKeyWASM.fromHex(f.raw)
        assert.equal(result.data, old.data)
        assert.equal(result.readOnly, old.readOnly)
        assert.equal(result.raw, old.hex())
        assert.equal(result.publicKeyHash, old.getPublicKeyHash())
        const bounds = old.getContractBounds()
        assert.deepEqual(result.contractBounds, bounds ? { identifier: bounds.identifier.base58(), documentTypeName: bounds.documentTypeName ?? null } : null)
      } else {
        assert.throws(() => IdentityPublicKeyWASM.fromHex(f.raw), /IdentityPublicKey/)
      }
      key.free()
    }
    assert.equal(fixture.keys.length, 19)
    assert.equal(fixture.keys.filter(k => k.json.$formatVersion === '1').length, 1)
  })

  it('requires proof-enabled explicit devnet queries, retains amounts/nonces and refreshes only bounded trust cache', async () => {
    const m = await import('@dashevo/wasm-sdk')
    await m.default()
    let now = 1
    let fetches = 0
    let disposed = 0
    let rejectProof = false
    const options = []
    const builder = {
      withVersion (v) { assert.equal(v, 14); return this },
      withTrustedContext () { return this },
      withProofs (v) { assert.equal(v, true); return this },
      withSettings (...v) { assert.deepEqual(v, [10000, 15000, 1, false]); return this },
      build () {
        return {
          async getIdentityKeys (query) {
            assert.deepEqual(query, { identityId: fixture.identifier, request: { type: 'all' } })
            if (rejectProof) throw new Error('invalid proof')
            return fixture.keys.map(k => m.IdentityPublicKey.fromHex(k.raw))
          },
          async getIdentity () { return { balance: 9007199254740999n, revision: 19n, free () {} } },
          async getIdentityNonce () { return 9007199254740997n },
          free () { disposed++ }
        }
      }
    }
    const reader = createIdentityReader({
      devnet: 'sakura',
      addresses: ['https://fixture.invalid'],
      now: () => now,
      loadModule: async () => ({
        WasmSdkBuilder: { withAddresses (...args) { options.push(args); return builder } },
        WasmTrustedContext: { async prefetchDevnet (name, discovery) { assert.equal(name, 'sakura'); assert.equal(discovery, false); fetches++; return { free () {} } } }
      })
    })
    const first = await reader(fixture.identifier)
    assert.equal(first.publicKeys.length, 19)
    assert.equal(first.identityInfo.balance, 9007199254740999n)
    assert.equal(first.nonce, 5n) // Legacy API exposes only the stored nonce's low40bits.
    assert.equal(BigInt(fixture.nonce) & 0xFFFFFFFFFFn, 35n)
    await reader(fixture.identifier)
    assert.equal(fetches, 1)
    now += 60001
    await reader(fixture.identifier)
    assert.equal(fetches, 2)
    rejectProof = true
    await assert.rejects(reader(fixture.identifier), /invalid proof/)
    assert.equal(disposed, 4)
    assert.ok(options.every(args => args[1] === 'devnet'))
  })
})
