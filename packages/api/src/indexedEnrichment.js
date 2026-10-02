// Explorer read models for SDK wire versions newer than the legacy reader.
// These are indexed-chain views, not a replacement for proof verification.
const Token = require('./models/Token')
const StateTransitionEnum = require('./enums/StateTransitionEnum')
const { DPNS_CONTRACT } = require('./constants')

const isContractVersionError = error => /unable to deserialize DataContract: UnexpectedVariant.*type_name: "DataContractConfig"/.test(error?.message ?? '')
const isDocumentVersionError = error => /dpp unknown version on Document::from_bytes \(deserialization\), known versions:.*received:/.test(error?.message ?? '')

const getIndexedTokenContract = async (knex, identifier, decode) => {
  const row = await knex('data_contracts')
    .select('state_transitions.data', 'state_transitions.hash', 'state_transitions.block_height')
    .join('state_transitions', 'state_transitions.hash', 'data_contracts.state_transition_hash')
    .where('data_contracts.identifier', identifier)
    .orderBy('data_contracts.version', 'desc')
    .orderBy('data_contracts.id', 'desc')
    .first()
  if (!row?.data) throw new Error(`Indexed contract transition unavailable: ${identifier}`)
  const contract = await decode(row.data)
  if (![StateTransitionEnum.DATA_CONTRACT_CREATE, StateTransitionEnum.DATA_CONTRACT_UPDATE].includes(contract.type) || contract.dataContractId !== identifier || !Array.isArray(contract.tokens)) {
    throw new Error(`Indexed contract transition mismatch: ${identifier}`)
  }
  return {
    ...contract,
    configurationSource: {
      type: 'index',
      stateTransitionHash: row.hash,
      blockHeight: row.block_height,
      version: contract.version
    }
  }
}

const indexedTokenToModel = async (contract, config, row, aliases, sdk, priceTx) => {
  const supply = await sdk.tokens.getTokenTotalSupply(config.tokenId)
  const token = Token.fromObject({
    identifier: config.tokenId,
    dataContractIdentifier: contract.dataContractId,
    owner: { identifier: contract.ownerId, aliases },
    price: priceTx?.price,
    prices: priceTx?.prices,
    timestamp: row.timestamp,
    totalGasUsed: Number(row.total_gas_used),
    totalTransitionsCount: Number(row.total_transitions_count),
    totalBurnTransitionsCount: Number(row.total_burn_transitions_count),
    totalFreezeTransitionsCount: Number(row.total_freeze_transitions_count),
    position: config.position,
    totalSupply: supply?.totalSystemAmount.toString(),
    description: config.description,
    localizations: config.conventions.localizations,
    decimals: config.conventions.decimals,
    baseSupply: config.baseSupply,
    maxSupply: config.maxSupply,
    mintable: config.manualMintingRules.authorizedToMakeChange.takerType !== 'NoOne',
    burnable: config.manualBurningRules.authorizedToMakeChange.takerType !== 'NoOne',
    freezable: config.freezeRules.authorizedToMakeChange.takerType !== 'NoOne',
    changeMaxSupply: config.maxSupplyChangeRules.authorizedToMakeChange.takerType !== 'NoOne',
    unfreezable: config.unfreezeRules.authorizedToMakeChange.takerType !== 'NoOne',
    destroyable: config.destroyFrozenFundsRules.authorizedToMakeChange.takerType !== 'NoOne',
    allowedEmergencyActions: config.emergencyActionRules.authorizedToMakeChange.takerType !== 'NoOne',
    mainGroup: config.mainControlGroup,
    perpetualDistribution: config.distributionRules.perpetualDistribution,
    preProgrammedDistribution: config.distributionRules.preProgrammedDistribution
  })
  return { ...token, configurationSource: contract.configurationSource }
}

const getIndexedAliasDocument = async (knex, identifier) => {
  // Select the current event before checking deletion; an old create must never
  // resurrect a deleted alias. Transfers/pricing events can have null data.
  const { rows } = await knex.raw(`
    WITH candidates AS (
      SELECT DISTINCT d.identifier FROM documents d
      JOIN data_contracts c ON c.id = d.data_contract_id
      WHERE c.identifier = ? AND d.document_type_name = 'domain'
        AND d.data #>> '{records,identity}' = ?
    ), latest AS (
      SELECT DISTINCT ON (d.identifier) d.* FROM documents d
      JOIN candidates c ON c.identifier = d.identifier
      ORDER BY d.identifier, d.id DESC
    )
    SELECT latest.identifier, properties.data, created.timestamp
    FROM latest
    JOIN LATERAL (
      SELECT d.data FROM documents d WHERE d.identifier = latest.identifier
        AND d.data IS NOT NULL ORDER BY d.id DESC LIMIT 1
    ) properties ON true
    LEFT JOIN LATERAL (
      SELECT b.timestamp FROM documents d
      JOIN state_transitions s ON s.hash = d.state_transition_hash
      JOIN blocks b ON b.hash = s.block_hash
      WHERE d.identifier = latest.identifier AND d.transition_type = 0
      ORDER BY d.id ASC LIMIT 1
    ) created ON true
    WHERE latest.deleted = false AND properties.data #>> '{records,identity}' = ?
    ORDER BY latest.identifier LIMIT 1
  `, [DPNS_CONTRACT, identifier, identifier])
  const [row] = rows
  if (!row) return undefined
  const { label, normalizedLabel, parentDomainName, records } = row.data ?? {}
  if (typeof label !== 'string' || typeof normalizedLabel !== 'string' || typeof parentDomainName !== 'string' || records?.identity !== identifier) {
    throw new Error(`Invalid indexed DPNS document: ${row.identifier}`)
  }
  return {
    id: { base58: () => row.identifier },
    properties: row.data,
    createdAt: row.timestamp == null ? null : new Date(row.timestamp).getTime(),
    // A successful contested create is not proof of winning the name. Preserve
    // the label but use the API's existing unknown status until verified.
    aliasStatus: /^[a-zA-Z01-]{3,19}$/.test(normalizedLabel) ? 'unknown' : 'ok',
    source: 'index'
  }
}

module.exports = { isContractVersionError, isDocumentVersionError, getIndexedTokenContract, indexedTokenToModel, getIndexedAliasDocument }
