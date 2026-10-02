const { describe, it } = require('node:test')
const assert = require('node:assert/strict')
const crypto = require('node:crypto')
const { decodeStateTransition } = require('../../src/utils')
const { decodeProtocolStateTransition } = require('../../src/protocolDecoder')
const TransactionsController = require('../../src/controllers/TransactionsController')
const BatchEnum = require('../../src/enums/BatchEnum')
const fixtures = require('./mocks/sakura-protocol-14.json').transactions
const { StateTransitionWASM } = require('pshenmic-dpp')

const json = value => JSON.parse(JSON.stringify(value))

describe('protocol-14 Explorer presentation', () => {
  it('decodes the original block 325 V2 bytes without changing identifiers, bytes or numeric types', async () => {
    const f = fixtures.documentV2
    assert.equal(f.height, 325)
    assert.throws(() => StateTransitionWASM.fromBase64(f.base64), /DocumentBaseTransition/)
    const result = await decodeStateTransition(f.base64)
    assert.equal(result.typeString, 'BATCH')
    assert.equal(result.raw, Buffer.from(f.base64, 'base64').toString('hex'))
    assert.equal(result.transitions.length, 1)
    const document = result.transitions[0]
    assert.equal(document.action, 'DOCUMENT_CREATE')
    assert.equal(document.id, 'HzRz773fNcC7HtdYEKmi6ksoHXrvDDjaUgwgo1VWuQEV')
    assert.equal(document.dataContractId, 'GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec')
    assert.equal(document.identityContractNonce, '1')
    assert.equal(document.revision, '1')
    assert.deepEqual(document.data.saltedDomainHash, Uint8Array.from(Buffer.from('ab2OOZSnxSr1QqNmzrlYZxPbine0k0AuOnqYcqKQJ84=', 'base64')))
    assert.equal(document.actionFeeAgreement, null)
    assert.equal(result.protocolFields.transitions[0].$baseFormatVersion, '2')
  })

  it('retains exact wire data and JSON-serializable fields for each captured compatibility variant', async () => {
    for (const f of Object.values(fixtures)) {
      const bytes = Buffer.from(f.base64, 'base64')
      assert.equal(crypto.createHash('sha256').update(bytes).digest('hex'), f.sha256)
      const decoded = await decodeStateTransition(f.base64)
      assert.equal(decoded.type, f.type)
      assert.equal(decoded.raw, bytes.toString('hex'))
      assert.doesNotThrow(() => JSON.stringify(decoded))
    }
  })

  it('keeps the full index-only property tuple and appends, rather than renumbers, its action', async () => {
    const decoded = await decodeStateTransition(fixtures.indexOnlyDelete.base64)
    assert.equal(BatchEnum.TOKEN_BURN, 6)
    assert.equal(BatchEnum.TOKEN_SET_PRICE_FOR_DIRECT_PURCHASE, 16)
    assert.equal(BatchEnum.DOCUMENT_INDEX_ONLY_DELETE, 17)
    assert.equal(decoded.transitions[0].action, 'DOCUMENT_INDEX_ONLY_DELETE')
    assert.deepEqual(Object.keys(decoded.transitions[0].data).sort(), ['postAuthor', 'postId'])
    assert.equal(Buffer.from(decoded.transitions[0].data.postId).toString('base64'), decoded.protocolFields.transitions[0].postId)
  })

  it('exposes moderation action details and fee pot without inventing a payout', async () => {
    const moderation = await decodeStateTransition(fixtures.moderation.base64)
    assert.equal(fixtures.moderation.height, 703)
    assert.equal(moderation.typeString, 'CONTRACT_USER_MODERATION')
    assert.equal(moderation.action.$type, 'deleteDocument')
    assert.equal(moderation.action.documentId, '6TbhLVNPwXjX2JEemGXbypLifEWrgWspv2xS7QM7cfzh')
    assert.equal(moderation.action.documentTypeName, 'post')
    assert.deepEqual(moderation.action.reason, { code: null, text: 'kf-1 proof' })
    const claim = await decodeStateTransition(fixtures.feeClaim.base64)
    assert.equal(claim.typeString, 'CONTRACT_FEE_CLAIM')
    assert.equal(claim.pot, 'moderators')
    assert.equal(claim.identityContractNonce, '24')
    assert.equal('amount' in claim, false)
  })

  it('retains moderation config, new distributions and budgeted/expiring keys', async () => {
    const contract = await decodeStateTransition(fixtures.contractV1.base64)
    assert.equal(contract.internalConfig.moderation.moderators.$type, 'elected')
    assert.equal(contract.internalConfig.sizedIntegerTypes, true)
    assert.ok(contract.schema.post)
    const claim = await decodeStateTransition(fixtures.oncePerIdentity.base64)
    assert.equal(claim.transitions[0].distributionType, 'OncePerIdentity')
    assert.equal(claim.transitions[0].action, 'TOKEN_CLAIM')
    assert.equal(claim.transitions[0].identityContractNonce, '3')
    assert.ok(claim.transitions[0].historicalDocumentId)
    const update = await decodeStateTransition(fixtures.budgetedKey.base64)
    assert.equal(update.publicKeysToAdd[0].totalBudget, '20000000000')
    assert.equal(update.publicKeysToAdd[0].expiresAt, '1822424143185')
    assert.equal(update.publicKeysToAdd[0].contractBounds.typeName, 'checkRun')
  })

  it('represents unknown chain-lock funding amount explicitly, without dereferencing a missing output', async () => {
    const decoded = await decodeStateTransition(fixtures.identityTopUp.base64)
    assert.equal(decoded.assetLockProof.type, 'chainLock')
    assert.equal(decoded.assetLockProof.fundingAmount, null)
    assert.equal(decoded.amount, null)
  })

  it('matches the established normalized contract where both decoders support the wire', async () => {
    for (const name of ['document_transition', 'data_contract_create', 'token_transfer_transition', 'token_mint_transition', 'identity_update']) {
      const { data } = require(`./mocks/${name}.json`)
      const expected = json(await decodeStateTransition(data))
      const adapted = json(await decodeProtocolStateTransition(data))
      delete adapted.protocolFields
      assert.deepEqual(adapted, expected, name)
    }
    const expected = json(await decodeStateTransition(fixtures.binaryData.base64))
    const adapted = json(await decodeProtocolStateTransition(fixtures.binaryData.base64))
    delete adapted.protocolFields
    assert.deepEqual(adapted, expected)
  })

  it('serializes the decode controller response through Fastify without network or database calls', async () => {
    const server = require('fastify')()
    const controller = new TransactionsController(null, null)
    server.post('/transactions/decode', controller.decode)
    try {
      const response = await server.inject({ method: 'POST', url: '/transactions/decode', payload: { base64: fixtures.nestedIntegerData.base64 } })
      assert.equal(response.statusCode, 200)
      assert.equal(response.json().transitions[0].data.imported.createdAt, 1)
      assert.equal(response.json().transitions[0].data.upstreamNumber, 9)
    } finally { await server.close() }
  })

  it('preserves identity/group token authority and flattened group-action fields', async () => {
    // Synthetic serialization checks only: no signing, SDK client or broadcast.
    const sdk = await import('@dashevo/wasm-sdk')
    await sdk.default()
    const original = sdk.StateTransition.fromBase64(require('./mocks/data_contract_create_with_tokens.json').data)
    const contract = sdk.DataContractCreateTransition.fromStateTransition(original)
    const fields = contract.toJSON()
    const config = Object.values(fields.dataContract.tokens)[0]
    config.manualMintingRules.authorizedToMakeChange = { $type: 'identity', identity: fields.dataContract.ownerId }
    config.mainControlGroupCanBeModified = { $type: 'group', position: 7 }
    const constructed = sdk.DataContractCreateTransition.fromJSON(fields)
    const state = constructed.toStateTransition()
    try {
      const decoded = await decodeProtocolStateTransition(state.toBase64())
      assert.equal(decoded.tokens[0].manualMintingRules.authorizedToMakeChange.taker, fields.dataContract.ownerId)
      assert.deepEqual(decoded.tokens[0].mainControlGroupCanBeModified, { takerType: 'Group', taker: 7 })
    } finally { state.free(); constructed.free(); contract.free(); original.free() }
    const batchState = sdk.StateTransition.fromBase64(fixtures.oncePerIdentity.base64)
    const batch = sdk.BatchTransition.fromStateTransition(batchState)
    const batchFields = batch.toJSON()
    Object.assign(batchFields.transitions[0], { $groupContractPosition: 0, $groupActionId: batchFields.ownerId, $groupActionIsProposer: false })
    const grouped = sdk.BatchTransition.fromJSON(batchFields)
    const groupedState = grouped.toStateTransition()
    try {
      const decoded = await decodeProtocolStateTransition(groupedState.toBase64())
      assert.deepEqual(decoded.transitions[0].groupInfo, { groupContractPosition: 0, actionId: batchFields.ownerId, actionIsProposer: false })
    } finally { groupedState.free(); grouped.free(); batch.free(); batchState.free() }
  })

  it('rejects malformed, truncated and trailing bytes rather than returning raw-only success', async () => {
    await assert.rejects(decodeProtocolStateTransition('!not-base64!'))
    const bytes = Buffer.from(fixtures.documentV2.base64, 'base64')
    await assert.rejects(decodeProtocolStateTransition(bytes.subarray(0, bytes.length - 1).toString('base64')))
    await assert.rejects(decodeProtocolStateTransition(Buffer.concat([bytes, Buffer.from([0])]).toString('base64')))
  })
})
