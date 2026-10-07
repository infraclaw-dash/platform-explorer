# Protocol-14 decoding adapter — bounded validation, not rollout approval

`protocolDecoder.js` is Explorer presentation code, not a Platform/pshenmic library
patch. It uses the **unchanged published `@dashevo/wasm-sdk@5.0.0-beta.2`**, pinned
exactly in `package.json` and by registry integrity in the API's npm lockfile.
The API Dockerfile consumes that lockfile with `npm ci`. The monorepo's Yarn
lockfile is unchanged; install/build this API using its npm lockfile.

Only failure of the existing binary decoder selects this adapter. Existing
successful decodes retain their response shape and formatting path. Presentation
errors do not trigger a second decoder. The adapter requires exact binary
round-trip and refuses unknown presentation variants instead of reporting raw-only
success. The original `raw` field is unchanged.

## Response contract

- Existing names, identifier encodings, string nonces/amounts, hex signatures and
  entropy, and typed-array document properties are preserved. New document data
  integers are emitted as numbers only within the safe integer range; larger
  values are decimal strings rather than silently rounded values.
- Document V2 exposes `actionFeeAgreement`. Index-only deletion is
  `DOCUMENT_INDEX_ONLY_DELETE` (appended enum ID **17**; existing token IDs do not
  move) and retains the complete property tuple in `data`.
- Moderation exposes `ownerId`, `dataContractId`, string `identityContractNonce`
  and the full tagged `action`, including its reason and document/team fields.
- Fee claims expose the selected `pot`. **No amount is fabricated**: the
  resulting payout is not encoded in the signed transition.
- New contract configurations, contract groups, distribution rules and
  budget/expiry key fields are retained. `totalBudget` and `expiresAt` are strings.
- Every adapter response adds `protocolFields`: the complete, version-tagged
  upstream canonical JSON. Its byte arrays are base64 and identifiers base58;
  integers follow the upstream number-or-string JSON representation. This is an
  additive lossless record, **not** a replacement for the normalized fields above.
  In particular, V1 distribution controls and optional future fields must not
  vanish just because older Explorer models have no slot for them.
- Existing chain-lock top-up formatting now returns `amount: null`, matching its
  existing `assetLockProof.fundingAmount: null`. A chain proof contains an
  outpoint, not the referenced output amount; the previous formatter threw on
  that case. This endpoint does not fetch or invent that amount.

## Local gates

Run `npm run test:unit` and focused Standard lint. Actual public Sakura fixtures
include block 325 V2, 703 moderation, 950 fee claim, index-only deletion, contract
V1, once-per-identity distribution, budgeted key, chain top-up and nested binary /
integer properties. A local Fastify injection exercises controller serialization;
no network connection or database is used.

For the retained immutable capture:

```sh
EXPLORER_PROTOCOL_SNAPSHOT=/absolute/path/to/snapshot-checkpoint.ndjson npm run test:protocol-corpus
```

The gate verifies contiguous captured blocks, original transaction bytes,
JSON-serializable API results, and all existing fields for compatible old-decoder
cases. It reports the snapshot digest and clearly labels the captured prefix.
This does not prove all possible protocol variants or historical projections.

## Remaining gates / boundaries

- Broadcast and verify controllers still use `pshenmic-dpp`. They are unchanged
  and reject newer wire formats. No broadcast or verification RPC was executed.
- With explicit `PROTOCOL_IDENTITY_DEVNET`, identity-detail keys/balance/revision/
  nonce and contested-alias enrichment use the published modern SDK with proofs enabled, explicit devnet and
  protocol14, and its official devnet trusted quorum context (60-second cache).
  Otherwise that path is unchanged. Actual Sakura19key regression includes V1
  budget/expiry and18 old keys with unchanged legacy fields. Other
  network/proof-backed SDK queries still use `dash-platform-sdk` and its legacy
  proof/contract decoder. Contract/token/identity query compatibility is **not
  established**. The shared state-transition helper used for token price data is
  repaired, but this is not proof that the whole token endpoint works.
- No full-tip API check, complete historical SQL replay, retained-state catch-up,
  image build, publish or live rollout is established by these tests. A bounded
  prefix is not an authorization to deploy.
- New top-level families 21–23 and unseen new-version token actions/configuration
  combinations are not covered by the actual corpus regression. Unmapped token
  presentations fail explicitly. Do not generalize the prefix result to them.

All product source, versions, protocol settings, chain state and databases stay
unchanged. No skip/reset/cleanup workaround is introduced.

## Sakura beta2 captured-tail extension

The npm manifest/lock change is only beta1→beta2 of the published SDK (no
transitive dependencies). Actual successful block7392 TokenConfigurationV1 now
decodes, preserving normalized pool enablement/minimum-note controls and complete
canonical rules. Actual type4 contract updates are mapped with contract identity,
version, schemas, nonce, tokens and groups. Source tests include both successful
and failed transactions; presentation does not invent a successful chain result.
Batch display enum IDs18–24 are appended without renumbering existing IDs.
These names do not claim support for unseen batch wire variants; unsupported
presentations still fail explicitly. A read-only corpus check of116 captured
transactions from7392–7505 passes exact raw preservation and JSON serialization;
this is a bounded historical corpus, not a perpetual/full-tip compatibility claim.

Contested aliases retain the existing homograph classification and status mapper.
The modern query requests `documentsAndVoteTally`, verifies the original proof,
and passes the winner/absence state to the existing mapper; it does not need to
deserialize contender documents merely to determine a vote winner. Proven-empty
state remains `unknown`; pending/won/locked semantics are unchanged and RPC/proof
errors propagate. Alias requests are sequential to bound RPC concurrency.

Historical proof quorums can outlive both official four-entry recent lists. Only
an exact typed quorum-cache-miss may fetch the missing hash from the existing
loopback Core-backed quorum endpoint (devnet Platform type107). Hash/type/key are
strictly checked and conflicts with either official list fail closed. The
unchanged published SDK imports the original official lists plus that verified
historical key through its documented custom-URL factory; an ephemeral
127.0.0.1 random-port feed closes in `finally`, with a10-second import bound.
Upstream HTTP requests have8-second limits. At most two distinct missing hashes
are filled per identity read, followed by proof-enabled retries; repeated misses
and all other proof/network errors remain errors. No public route or service
configuration changes, custom proof implementation or SDK patch are introduced.
