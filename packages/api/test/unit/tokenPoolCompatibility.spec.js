const { describe, it } = require('node:test')
const assert = require('node:assert/strict')
const crypto = require('node:crypto')
const { decodeStateTransition } = require('../../src/utils')
const { decodeProtocolStateTransition } = require('../../src/protocolDecoder')
const BatchEnum = require('../../src/enums/BatchEnum')
const pool = require('./mocks/sakura-token-pool-7392.json')
const updates = require('./mocks/sakura-contract-updates.json')

describe('published beta2 token pool/contract update compatibility', () => {
  it('presents the exact successful7392 V1 token pool with normalized fields and full canonical rules', async () => {
    const bytes = Buffer.from(pool.base64, 'base64')
    assert.equal(crypto.createHash('sha256').update(bytes).digest('hex'), pool.sha256)
    assert.equal(pool.result.code ?? 0, 0)
    const decoded = await decodeStateTransition(pool.base64)
    assert.equal(decoded.raw, bytes.toString('hex'))
    assert.equal(decoded.typeString, 'DATA_CONTRACT_CREATE')
    const token = decoded.tokens[0]
    const canonical = decoded.protocolFields.dataContract.tokens[0]
    assert.equal(canonical.$formatVersion, '1')
    assert.equal(token.hasShieldedPool, true)
    assert.equal(token.baseSupply, '1000000')
    assert.equal(token.conventions.localizations.en.singularForm, 'qapool')
    assert.equal(token.minimumPoolNotesForOutgoing, null)
    assert.equal(token.minimumPoolNotesForOutgoingChangeRules.authorizedToMakeChange.takerType, 'NoOne')
    assert.doesNotThrow(() => JSON.stringify(decoded))
  })

  it('fully presents actual successful and failed contract updates rather than raw-only fallback', async () => {
    for (const fixture of updates) {
      const bytes = Buffer.from(fixture.txBase64, 'base64')
      assert.equal(crypto.createHash('sha256').update(bytes).digest('hex').toUpperCase(), fixture.txSha256.toUpperCase())
      const decoded = await decodeStateTransition(fixture.txBase64)
      assert.equal(decoded.typeString, 'DATA_CONTRACT_UPDATE')
      assert.equal(decoded.raw, bytes.toString('hex'))
      const contract = decoded.protocolFields.dataContract
      assert.equal(decoded.dataContractId, contract.id)
      assert.equal(decoded.ownerId, contract.ownerId)
      assert.equal(decoded.dataContractOwner, contract.ownerId)
      assert.equal(decoded.version, contract.version)
      assert.equal(decoded.identityContractNonce, String(decoded.protocolFields['$identity-contract-nonce']))
      assert.deepEqual(decoded.schema, contract.documentSchemas)
      assert.ok(decoded.internalConfig)
      assert.doesNotThrow(() => JSON.stringify(decoded))
    }
  })

  it('preserves existing supported legacy update fields and existing numeric batch IDs', async () => {
    const fixture = require('./mocks/data_contract_update.json')
    const legacy = JSON.parse(JSON.stringify(await decodeStateTransition(fixture.data)))
    const modern = JSON.parse(JSON.stringify(await decodeProtocolStateTransition(fixture.data)))
    for (const field of ['type', 'typeString', 'identityContractNonce', 'dataContractId', 'dataContractOwner', 'ownerId', 'version', 'schema', 'raw', 'signature']) assert.deepEqual(modern[field], legacy[field], field)
    assert.equal(BatchEnum.TOKEN_BURN, 6)
    assert.equal(BatchEnum.DOCUMENT_INDEX_ONLY_DELETE, 17)
    assert.equal(BatchEnum.TOKEN_SHIELD, 18)
    assert.equal(BatchEnum[24], 'TOKEN_DIRECT_PURCHASE_TO_POOL')
  })
})
