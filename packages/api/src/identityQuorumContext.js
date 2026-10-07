const http = require('node:http')

const missingQuorumHash = error => {
  if (error?.kind !== 19) return null
  return /^context provider error: invalid quorum: Quorum not found in cache for hash: ([a-f0-9]{64})$/i.exec(error.message)?.[1].toLowerCase() ?? null
}

const validateCoreQuorum = (value, hash) => {
  if (value?.type !== 'llmq_devnet_platform' || value.quorumHash?.toLowerCase() !== hash || !/^[a-f0-9]{96}$/i.test(value.quorumPublicKey ?? '') || !Number.isSafeInteger(value.height) || value.height < 0 || !Array.isArray(value.members)) {
    throw new Error('Invalid historical devnet quorum response')
  }
  return { quorum_hash: hash, key: value.quorumPublicKey, height: value.height, valid_members_count: value.members.filter(member => member.valid === true).length }
}

const prefetchLocalContext = async (m, current, previous) => {
  const server = http.createServer((request, response) => {
    const body = request.method === 'GET' && ({ '/quorums': current, '/previous': previous })[request.url]
    response.writeHead(body ? 200 : 404, { 'Content-Type': 'application/json', Connection: 'close' })
    response.end(JSON.stringify(body ?? { error: 'Not found' }))
  })
  let timer
  let timedOut = false
  try {
    await new Promise((resolve, reject) => {
      server.once('error', reject)
      server.listen(0, '127.0.0.1', resolve)
    })
    return await Promise.race([
      m.WasmTrustedContext.prefetchDevnetWithUrl(`http://127.0.0.1:${server.address().port}`, false).then(context => {
        if (timedOut) context.free()
        return context
      }),
      new Promise((resolve, reject) => {
        timer = setTimeout(() => { timedOut = true; reject(new Error('Historical quorum context import timed out')) }, 10000)
      })
    ])
  } finally {
    clearTimeout(timer)
    server.closeAllConnections()
    if (server.listening) await new Promise(resolve => server.close(resolve))
  }
}

const createQuorumBackfill = (devnet, fetchFn = fetch) => {
  // Retain at most8 previously Core-verified keys; never evict public SDK entries.
  const historical = new Map()
  if (!/^[a-z0-9]+(?:-[a-z0-9]+)*$/i.test(devnet) || ['mainnet', 'testnet', 'devnet', 'local', 'regtest'].includes(devnet.toLowerCase())) throw new Error('Invalid devnet for historical quorum context')
  const json = async url => {
    const response = await fetchFn(url, { signal: AbortSignal.timeout(8000), redirect: 'error' })
    if (!response.ok) throw new Error(`Historical quorum context HTTP ${response.status}`)
    return response.json()
  }
  return async (m, hash) => {
    if (!/^[a-f0-9]{64}$/.test(hash)) throw new Error('Invalid missing quorum hash')
    const base = `https://quorums.${devnet}.networks.dash.org`
    const [current, previous, core] = await Promise.all([
      json(`${base}/quorums`), json(`${base}/previous`),
      json(`http://127.0.0.1:3005/quorum/info?quorumType=107&quorumHash=${hash}`)
    ])
    if (current?.success !== true || previous?.success !== true || !Array.isArray(current.data) || !Array.isArray(previous.data?.quorums)) throw new Error('Invalid official quorum context response')
    const entry = validateCoreQuorum(core, hash)
    historical.set(hash, entry)
    if (historical.size > 8) historical.delete(historical.keys().next().value)
    const merged = [...current.data]
    for (const row of historical.values()) {
      for (const existing of previous.data.quorums.filter(item => item.quorum_hash?.toLowerCase() === row.quorum_hash)) {
        if (existing.key?.toLowerCase() !== row.key.toLowerCase()) throw new Error('Conflicting trusted quorum key')
      }
      const existing = merged.find(item => item.quorum_hash?.toLowerCase() === row.quorum_hash)
      if (existing && existing.key?.toLowerCase() !== row.key.toLowerCase()) throw new Error('Conflicting trusted quorum key')
      if (!existing) merged.push(row)
    }
    // The upstream WASM context has100-entry per-list caches; refuse eviction.
    if (merged.length > 100 || previous.data.quorums.length > 100) throw new Error('Historical quorum context exceeds SDK cache capacity')
    return prefetchLocalContext(m, { ...current, data: merged }, previous)
  }
}

module.exports = { createQuorumBackfill, missingQuorumHash, validateCoreQuorum, prefetchLocalContext }
