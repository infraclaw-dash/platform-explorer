//! Opt-in evidence gates. These tests only write to the explicitly named,
//! disposable replay database; they cannot target Sakura's database.
use crate::decoder::decoder::StateTransitionDecoder;
use crate::entities::block::Block;
use crate::entities::block_header::BlockHeader;
use crate::entities::validator::Validator;
use crate::models::{
    TenderdashRPCBlockResponse, TenderdashRPCBlockResultsResponse, TenderdashRPCValidatorsResponse,
    TransactionResult, TransactionStatus,
};
use crate::processor::psql::{PSQLProcessor, PostgresDAO};
use base64::{engine::general_purpose::STANDARD, Engine};
use dashcore_rpc::{Auth, Client};
use dpp::dashcore::Network;
use dpp::identity::state_transition::AssetLockProved;
use dpp::prelude::AssetLockProof;
use dpp::serialization::PlatformSerializable;
use dpp::state_transition::StateTransition;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader};

fn snapshot() -> impl Iterator<Item = serde_json::Value> {
    let path = std::env::var("PE_REPLAY_FIXTURE").expect("PE_REPLAY_FIXTURE is required");
    BufReader::new(std::fs::File::open(path).unwrap())
        .lines()
        .map(|line| serde_json::from_str(&line.unwrap()).unwrap())
}

#[tokio::test]
#[ignore = "requires a complete read-only chain snapshot; no DB writes"]
async fn decode_snapshot_without_skipping() {
    let decoder = StateTransitionDecoder::new();
    let mut types = BTreeMap::<String, usize>::new();
    let mut outcomes = BTreeMap::<String, usize>::new();
    let mut core_txids = BTreeSet::new();
    let mut pro_tx_hashes = BTreeSet::new();
    let mut blocks = 0;
    let mut transactions = 0;
    let mut complete = false;
    for item in snapshot() {
        match item["kind"].as_str().unwrap() {
            "manifest" => {
                assert_eq!(item["chain"], "dash-devnet-sakura");
                assert_eq!(item["appVersion"]["app"], "14");
            }
            "block" => {
                blocks += 1;
                assert_eq!(item["height"], blocks);
                for v in item["validators"]["validators"].as_array().unwrap() {
                    pro_tx_hashes.insert(v["pro_tx_hash"].as_str().unwrap().to_owned());
                }
                for (index, tx) in item["block"]["block"]["data"]["txs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .enumerate()
                {
                    transactions += 1;
                    let bytes = STANDARD.decode(tx.as_str().unwrap()).unwrap();
                    let st = decoder
                        .decode(bytes.clone())
                        .await
                        .unwrap_or_else(|e| panic!("block {blocks} tx {transactions}: {e}"));
                    assert_eq!(
                        st.serialize_to_bytes().unwrap(),
                        bytes,
                        "wire data must be retained unchanged"
                    );
                    *types
                        .entry(format!("{:?}", st.state_transition_type()))
                        .or_default() += 1;
                    let code = item["results"]["txs_results"][index]["code"]
                        .as_u64()
                        .unwrap_or(0);
                    *outcomes
                        .entry(format!(
                            "{:?}:{}",
                            st.state_transition_type(),
                            if code == 0 { "SUCCESS" } else { "FAIL" }
                        ))
                        .or_default() += 1;
                    let proof = match &st {
                        StateTransition::IdentityCreate(st) => Some(st.asset_lock_proof()),
                        StateTransition::IdentityTopUp(st) => Some(st.asset_lock_proof()),
                        StateTransition::AddressFundingFromAssetLock(st) => {
                            Some(st.asset_lock_proof())
                        }
                        StateTransition::ShieldFromAssetLock(st) => Some(st.asset_lock_proof()),
                        _ => None,
                    };
                    if let Some(AssetLockProof::Chain(chain)) = proof {
                        core_txids.insert(chain.out_point.txid.to_string());
                    }
                }
            }
            "complete" => {
                assert_eq!(item["tip"], blocks);
                assert_eq!(item["transactions"], transactions);
                complete = true;
            }
            other => panic!("unexpected snapshot entry {other}"),
        }
    }
    assert!(
        complete,
        "partial snapshots cannot establish through-tip compatibility"
    );
    let evidence = serde_json::json!({"blocks": blocks, "transactions": transactions, "transitionTypes":types, "outcomes":outcomes, "coreTxids": core_txids, "proTxHashes":pro_tx_hashes});
    std::fs::write(
        std::env::var("PE_DECODE_EVIDENCE").unwrap(),
        format!("{evidence:#}\n"),
    )
    .unwrap();
    println!("Decoded {transactions} transactions across all {blocks} blocks without skipping");
}

#[tokio::test]
#[ignore = "requires the named disposable replay DB and complete snapshot"]
async fn replay_snapshot_to_isolated_db_without_skipping() {
    assert_eq!(
        std::env::var("POSTGRES_HOST").unwrap(),
        "pe-sakura-replay-db"
    );
    assert_eq!(std::env::var("POSTGRES_DB").unwrap(), "pe_sakura_replay");
    assert_eq!(
        std::env::var("PE_REPLAY_CORE_URL").unwrap(),
        "http://pe-sakura-replay-core:8000"
    );
    let dao = PostgresDAO::new(Network::Testnet);
    let mut client = dao.connection_pool.get().await.unwrap();
    crate::embedded::migrations::runner()
        .run_async(&mut **client)
        .await
        .unwrap();
    let existing: i64 = client
        .query_one("SELECT COUNT(*) FROM blocks", &[])
        .await
        .unwrap()
        .get(0);
    let retained = std::env::var("PE_REPLAY_RETAINED").as_deref() == Ok("1");
    if retained {
        assert!(existing > 0, "retained-state gate needs a previous replay");
    } else {
        assert_eq!(
            existing, 0,
            "fresh replay refuses to overwrite retained state"
        );
    }
    let core = Client::new(&std::env::var("PE_REPLAY_CORE_URL").unwrap(), Auth::None).unwrap();
    let processor = PSQLProcessor::new(core, Network::Testnet);
    let mut blocks = 0i32;
    let mut transactions = 0i64;
    let mut complete = false;
    for item in snapshot() {
        match item["kind"].as_str().unwrap() {
            "manifest" => {
                assert_eq!(item["chain"], "dash-devnet-sakura");
                assert_eq!(item["appVersion"]["app"], "14");
            }
            "block" => {
                blocks += 1;
                assert_eq!(item["height"], blocks);
                let response: TenderdashRPCBlockResponse =
                    serde_json::from_value(item["block"].clone()).unwrap();
                let results: TenderdashRPCBlockResultsResponse =
                    serde_json::from_value(item["results"].clone()).unwrap();
                let validators: TenderdashRPCValidatorsResponse =
                    serde_json::from_value(item["validators"].clone()).unwrap();
                let quorum_hash = validators.quorum_hash.clone();
                let validators = Vec::<Validator>::try_from(validators).unwrap();
                let tx_results = results.txs_results.unwrap_or_default();
                assert_eq!(response.block.data.txs.len(), tx_results.len());
                let original_txs = response.block.data.txs.clone();
                let txs = original_txs
                    .iter()
                    .zip(tx_results)
                    .map(|(data, result)| TransactionResult {
                        data: data.clone(),
                        gas_used: result.gas_used,
                        status: if result.code.unwrap_or(0) == 0 {
                            TransactionStatus::SUCCESS
                        } else {
                            TransactionStatus::FAIL
                        },
                        error: result.info,
                    })
                    .collect::<Vec<_>>();
                let h = response.block.header;
                let block = Block {
                    header: BlockHeader {
                        hash: response.block_id.hash,
                        height: blocks,
                        timestamp: h.timestamp,
                        block_version: h.version.block.parse().unwrap(),
                        app_version: h.version.app.parse().unwrap(),
                        l1_locked_height: h.core_chain_locked_height,
                        app_hash: h.app_hash,
                        proposer_pro_tx_hash: h.proposer_pro_tx_hash,
                        quorum_hash: Some(quorum_hash),
                    },
                    txs,
                };
                if blocks == 325 && !retained {
                    let mut invalid = Block {
                        header: block.header.clone(),
                        txs: block.txs.clone(),
                    };
                    invalid.txs.push(TransactionResult {
                        data: "!invalid".into(),
                        gas_used: 0,
                        status: TransactionStatus::SUCCESS,
                        error: None,
                    });
                    let error = processor
                        .handle_block(invalid, validators.clone())
                        .await
                        .unwrap_err();
                    assert!(matches!(
                        error,
                        crate::processor::psql::ProcessorError::TransactionError {
                            height: 325,
                            index: 1,
                            stage: "base64 decode",
                            ..
                        }
                    ));
                    let absent: i64 = client
                        .query_one("SELECT COUNT(*) FROM blocks WHERE height = 325", &[])
                        .await
                        .unwrap()
                        .get(0);
                    let tx_absent: i64 = client
                        .query_one(
                            "SELECT COUNT(*) FROM state_transitions WHERE block_height = 325",
                            &[],
                        )
                        .await
                        .unwrap()
                        .get(0);
                    assert_eq!(
                        (absent, tx_absent),
                        (0, 0),
                        "malformed later tx must not partially commit block 325"
                    );
                }
                processor
                    .handle_block(block, validators)
                    .await
                    .unwrap_or_else(|e| panic!("replay failed at block {blocks}: {e:?}"));
                let persisted = client
                    .query(
                        "SELECT data FROM state_transitions WHERE block_height = $1 ORDER BY index",
                        &[&blocks],
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    persisted.len(),
                    original_txs.len(),
                    "block {blocks}: no transaction can be skipped"
                );
                for (row, original) in persisted.into_iter().zip(original_txs) {
                    assert_eq!(
                        row.get::<_, String>(0),
                        original,
                        "block {blocks}: exact original bytes must survive indexing"
                    );
                    transactions += 1;
                }
            }
            "complete" => {
                assert_eq!(item["tip"], blocks);
                assert_eq!(item["transactions"], transactions);
                complete = true;
            }
            other => panic!("unexpected snapshot entry {other}"),
        }
    }
    assert!(complete);
    let count: i64 = client
        .query_one("SELECT COUNT(*) FROM state_transitions", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(count, transactions);
    let highest: i32 = client
        .query_one("SELECT MAX(height) FROM blocks", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(highest, blocks);
    println!("REPLAY VERIFIED: {blocks} contiguous blocks, {transactions} exact transactions, zero skips; database retained");
}

#[tokio::test]
#[ignore = "requires retained disposable replay DB at block 702 and captured block 703"]
async fn unsupported_real_moderation_block_does_not_commit_or_lose_retained_data() {
    assert_eq!(
        std::env::var("POSTGRES_HOST").unwrap(),
        "pe-sakura-replay-db"
    );
    assert_eq!(std::env::var("POSTGRES_DB").unwrap(), "pe_sakura_replay");
    assert_eq!(
        std::env::var("PE_REPLAY_CORE_URL").unwrap(),
        "http://pe-sakura-replay-core:8000"
    );
    let dao = PostgresDAO::new(Network::Testnet);
    let client = dao.connection_pool.get().await.unwrap();
    let before = client.query_one("SELECT MAX(height), (SELECT count(*) FROM blocks), (SELECT count(*) FROM state_transitions), (SELECT count(*) FROM documents), (SELECT count(*) FROM data_contracts) FROM blocks", &[]).await.unwrap();
    assert_eq!(before.get::<_, i32>(0), 702);
    let counts = (
        before.get::<_, i64>(1),
        before.get::<_, i64>(2),
        before.get::<_, i64>(3),
        before.get::<_, i64>(4),
    );
    let item = snapshot()
        .find(|item| item["kind"] == "block" && item["height"] == 703)
        .expect("actual moderation block required");
    let response: TenderdashRPCBlockResponse =
        serde_json::from_value(item["block"].clone()).unwrap();
    let results: TenderdashRPCBlockResultsResponse =
        serde_json::from_value(item["results"].clone()).unwrap();
    let validators: TenderdashRPCValidatorsResponse =
        serde_json::from_value(item["validators"].clone()).unwrap();
    let quorum = validators.quorum_hash.clone();
    let txs = response
        .block
        .data
        .txs
        .into_iter()
        .zip(results.txs_results.unwrap())
        .map(|(data, r)| TransactionResult {
            data,
            gas_used: r.gas_used,
            status: if r.code.unwrap_or(0) == 0 {
                TransactionStatus::SUCCESS
            } else {
                TransactionStatus::FAIL
            },
            error: r.info,
        })
        .collect();
    let h = response.block.header;
    let block = Block {
        header: BlockHeader {
            hash: response.block_id.hash,
            height: 703,
            timestamp: h.timestamp,
            block_version: h.version.block.parse().unwrap(),
            app_version: h.version.app.parse().unwrap(),
            l1_locked_height: h.core_chain_locked_height,
            app_hash: h.app_hash,
            proposer_pro_tx_hash: h.proposer_pro_tx_hash,
            quorum_hash: Some(quorum),
        },
        txs,
    };
    let core = Client::new(&std::env::var("PE_REPLAY_CORE_URL").unwrap(), Auth::None).unwrap();
    let processor = PSQLProcessor::new(core, Network::Testnet);
    let error = processor
        .handle_block(block, Vec::<Validator>::try_from(validators).unwrap())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        crate::processor::psql::ProcessorError::TransactionError {
            height: 703,
            index: 0,
            stage: "handler compatibility",
            ..
        }
    ));
    let after = client.query_one("SELECT MAX(height), (SELECT count(*) FROM blocks), (SELECT count(*) FROM state_transitions), (SELECT count(*) FROM documents), (SELECT count(*) FROM data_contracts) FROM blocks", &[]).await.unwrap();
    assert_eq!(after.get::<_, i32>(0), 702);
    assert_eq!(
        counts,
        (
            after.get::<_, i64>(1),
            after.get::<_, i64>(2),
            after.get::<_, i64>(3),
            after.get::<_, i64>(4)
        )
    );
    let absent: i64 = client
        .query_one(
            "SELECT count(*) FROM state_transitions WHERE block_height=703",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        absent, 0,
        "unhandled successful transitions must neither be skipped nor partially committed"
    );
    println!("Verified real block 703 fail-closed rollback; all retained block/transaction/document/contract counts unchanged");
}
