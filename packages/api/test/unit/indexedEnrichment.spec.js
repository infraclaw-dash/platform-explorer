const { describe, it } = require('node:test')
const assert = require('node:assert/strict')
const Fastify = require('fastify')
const knexFactory = require('knex')
const utils = require('../../src/utils')
const { getIndexedTokenContract } = require('../../src/indexedEnrichment')
const TransactionsController = require('../../src/controllers/TransactionsController')
const TokensController = require('../../src/controllers/TokensController')
const DataContractsDAO = require('../../src/dao/DataContractsDAO')
const TokensDAO = require('../../src/dao/TokensDAO')
const fixture = require('./mocks/sakura-indexed-enrichment.json')
const contractError = new Error('protocol: platform deserialization error: unable to deserialize DataContract: UnexpectedVariant { type_name: "DataContractConfig", allowed: Range { min: 0, max: 1 }, found: 2 }')
const documentError = new Error('protocol: dpp unknown version on Document::from_bytes (deserialization), known versions: [0, 1, 2], received: 3')

const legacySdk = () => ({
  documents: { query: async () => { throw documentError } },
  dataContracts: { getDataContractByIdentifier: async () => { throw contractError } },
  tokens: {
    getTokenTotalSupply: async identifier => {
      assert.ok(fixture.tokens.some(row => row.identifier === identifier))
      return { totalSystemAmount: 12345678901234567890n }
    }
  }
})

// Compile actual Knex PostgreSQL queries, intercept only the database transport.
// Parent's isolated live-DB gate executes these statements on PostgreSQL.
const database = handler => {
  const knex = knexFactory({ client: 'pg' })
  knex.client.acquireConnection = async () => ({})
  knex.client.releaseConnection = async () => {}
  knex.client._query = async (_, query) => {
    query.response = { rows: await handler(query), command: 'SELECT' }
    return query
  }
  return knex
}
const fixtureRows = query => {
  if (query.sql.includes('WITH candidates')) {
    assert.match(query.sql, /ORDER BY d.identifier, d.id DESC/)
    assert.match(query.sql, /WHERE latest.deleted = false/)
    return fixture.aliases.filter(row => row.data.records.identity === query.bindings[1])
  }
  if (query.sql.startsWith('select "state_transitions"."data"')) {
    assert.match(query.sql, /order by "data_contracts"."version" desc, "data_contracts"."id" desc/)
    return fixture.tokens.filter(row => row.data_contract_identifier === query.bindings[0]).map(row => ({ data: row.contract_transition_data, hash: row.state_transition_hash, block_height: 687 }))
  }
  throw new Error(`Unexpected query: ${query.sql}`)
}

describe('indexed read enrichment for incompatible legacy SDK wire versions', () => {
  it('serves actual transaction325 detail/list through Fastify, preserving its indexed alias', async () => {
    const row = fixture.transactions[0]
    const knex = database(query => query.sql.includes('WITH candidates') ? fixtureRows(query) : [{ ...row, tx_hash: row.hash, total_count: '1' }])
    const controller = new TransactionsController(knex, legacySdk())
    const app = Fastify()
    app.get('/transaction/:hash', controller.getTransactionByHash)
    app.get('/transactions', controller.getTransactions)
    try {
      for (const path of [`/transaction/${row.hash}`, '/transactions?limit=2']) {
        const response = await app.inject(path)
        assert.equal(response.statusCode, 200, response.body)
        const result = response.json()
        const tx = result.resultSet ? result.resultSet[0] : result
        assert.equal(tx.hash, row.hash)
        assert.equal(tx.data, row.data)
        assert.equal(tx.owner.identifier, row.owner)
        assert.deepEqual(tx.owner.aliases, [{
          alias: 'yappr-maker-260929.dash',
          status: 'ok',
          source: 'index',
          timestamp: '2026-10-01T15:25:10.704Z',
          documentId: 'DEyD2bqnZLPtFFsYYLUjgYvNzoPAapJPu3uc8XqPFZMV',
          contested: false
        }])
      }
    } finally { await app.close(); await knex.destroy() }
  })

  it('does not invent a winner for an indexed contested name or an alias absent from the index', async () => {
    const row = fixture.aliases[0]
    const knex = database(() => [{ ...row, data: { ...row.data, label: 'Name', normalizedLabel: 'name' } }])
    const alias = await utils.getAliasDocumentForIdentifier(row.owner, legacySdk(), knex)
    assert.equal(utils.getAliasFromDocument(alias).status, 'unknown')
    assert.equal(utils.getAliasFromDocument(alias).alias, 'Name.dash')
    const empty = database(() => [])
    assert.equal(await utils.getAliasDocumentForIdentifier(row.owner, legacySdk(), empty), undefined)
    await knex.destroy(); await empty.destroy()
  })

  it('serves first two actual token contracts with correct token IDs, config and lossless live supply', async () => {
    const knex = database(query => query.sql.includes('subquery') && query.sql.includes('"tokens"')
      ? fixture.tokens.map(row => ({ ...row, total_count: '2' }))
      : fixtureRows(query))
    const controller = new TokensController(knex, legacySdk())
    const app = Fastify()
    app.get('/tokens', controller.getTokens)
    try {
      const response = await app.inject('/tokens?limit=2')
      assert.equal(response.statusCode, 200, response.body)
      const { resultSet, pagination } = response.json()
      assert.equal(resultSet.length, 2)
      assert.equal(pagination.total, 2)
      for (const [i, token] of resultSet.entries()) {
        assert.equal(token.identifier, fixture.tokens[i].identifier)
        assert.equal(token.dataContractIdentifier, fixture.tokens[i].data_contract_identifier)
        assert.equal(token.owner.identifier, fixture.tokens[i].owner)
        assert.equal(token.totalSupply, '12345678901234567890')
        assert.equal(token.baseSupply, '1000000')
        assert.equal(token.maxSupply, null)
        assert.equal(token.decimals, 0)
        assert.equal(token.localizations.en.singularForm, 'yapp')
        assert.equal(token.mintable, true)
        assert.equal(token.freezable, true)
        assert.equal(token.allowedEmergencyActions, false)
        assert.equal(token.configurationSource.type, 'index')
        assert.equal(token.configurationSource.stateTransitionHash, fixture.tokens[i].state_transition_hash)
      }
    } finally { await app.close(); await knex.destroy() }
  })

  it('keeps contract-detail token/group enrichment instead of swallowing its version error', async () => {
    const token = fixture.tokens[0]
    const knex = database(query => query.sql.endsWith('as "data_sub"')
      ? [{ identifier: token.data_contract_identifier, owner: token.owner, schema: {}, version: 1 }]
      : fixtureRows(query))
    try {
      const contract = await new DataContractsDAO(knex, legacySdk()).getDataContractByIdentifier(token.data_contract_identifier)
      assert.equal(contract.tokensCount, 1)
      assert.equal(contract.tokens[0].identifier, token.identifier)
      assert.deepEqual(contract.groups, [])
    } finally { await knex.destroy() }
  })

  it('retains token trend labels through the same contract-version fallback', async () => {
    const token = fixture.tokens[0]
    const knex = database(query => query.sql.startsWith('select "token_identifier"')
      ? [{ ...token, token_identifier: token.identifier, transitions_count: '7', total_count: '1' }]
      : fixtureRows(query))
    try {
      const trends = await new TokensDAO(knex, legacySdk()).getTokensTrends(new Date(0), new Date(), 1, 2, 'asc')
      assert.equal(trends.resultSet[0].localizations.en.singularForm, 'yapp')
      assert.equal(trends.resultSet[0].transitionCount, 7)
    } finally { await knex.destroy() }
  })

  it('passes through successful legacy reads and propagates unrelated RPC/proof/database errors', async () => {
    const alias = { properties: { label: 'existing' } }
    const sdk = legacySdk()
    sdk.documents.query = async () => [alias]
    const knex = database(() => { throw new Error('database unavailable') })
    assert.equal(await utils.getAliasDocumentForIdentifier('owner', sdk, knex), alias)
    const proofError = new Error('Failed to verify query')
    sdk.documents.query = async () => { throw proofError }
    await assert.rejects(utils.getAliasDocumentForIdentifier('owner', sdk, knex), error => error === proofError)
    await assert.rejects(utils.getAliasDocumentForIdentifier('owner', legacySdk(), knex), /database unavailable/)
    sdk.documents.query = async () => []
    sdk.dataContracts.getDataContractByIdentifier = async () => { throw proofError }
    await assert.rejects(utils.fetchTokenInfoByRows(fixture.tokens, sdk, knex), error => error === proofError)
    await knex.destroy()
  })

  it('rejects missing, mismatched or corrupt indexed contract bytes and missing token positions', async () => {
    const token = fixture.tokens[0]
    const empty = database(() => [])
    await assert.rejects(getIndexedTokenContract(empty, token.data_contract_identifier, utils.decodeStateTransition), /unavailable/)
    const wrong = database(() => [{ data: fixture.tokens[1].contract_transition_data }])
    await assert.rejects(getIndexedTokenContract(wrong, token.data_contract_identifier, utils.decodeStateTransition), /mismatch/)
    const corrupt = database(() => [{ data: 'AA==' }])
    await assert.rejects(getIndexedTokenContract(corrupt, token.data_contract_identifier, utils.decodeStateTransition))
    const indexed = database(fixtureRows)
    await assert.rejects(utils.fetchTokenInfoByRows([{ ...token, position: 99 }], legacySdk(), indexed), /Indexed token configuration unavailable/)
    for (const knex of [empty, wrong, corrupt, indexed]) await knex.destroy()
  })
})
