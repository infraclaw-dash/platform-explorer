# Sakura protocol-14 candidate — compatibility remains blocked

This branch is a **partial repair**, not a network-compatible release.

- Upstream 2.5.3/master `c1657220e1d683067788568ca557b525f599f735` pins Platform v4.1.0.
- This candidate pins Platform v5.0.0-beta.1 source `b95849a1767aa26fa794cf69a44d4fc107ee6528` and its matching rust-dashcore revision.
- Minimum Rust is 1.98. Ordinary `cargo test --locked` includes actual block 325, exact-byte round-trip, the production document conversion, error context, catalog and append-only discriminant checks.
- The two block-325 fixtures are public read-only Tenderdash evidence. Unknown successful transition families fail closed; no catch-all handler skips or advances them.

Opt-in replay tests in `src/replay_tests.rs` require explicit fixture paths. Database writers require exactly `pe-sakura-replay-db`, database `pe_sakura_replay`, and read-only fixture Core URL `http://pe-sakura-replay-core:8000`. Use an isolated Docker `--internal` network, no published ports and a new task-owned Postgres volume. Never point these gates at live Sakura. Initialization only inserts missing pinned system-contract catalog entries and does not replace existing rows.

```sh
PE_REPLAY_FIXTURE=/build/snapshot.ndjson PE_DECODE_EVIDENCE=/build/decode.json \
  cargo test --locked decode_snapshot_without_skipping -- --ignored --nocapture
PE_REPLAY_FIXTURE=/build/snapshot.ndjson \
  cargo test --locked replay_snapshot_to_isolated_db_without_skipping -- --ignored --nocapture
```

`PE_REPLAY_RETAINED=1` verifies/resumes retained task-owned state; it does not drop or reset a database. Each indexed block must retain every original transaction byte-for-byte. A separate real-block-703 gate proves unsupported moderation returns a contextual error and leaves all retained counts unchanged.

## Current evidence and blockers

- Eight ordinary Rust tests pass.
- All 26,661 transitions across captured blocks 1–8915 decode and reserialize identically.
- Actual database replay verified 702 contiguous blocks and 534 exact transactions, including block 325; retained-state verification passed.
- First unresolved successful transaction is block 703, `ContractUserModeration` (type 24). Complete moderation/document/approval projections and fee-claim outcomes must be implemented, not discarded. The captured prefix contains 213 successful moderation transitions and 3 successful fee claims.
- API `pshenmic-dpp@2.0.0-dev.25` and newest dev.29 both reject the real V2 fixture. Fix and independently validate the API decoder/handlers before marking the whole Explorer compatible.
- Fixed observed network tip was 26072. Read-only capture hit its 900-second bound at block 8915, so through-network-tip replay is **not established**.

Only after those gates pass: publish a tested image, record its OCI digest and exact source/protocol evidence, then let the parent integrate that digest. Preserve and back up the live DB, verify restart/catch-up/idempotence, and do not use a transaction skip list or network/database reset.

The Dockerfile pins Linux/amd64 Rust and Debian images; its complete image build is a separate gate, not implied by local Rust compilation.
