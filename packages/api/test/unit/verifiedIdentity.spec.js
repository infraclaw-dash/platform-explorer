const { describe, it } = require('node:test')
const assert = require('node:assert/strict')
const { createIdentityReader, formatKey } = require('../../src/verifiedIdentity')
const fixture = require('./mocks/sakura-identity-key-v1.json')

describe('verified protocol-14 identity reads', () => {
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
    assert.equal(first.nonce, 9007199254740997n)
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
