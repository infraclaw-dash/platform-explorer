// Explorer-owned presentation/query adapter; published SDK and proofs are unchanged.
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

  return async identifier => {
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
      return {
        identityInfo: { balance: identity.balance, revision: identity.revision },
        nonce: nonce ?? 0n,
        publicKeys: keys.map(formatKey)
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
        ? { identifier: boundIdentifier.base58(), documentTypeName: bounds.documentTypeName ?? null }
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
const readVerifiedIdentity = async identifier => {
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
  return reader(identifier)
}

module.exports = { createIdentityReader, readVerifiedIdentity, formatKey }
