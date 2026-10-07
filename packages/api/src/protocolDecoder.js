// Explorer-owned presentation adapter. The pinned upstream decoder is unmodified.
// Keep legacy responses unchanged; use this only when the legacy decoder rejects
// a wire variant. Never substitute raw-only success for an unmapped transition.
const StateTransitionEnum = require('./enums/StateTransitionEnum')
const BatchEnum = require('./enums/BatchEnum')
const DistributionFunction = require('./models/DistributionFunction')
const PerpetualDistribution = require('./models/PerpetualDistribution')
const PreProgrammedDistribution = require('./models/PreProgrammedDistribution')
const Localization = require('./models/Localization')

let loading
const loadDecoder = () => {
  loading ??= import('@dashevo/wasm-sdk').then(async sdk => {
    await sdk.default()
    return sdk
  })
  return loading
}
const hex = value => Buffer.from(value ?? '', 'base64').toString('hex')
const stringOrNull = value => value == null ? null : String(value)
const withoutVersion = value => Object.fromEntries(Object.entries(value ?? {}).filter(([k]) => k !== '$formatVersion'))
// Preserve legacy byte-array representation while avoiding BigInt JSON failures
// or precision loss in user document properties (not just top-level amounts).
const presentationValue = value => {
  if (typeof value === 'bigint') return value <= BigInt(Number.MAX_SAFE_INTEGER) && value >= BigInt(Number.MIN_SAFE_INTEGER) ? Number(value) : value.toString()
  if (value instanceof Uint8Array) return value
  if (Array.isArray(value)) return value.map(presentationValue)
  if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, presentationValue(item)]))
  return value
}
const upperFirst = s => s[0].toUpperCase() + s.slice(1)
const unsupported = message => { throw new Error(`Explorer protocol presentation unsupported: ${message}`) }

const actionTaker = value => ({
  takerType: upperFirst(value.$type),
  taker: value.identity ?? value.position ?? null
})
const controlRules = value => ({
  ...withoutVersion(value),
  authorizedToMakeChange: actionTaker(value.authorizedToMakeChange),
  adminActionTakers: actionTaker(value.adminActionTakers)
})

const tokenConfig = (sdk, id, position, config) => {
  const tokenId = sdk.TokenConfiguration.calculateTokenId(id, Number(position))
  let identifier
  try { identifier = tokenId.toBase58() } finally { tokenId.free() }
  const out = {
    position: Number(position),
    tokenId: identifier,
    conventions: {
      decimals: config.conventions.decimals,
      localizations: Object.fromEntries(Object.entries(config.conventions.localizations).map(([key, value]) => [key, Localization.fromObject(value)]))
    },
    baseSupply: String(config.baseSupply),
    maxSupply: stringOrNull(config.maxSupply),
    keepsHistory: withoutVersion(config.keepsHistory),
    startAsPaused: config.startAsPaused,
    isAllowedTransferToFrozenBalance: config.allowTransferToFrozenBalance,
    mainControlGroup: config.mainControlGroup ?? null,
    mainControlGroupCanBeModified: actionTaker(config.mainControlGroupCanBeModified),
    description: config.description ?? null
  }
  for (const key of ['conventionsChangeRules', 'maxSupplyChangeRules', 'manualMintingRules', 'manualBurningRules', 'freezeRules', 'unfreezeRules', 'destroyFrozenFundsRules', 'emergencyActionRules']) out[key] = controlRules(config[key])
  const distribution = config.distributionRules
  // The complete canonical rule set is also returned in protocolFields. In
  // particular, V1 once-per-identity rules must not disappear from the response.
  out.distributionRules = {
    perpetualDistribution: null,
    preProgrammedDistribution: distribution.preProgrammedDistribution
      ? PreProgrammedDistribution.fromWASMObject(distribution.preProgrammedDistribution)
      : null,
    newTokenDestinationIdentity: distribution.newTokensDestinationIdentity ?? null,
    mintingAllowChoosingDestination: distribution.mintingAllowChoosingDestination
  }
  if (distribution.perpetualDistribution) {
    const d = distribution.perpetualDistribution
    const type = d.distributionType
    const fn = type.function
    if (!type.$type || !fn?.$type || !d.distributionRecipient?.$type) unsupported('perpetual distribution shape')
    out.distributionRules.perpetualDistribution = PerpetualDistribution.fromObject({
      type: upperFirst(type.$type),
      recipientType: upperFirst(d.distributionRecipient.$type),
      recipientValue: d.distributionRecipient.identity ?? null,
      interval: Number(type.interval),
      functionName: upperFirst(fn.$type),
      functionValue: DistributionFunction.fromObject(fn)
    })
  }
  if (distribution.oncePerIdentityDistribution != null) out.distributionRules.oncePerIdentityDistribution = distribution.oncePerIdentityDistribution
  out.marketplaceRules = {
    tradeMode: config.marketplaceRules.tradeMode,
    tradeModeChangeRules: controlRules(config.marketplaceRules.tradeModeChangeRules)
  }
  if (config.$formatVersion === '1') {
    out.hasShieldedPool = config.hasShieldedPool
    out.minimumPoolNotesForOutgoing = config.minimumPoolNotesForOutgoing
    out.minimumPoolNotesForOutgoingChangeRules = controlRules(config.minimumPoolNotesForOutgoingChangeRules)
  }
  return out
}

const documentTransition = (transition, wrapped, object) => {
  const action = BatchEnum[transition.$action]
  if (!action) unsupported(`document action ${transition.$action}`)
  const out = {
    action,
    id: transition.$id,
    dataContractId: transition.$dataContractId,
    revision: String(transition.$revision ?? (transition.$action === 'create' ? 1 : 0)),
    type: transition.$type,
    identityContractNonce: String(transition.$identityContractNonce)
  }
  const payment = transition.$tokenPaymentInfo
  out.tokenPaymentInfo = payment
    ? { ...withoutVersion(payment), minimumTokenCost: stringOrNull(payment.minimumTokenCost), maximumTokenCost: stringOrNull(payment.maximumTokenCost) }
    : null
  if (transition.$baseFormatVersion === '2') out.actionFeeAgreement = transition.$actionFeeAgreement
  if (['create', 'replace', 'indexOnlyDelete'].includes(transition.$action)) {
    const controlFields = new Set(['$transition', '$action', '$formatVersion', '$baseFormatVersion', '$id', '$identityContractNonce', '$type', '$dataContractId', '$tokenPaymentInfo', '$actionFeeAgreement', '$entropy', '$prefundedVotingBalance', '$revision'])
    if (transition.$action === 'indexOnlyDelete') {
      out.data = presentationValue(Object.fromEntries(Object.entries(object).filter(([k]) => !controlFields.has(k))))
    } else {
      const concrete = transition.$action === 'create' ? wrapped.createTransition : wrapped.replaceTransition
      try { out.data = presentationValue(concrete.data) } finally { concrete.free() }
    }
  }
  if (transition.$action === 'create') {
    out.entropy = hex(transition.$entropy)
    const balance = transition.$prefundedVotingBalance
    out.prefundedVotingBalance = balance ? { [balance.indexName]: String(balance.credits) } : null
  }
  if (['purchase', 'updatePrice'].includes(transition.$action)) {
    const price = transition.$price
    if (!Number.isSafeInteger(Number(price))) unsupported('document price exceeds safe API number')
    out.price = Number(price)
  }
  if (transition.$action === 'transfer') out.recipientId = transition.$recipientId
  return out
}

const tokenTransition = (transition, wrapped, owner) => {
  const actionName = upperFirst(transition.$action)
  const action = BatchEnum[actionName]
  if (!action) unsupported(`token action ${transition.$action}`)
  const historicalId = wrapped.getHistoricalDocumentId(owner)
  const hasGroup = transition.$groupContractPosition != null
  const out = {
    action,
    tokenId: transition.$tokenId,
    identityContractNonce: String(transition['$identity-contract-nonce']),
    tokenContractPosition: transition.$tokenContractPosition,
    dataContractId: transition.$dataContractId,
    historicalDocumentTypeName: wrapped.historicalDocumentTypeName,
    historicalDocumentId: historicalId.toBase58(),
    groupInfo: hasGroup ? { groupContractPosition: transition.$groupContractPosition, actionId: transition.$groupActionId, actionIsProposer: transition.$groupActionIsProposer } : null,
    publicNote: transition.publicNote ?? null
  }
  historicalId.free()
  switch (transition.$action) {
    case 'claim': out.distributionType = transition.distributionType; break
    case 'mint': out.amount = String(transition.amount); out.issuedToIdentityId = transition.issuedToIdentityId; break
    case 'burn': out.burnAmount = String(transition.burnAmount); break
    case 'transfer': out.amount = String(transition.$amount); out.recipient = transition.recipientId; break
    case 'freeze': case 'unfreeze': case 'destroyFrozenFunds': out.frozenIdentityId = transition.frozenIdentityId; break
    case 'emergencyAction': out.emergencyAction = transition.emergencyAction; break
    default: unsupported(`new-version token ${transition.$action}`)
  }
  return out
}

const decodeProtocolStateTransition = async base64 => {
  const sdk = await loadDecoder()
  const state = sdk.StateTransition.fromBase64(base64)
  let typed
  try {
    const raw = Buffer.from(base64, 'base64').toString('hex')
    if (state.toHex() !== raw) throw new Error('State transition is not an exact canonical byte round-trip')
    const type = state.actionTypeNumber
    const classes = {
      0: 'DataContractCreateTransition',
      1: 'BatchTransition',
      4: 'DataContractUpdateTransition',
      5: 'IdentityUpdateTransition',
      21: 'ShieldFromIdentityTransition',
      22: 'IdentityTopUpFromShieldedPoolTransition',
      23: 'IdentityKeyLimitsUpdate',
      24: 'ContractUserModeration',
      25: 'ContractFeeClaim'
    }
    if (!classes[type] || !sdk[classes[type]]) unsupported(`state transition ${type}`)
    typed = sdk[classes[type]].fromStateTransition(state)
    const json = typed.toJSON()
    const out = {
      type,
      typeString: StateTransitionEnum[type],
      userFeeIncrease: state.userFeeIncrease,
      signature: Buffer.from(state.signature ?? []).toString('hex'),
      signaturePublicKeyId: state.signaturePublicKeyId,
      raw
    }
    if (type === 1) {
      out.ownerId = json.ownerId
      const wrappers = typed.transitions
      const objects = typed.toObject().transitions
      try {
        out.transitions = json.transitions.map((transition, index) => {
          const wrapper = wrappers[index].toTransition()
          try {
            if (transition.$transition === 'document') return documentTransition(transition, wrapper, objects[index])
            if (transition.$transition !== 'token') unsupported(`batch kind ${transition.$transition}`)
            return tokenTransition(transition, wrapper, json.ownerId)
          } finally { wrapper.free() }
        })
      } finally { wrappers.forEach(w => w.free()) }
    } else if (type === 0 || type === 4) {
      const contract = json.dataContract
      Object.assign(out, {
        internalConfig: withoutVersion(contract.config),
        version: contract.version,
        dataContractId: contract.id,
        ownerId: contract.ownerId,
        schema: contract.documentSchemas,
        tokens: Object.entries(contract.tokens ?? {}).map(([position, config]) => tokenConfig(sdk, contract.id, position, config)),
        groups: Object.entries(contract.groups ?? {}).map(([position, group]) => ({ position: Number(position), members: group.members, requiredPower: group.requiredPower })),
        contractGroup: json.contractGroup,
        contractGroupMemberships: json.contractGroupMemberships
      })
      if (type === 0) out.identityNonce = String(json.identityNonce)
      else {
        out.identityContractNonce = String(json['$identity-contract-nonce'])
        out.dataContractOwner = contract.ownerId
        out.groups = Object.fromEntries(Object.entries(contract.groups ?? {}).map(([position, group]) => [position, { members: group.members, requiredPower: group.requiredPower }]))
      }
    } else if (type === 5) {
      Object.assign(out, {
        identityNonce: String(json.nonce),
        identityId: json.identityId,
        revision: String(json.revision),
        publicKeyIdsToDisable: json.disablePublicKeys ?? [],
        publicKeysToAdd: (json.addPublicKeys ?? []).map(key => {
          const wrapped = sdk.IdentityPublicKeyInCreation.fromJSON(key)
          try {
            return {
              contractBounds: key.contractBounds ? { type: key.contractBounds.$type, id: key.contractBounds.id, typeName: key.contractBounds.documentTypeName } : null,
              id: key.id,
              type: wrapped.keyType,
              data: hex(key.data),
              publicKeyHash: Buffer.from(wrapped.getHash()).toString('hex'),
              purpose: wrapped.purpose,
              securityLevel: wrapped.securityLevel,
              readOnly: key.readOnly,
              signature: hex(key.signature),
              ...(key.$formatVersion === '1' ? { totalBudget: stringOrNull(key.totalBudget), expiresAt: stringOrNull(key.expiresAt) } : {})
            }
          } finally { wrapped.free() }
        })
      })
    } else if (type === 24 || type === 25) {
      Object.assign(out, { ownerId: json.ownerId, dataContractId: json.dataContractId, identityContractNonce: String(json.identityContractNonce) })
      if (type === 24) out.action = json.action
      else out.pot = json.pot // No payout amount is encoded; never fabricate one.
    } else {
      // No pre-existing Explorer schema for these top-level families. Retain all
      // named fields; integers/bytes in canonical JSON are lossless upstream.
      Object.assign(out, withoutVersion(json), { signature: out.signature, raw })
    }
    // Additive, documented escape hatch for *represented* upstream fields, not
    // a substitute for normalized mappings. Unknown mapped families fail above.
    out.protocolFields = json
    return out
  } finally {
    typed?.free()
    state.free()
  }
}

module.exports = { decodeProtocolStateTransition }
