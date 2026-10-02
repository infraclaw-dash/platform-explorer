# Protocol-14 decoding adapter — bounded validation, not rollout approval

`protocolDecoder.js` is Explorer presentation code, not a Platform/pshenmic library
patch. It uses the **unchanged published `@dashevo/wasm-sdk@5.0.0-beta.1`**, pinned
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
- Network/proof-backed SDK queries still use `dash-platform-sdk` and its legacy
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
