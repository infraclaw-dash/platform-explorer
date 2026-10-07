// Explorer-owned presentation/query adapter; published SDK and proofs are unchanged.
const { DPNS_CONTRACT } = require('./constants')
const { convertToHomographSafeChars } = require('./utils')

const readAliasInfo = async (sdk, aliasText) => {
  const [label, domain] = aliasText.split('.')
  const normalizedLabel = convertToHomographSafeChars(label ?? '')
  if (!/^[a-zA-Z01-]{3,19}$/.test(normalizedLabel)) return { alias: aliasText, contestedState: null }
  const state = await sdk.getContestedResourceVoteState({
    dataContractId: DPNS_CONTRACT,
    documentTypeName: 'domain',
    indexName: 'parentNameAndLabel',
    indexValues: [domain, normalizedLabel],
    resultType: 'documentsAndVoteTally',
    includeLockedAndAbstaining: false
  })
  const contenders = state.contenders
  let winner
  let winnerId
  try {
    // Match the legacy query's proven-empty semantics; errors still propagate.
    if (contenders.length === 0) return { alias: aliasText, contestedState: null, unknownState: true }
    winner = state.winner
    winnerId = winner?.identityId
    const bytes = winnerId?.toBytes()
    return {
      alias: aliasText,
      contestedState: {
        finishedVoteInfo: winner
          ? { wonByIdentityId: bytes ? { bytes: () => bytes } : undefined }
          : undefined
      }
    }
  } finally {
    winnerId?.free()
    winner?.free()
    contenders.forEach(contender => contender.free())
    state.free()
  }
}

const createIdentityReader = ({ devnet, addresses, loadModule, now = Date.now }) => {
  let modulePromise
  let contextPromise
  let contextExpires = 0
  let context

  const trustedContext = async m => {
    if (context && now() < contextExpires) return context
    if (!contextPromise) {
      contextPromise = m.WasmTrustedContext.prefetchDevnet(devnet, false).then(next => {
        // Builders retain their own Arc; freeing this wrapper does not invalidate readers.
        context?.free()
        context = next
        contextExpires = now() + 60000
        return context
      }).finally(() => { contextPromise = undefined })
    }
    return contextPromise
  }

  return async (identifier, aliases = []) => {
    modulePromise ??= loadModule()
    const m = await modulePromise
    const sdk = m.WasmSdkBuilder.withAddresses(addresses, 'devnet')
      .withVersion(14).withTrustedContext(await trustedContext(m)).withProofs(true)
      .withSettings(10000, 15000, 1, false).build()
    let keys
    let identity
    try {
      keys = await sdk.getIdentityKeys({ identityId: identifier, request: { type: 'all' } })
      identity = await sdk.getIdentity(identifier)
      if (!identity) throw new Error(`Identity with identifier ${identifier} not found`)
      const nonce = await sdk.getIdentityNonce(identifier)
      const aliasInfo = []
      // Bound concurrent RPC work even for identities with many aliases.
      for (const alias of aliases) aliasInfo.push(await readAliasInfo(sdk, alias))
      return {
        identityInfo: { balance: identity.balance, revision: identity.revision },
        // Match dash-platform-sdk's public API: strip the stored nonce's high bits.
        nonce: (nonce ?? 0n) & 0xFFFFFFFFFFn,
        publicKeys: keys.map(formatKey),
        aliasInfo
      }
    } finally {
      keys?.forEach(key => key.free())
      identity?.free()
      sdk.free()
    }
  }
}

const formatKey = key => {
  const json = key.toJSON()
  const bounds = key.contractBounds
  const boundIdentifier = bounds?.identifier
  try {
    return {
      keyId: key.keyId,
      keyType: key.keyType,
      raw: key.hex(),
      data: key.data,
      purpose: key.purpose,
      securityLevel: key.securityLevel,
      readOnly: key.isReadOnly,
      publicKeyHash: key.getPublicKeyHash(),
      contractBounds: bounds
        ? { identifier: boundIdentifier.toBase58(), documentTypeName: bounds.documentTypeName ?? null }
        : null,
      disabledAt: key.disabledAt?.toString() ?? null,
      totalBudget: key.totalBudget?.toString() ?? null,
      expiresAt: key.expiresAt?.toString() ?? null,
      protocolFields: json
    }
  } finally {
    boundIdentifier?.free()
    bounds?.free()
  }
}

let reader
const readVerifiedIdentity = async (identifier, aliases = []) => {
  const devnet = process.env.PROTOCOL_IDENTITY_DEVNET
  if (!devnet) return null
  if (!process.env.DAPI_URL) throw new Error('DAPI_URL is required for verified devnet identity reads')
  reader ??= createIdentityReader({
    devnet,
    addresses: process.env.DAPI_URL.split(','),
    loadModule: async () => {
      const m = await import('@dashevo/wasm-sdk')
      await m.default()
      return m
    }
  })
  return reader(identifier, aliases)
}

module.exports = { createIdentityReader, readVerifiedIdentity, formatKey, readAliasInfo }
