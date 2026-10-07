//! Explorer projections of successful moderation. Consensus validation is not
//! repeated here. A proposal/approval is an event, not evidence of deletion;
//! retain it with an explicitly unavailable outcome instead of inventing effects.
use super::super::{PSQLProcessor, ProcessorError};
use base64::{engine::general_purpose::STANDARD, Engine};
use deadpool_postgres::Transaction;
use dpp::data_contract::accessors::v0::DataContractV0Getters;
use dpp::document::serialization_traits::DocumentPlatformConversionMethodsV0;
use dpp::document::DocumentV0Getters;
use dpp::platform_value::string_encoding::Encoding::Base58;
use dpp::prelude::Identifier;
use dpp::state_transition::contract_user_moderation_transition::{
    ContractUserModerationAction as Action, ContractUserModerationTransition,
};
use dpp::state_transition::StateTransition;
use serde_json::{json, Value};

fn projection_outcome(action: &Action) -> Value {
    match action {
        Action::DeleteSettledDocument { .. } | Action::ApproveTeamAction { .. } => json!({
            "status": "unavailable",
            "documentDeleted": null,
            "reason": "Successful proposal or approval does not establish the team action outcome"
        }),
        _ => json!({"status": "projected"}),
    }
}

// Explicit Explorer JSON mapping: do not alter DPP feature flags or libraries
// merely to obtain a serde representation for the upstream enum.
fn action_json(action: &Action) -> Value {
    match action {
        Action::Ban {
            identity_id,
            reason,
        } => json!({"type":"ban","identityId":identity_id.to_string(Base58),"reason":reason}),
        Action::Unban { identity_id } => {
            json!({"type":"unban","identityId":identity_id.to_string(Base58)})
        }
        Action::Suspend {
            identity_id,
            until,
            reason,
        } => {
            json!({"type":"suspend","identityId":identity_id.to_string(Base58),"until":until,"reason":reason})
        }
        Action::Unsuspend { identity_id } => {
            json!({"type":"unsuspend","identityId":identity_id.to_string(Base58)})
        }
        Action::Warn {
            identity_id,
            reason,
        } => json!({"type":"warn","identityId":identity_id.to_string(Base58),"reason":reason}),
        Action::ClearWarnings { identity_id } => {
            json!({"type":"clearWarnings","identityId":identity_id.to_string(Base58)})
        }
        Action::DeleteDocument {
            document_type_name,
            document_id,
            reason,
        } => {
            json!({"type":"deleteDocument","documentTypeName":document_type_name,"documentId":document_id.to_string(Base58),"reason":reason})
        }
        Action::RestoreDocument {
            document_type_name,
            document,
        } => {
            json!({"type":"restoreDocument","documentTypeName":document_type_name,"documentBase64":STANDARD.encode(document.as_slice())})
        }
        Action::ChangeDocumentFields {
            document_type_name,
            document_id,
            fields,
            reason,
        } => {
            json!({"type":"changeDocumentFields","documentTypeName":document_type_name,"documentId":document_id.to_string(Base58),"fields":fields,"reason":reason})
        }
        Action::DeleteSettledDocument {
            document_type_name,
            document_id,
            reason,
        } => {
            json!({"type":"deleteSettledDocument","documentTypeName":document_type_name,"documentId":document_id.to_string(Base58),"reason":reason})
        }
        Action::ApproveTeamAction { action_id } => {
            json!({"type":"approveTeamAction","actionId":action_id.to_string(Base58)})
        }
    }
}

// Pure projection shared by production and fixture tests. Timestamps use block
// time, never wall-clock time; a suspension remains explicit after expiration.
fn identity_effect(
    previous: Value,
    action: &Action,
    timestamp: i64,
    moderator: &str,
) -> Result<Value, String> {
    let mut state = previous
        .as_object()
        .cloned()
        .ok_or("identity moderation state is not an object")?;
    let entry = |reason: &dpp::data_contract::config::moderation::ContractModerationReason| json!({"reason": reason, "timestampMs": timestamp, "moderator": moderator});
    match action {
        Action::Ban { reason, .. } => {
            state.insert("ban".into(), entry(reason));
            state.remove("suspension");
        }
        Action::Unban { .. } => {
            state.remove("ban");
        }
        Action::Suspend { until, reason, .. } => {
            let mut value = entry(reason);
            value["until"] = json!(until);
            state.insert("suspension".into(), value);
        }
        Action::Unsuspend { .. } => {
            state.remove("suspension");
        }
        Action::Warn { reason, .. } => {
            let warnings = state.entry("warnings".to_string()).or_insert(json!([]));
            warnings
                .as_array_mut()
                .ok_or("warnings is not an array")?
                .push(entry(reason));
        }
        Action::ClearWarnings { .. } => {
            state.remove("warnings");
        }
        _ => return Err("not an identity moderation action".into()),
    }
    Ok(Value::Object(state))
}

fn changed_properties(
    previous: Value,
    fields: &std::collections::BTreeMap<String, dpp::platform_value::Value>,
) -> Result<Value, String> {
    let mut data = previous
        .as_object()
        .cloned()
        .ok_or("document properties are not an object")?;
    for (name, value) in fields {
        if matches!(value, dpp::platform_value::Value::Null) {
            data.remove(name);
        } else {
            data.insert(
                name.clone(),
                serde_json::to_value(value).map_err(|e| e.to_string())?,
            );
        }
    }
    Ok(Value::Object(data))
}

// A restore needs the target document's byte layout, not the indexes of every
// other document type in its contract. Retain its complete schema, shared
// definitions, config and tokens; only narrow this temporary decoder input.
// The original contract transition and full Explorer schema remain untouched.
fn restore_document_contract(
    mut format: dpp::data_contract::serialized_version::DataContractInSerializationFormat,
    document_type_name: &str,
    version: &dpp::version::PlatformVersion,
) -> Result<dpp::data_contract::DataContract, String> {
    if !format.document_schemas().contains_key(document_type_name) {
        return Err(format!(
            "missing restore document type {document_type_name}"
        ));
    }
    format
        .document_schemas_mut()
        .retain(|name, _| name == document_type_name);
    dpp::data_contract::DataContract::try_from_platform_versioned(
        format,
        false,
        &mut vec![],
        version,
    )
    .map_err(|e| format!("restore document contract: {e}"))
}

// Latest row is ordered by insertion, never by revision: deletes have nullable
// revisions and a restored document may have a lower revision than its history.
// Contract and document type are always part of the lookup key.
async fn current_document(
    tx: &Transaction<'_>,
    contract: &str,
    name: &str,
    identifier: &str,
) -> Result<tokio_postgres::Row, String> {
    tx.query_opt("SELECT d.id,d.owner,d.deleted,d.revision,d.data_contract_id,d.is_system,d.price,d.prefunded_voting_balance,d.moderated_at_ms,d.moderated_by, \
        (SELECT x.data FROM documents x JOIN data_contracts c ON c.id=x.data_contract_id WHERE x.identifier=d.identifier AND x.document_type_name=d.document_type_name AND c.identifier=$1 AND x.data IS NOT NULL ORDER BY x.id DESC LIMIT 1) AS data, \
        (SELECT x.revision FROM documents x JOIN data_contracts c ON c.id=x.data_contract_id WHERE x.identifier=d.identifier AND x.document_type_name=d.document_type_name AND c.identifier=$1 AND x.revision IS NOT NULL ORDER BY x.id DESC LIMIT 1) AS latest_revision \
        FROM documents d JOIN data_contracts c ON c.id=d.data_contract_id WHERE c.identifier=$1 AND d.document_type_name=$2 AND d.identifier=$3 ORDER BY d.id DESC LIMIT 1", &[&contract,&name,&identifier])
        .await.map_err(|e| e.to_string())?.ok_or_else(|| format!("missing retained document {contract}/{name}/{identifier}"))
}

async fn append_document(
    tx: &Transaction<'_>,
    contract: &str,
    name: &str,
    identifier: &str,
    hash: &str,
    source: &tokio_postgres::Row,
    data: Value,
    revision: Option<i32>,
    deleted: bool,
    action: i64,
    moderated_at: Option<i64>,
    moderated_by: Option<String>,
) -> Result<(), String> {
    let owner: String = source.get("owner");
    let contract_id: i32 = source.get("data_contract_id");
    let is_system: bool = source.get("is_system");
    let price: Option<i64> = source.get("price");
    let prefunded: Option<Value> = source.get("prefunded_voting_balance");
    // Unlike the batch DAO, never infer document owner from the moderator's ST.
    tx.execute("INSERT INTO documents(identifier,document_type_name,transition_type,owner,revision,data,deleted,state_transition_hash,data_contract_id,is_system,price,prefunded_voting_balance,moderated_at_ms,moderated_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)",
        &[&identifier,&name,&action,&owner,&revision,&data,&deleted,&hash,&contract_id,&is_system,&price,&prefunded,&moderated_at,&moderated_by]).await.map_err(|e| format!("document projection {contract}: {e}"))?;
    Ok(())
}

impl PSQLProcessor {
    pub async fn handle_contract_moderation(
        &self,
        transition: ContractUserModerationTransition,
        hash: String,
        height: i32,
        index: usize,
        tx: &Transaction<'_>,
    ) -> Result<(), ProcessorError> {
        self.project_contract_moderation(transition, &hash, height, tx)
            .await
            .map_err(|detail| ProcessorError::TransactionError {
                height,
                index,
                stage: "moderation projection",
                detail,
            })
    }

    async fn project_contract_moderation(
        &self,
        transition: ContractUserModerationTransition,
        hash: &str,
        height: i32,
        tx: &Transaction<'_>,
    ) -> Result<(), String> {
        let ContractUserModerationTransition::V0(st) = transition;
        let contract = st.data_contract_id.to_string(Base58);
        let moderator = st.owner_id.to_string(Base58);
        let time: i64 = tx
            .query_one(
                "SELECT (extract(epoch FROM timestamp)*1000)::bigint FROM blocks WHERE height=$1",
                &[&height],
            )
            .await
            .map_err(|e| e.to_string())?
            .get(0);
        let action = action_json(&st.action);
        let outcome = projection_outcome(&st.action);
        tx.execute("INSERT INTO contract_moderation_events(state_transition_hash,contract_identifier,moderator_identifier,timestamp_ms,action,outcome) VALUES ($1,$2,$3,$4,$5,$6)", &[&hash,&contract,&moderator,&time,&action,&outcome]).await.map_err(|e| e.to_string())?;
        match &st.action {
            Action::Ban { identity_id, .. }
            | Action::Unban { identity_id }
            | Action::Suspend { identity_id, .. }
            | Action::Unsuspend { identity_id }
            | Action::Warn { identity_id, .. }
            | Action::ClearWarnings { identity_id } => {
                let identity = identity_id.to_string(Base58);
                let previous = tx.query_opt("SELECT state FROM contract_moderation_identity_state WHERE contract_identifier=$1 AND identity_identifier=$2", &[&contract,&identity]).await.map_err(|e| e.to_string())?.map(|r| r.get(0)).unwrap_or(json!({}));
                let next = identity_effect(previous, &st.action, time, &moderator)?;
                tx.execute("INSERT INTO contract_moderation_identity_state(contract_identifier,identity_identifier,state,state_transition_hash) VALUES ($1,$2,$3,$4) ON CONFLICT(contract_identifier,identity_identifier) DO UPDATE SET state=EXCLUDED.state,state_transition_hash=EXCLUDED.state_transition_hash", &[&contract,&identity,&next,&hash]).await.map_err(|e| e.to_string())?;
            }
            Action::DeleteDocument {
                document_type_name,
                document_id,
                ..
            } => {
                let id = document_id.to_string(Base58);
                let current = current_document(tx, &contract, document_type_name, &id).await?;
                if current.get::<_, bool>("deleted") {
                    return Err(
                        "successful moderation delete targets an already deleted Explorer document"
                            .into(),
                    );
                }
                let data: Value = current
                    .get::<_, Option<Value>>("data")
                    .ok_or("missing pre-delete document properties")?;
                append_document(
                    tx,
                    &contract,
                    document_type_name,
                    &id,
                    hash,
                    &current,
                    data,
                    current.get("latest_revision"),
                    true,
                    2,
                    current.get("moderated_at_ms"),
                    current.get("moderated_by"),
                )
                .await?;
                let source_id: i32 = current.get("id");
                tx.execute("INSERT INTO contract_moderation_document_removals(contract_identifier,document_type_name,document_identifier,deleted_by_transition,removed_document_row_id) VALUES ($1,$2,$3,$4,$5)", &[&contract,&document_type_name,&id,&hash,&source_id]).await.map_err(|e| e.to_string())?;
            }
            Action::ChangeDocumentFields {
                document_type_name,
                document_id,
                fields,
                ..
            } => {
                let id = document_id.to_string(Base58);
                let current = current_document(tx, &contract, document_type_name, &id).await?;
                if current.get::<_, bool>("deleted") {
                    return Err("field change targets a deleted Explorer document".into());
                }
                let data = changed_properties(
                    current
                        .get::<_, Option<Value>>("data")
                        .ok_or("missing document properties")?,
                    fields,
                )?;
                let revision = current
                    .get::<_, Option<i32>>("latest_revision")
                    .ok_or("missing document revision")?
                    .checked_add(1)
                    .ok_or("document revision exceeds Explorer int32 representation")?;
                append_document(
                    tx,
                    &contract,
                    document_type_name,
                    &id,
                    hash,
                    &current,
                    data,
                    Some(revision),
                    false,
                    1,
                    Some(time),
                    Some(moderator.clone()),
                )
                .await?;
            }
            Action::RestoreDocument {
                document_type_name,
                document,
            } => {
                // Restore supplies the authoritative serialized historical document.
                // Recover the complete contract from retained original ST bytes, not
                // the lossy Explorer schema-only projection.
                let encoded: String = tx.query_opt("SELECT s.data FROM data_contracts c JOIN state_transitions s ON s.hash=c.state_transition_hash WHERE c.identifier=$1 ORDER BY c.version DESC,c.id DESC LIMIT 1", &[&contract]).await.map_err(|e| e.to_string())?.ok_or("missing retained contract transition for document restore")?.get(0);
                let decoded = self
                    .decoder
                    .decode(STANDARD.decode(encoded).map_err(|e| e.to_string())?)
                    .await
                    .map_err(|e| e.to_string())?;
                let format = match decoded {
                    StateTransition::DataContractCreate(dpp::state_transition::data_contract_create_transition::DataContractCreateTransition::V0(v)) => v.data_contract,
                    StateTransition::DataContractCreate(dpp::state_transition::data_contract_create_transition::DataContractCreateTransition::V1(v)) => v.data_contract,
                    StateTransition::DataContractUpdate(dpp::state_transition::data_contract_update_transition::DataContractUpdateTransition::V0(v)) => v.data_contract,
                    _ => return Err("contract row does not reference a contract transition".into()),
                };
                let version = dpp::version::PlatformVersion::get(14).map_err(|e| e.to_string())?;
                let full_contract = restore_document_contract(format, document_type_name, version)?;
                let document_type = full_contract
                    .document_type_for_name(document_type_name)
                    .map_err(|e| e.to_string())?;
                let restored = dpp::document::Document::from_bytes(
                    document.as_slice(),
                    document_type,
                    version,
                )
                .map_err(|e| e.to_string())?;
                let id = restored.id().to_string(Base58);
                let current = current_document(tx, &contract, document_type_name, &id).await?;
                // The successful restore carries the full authoritative document.
                // A preceding team deletion (or ordinary batch deletion) need not
                // have a direct-moderation removal row in this Explorer projection.
                let owner: String = current.get("owner");
                if owner.trim() != restored.owner_id().to_string(Base58) {
                    return Err("restored owner differs from retained document owner".into());
                }
                let revision = restored
                    .revision()
                    .map(i32::try_from)
                    .transpose()
                    .map_err(|_| "restored revision exceeds Explorer int32 representation")?;
                let data =
                    serde_json::to_value(restored.properties()).map_err(|e| e.to_string())?;
                append_document(
                    tx,
                    &contract,
                    document_type_name,
                    &id,
                    hash,
                    &current,
                    data,
                    revision,
                    false,
                    0,
                    current.get("moderated_at_ms"),
                    current.get("moderated_by"),
                )
                .await?;
                tx.execute("UPDATE contract_moderation_document_removals SET restored_by_transition=$1 WHERE contract_identifier=$2 AND document_type_name=$3 AND document_identifier=$4 AND restored_by_transition IS NULL", &[&hash,&contract,&document_type_name,&id]).await.map_err(|e| e.to_string())?;
            }
            Action::DeleteSettledDocument { .. } | Action::ApproveTeamAction { .. } => {
                // Full signed transition, successful result and action event are
                // retained. Do not turn a proposal/signature into a tombstone.
            }
        }
        self.handle_data_contract_transition(Some(hash.into()), st.data_contract_id, tx)
            .await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpp::data_contract::config::moderation::ContractModerationReason;

    async fn actual_unicode_6681_projection() -> (Value, Value) {
        use dpp::serialization::PlatformSerializable;
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/sakura-moderation-6681.json"
        ))
        .unwrap();
        let result: crate::models::TDTxResult =
            serde_json::from_value(fixture["result"].clone()).unwrap();
        assert_eq!(
            result.code.unwrap_or(0),
            0,
            "this action succeeded on chain"
        );
        let bytes = STANDARD
            .decode(fixture["txBase64"].as_str().unwrap())
            .unwrap();
        assert_eq!(sha256::digest(&bytes).to_uppercase(), fixture["txSha256"]);
        let decoder = crate::decoder::decoder::StateTransitionDecoder::new();
        let decoded = decoder.decode(bytes.clone()).await.unwrap();
        assert_eq!(decoded.serialize_to_bytes().unwrap(), bytes);
        let StateTransition::ContractUserModeration(ContractUserModerationTransition::V0(st)) =
            decoded
        else {
            panic!("expected moderation transition");
        };
        assert!(matches!(&st.action, Action::Warn { .. }));
        let action = action_json(&st.action);
        assert_eq!(action["reason"]["text"], "odd name");
        assert_eq!(
            action["reason"]["documents"][0]["documentTypeName"],
            fixture["expectedReasonDocumentTypeName"]
        );
        assert!(action["reason"]["documents"][0]["documentTypeName"]
            .as_str()
            .unwrap()
            .contains('\0'));
        let state = identity_effect(
            json!({}),
            &st.action,
            1791358527328,
            &st.owner_id.to_string(Base58),
        )
        .unwrap();
        assert_eq!(state["warnings"][0]["reason"], action["reason"]);
        assert_eq!(
            serde_json::from_str::<Value>(&serde_json::to_string(&action).unwrap()).unwrap(),
            action
        );
        assert_eq!(
            serde_json::from_str::<Value>(&serde_json::to_string(&state).unwrap()).unwrap(),
            state
        );
        (action, state)
    }

    #[tokio::test]
    async fn actual_successful_warning_6681_preserves_unicode_and_wire_bytes() {
        actual_unicode_6681_projection().await;
    }

    #[tokio::test]
    #[ignore = "requires explicit PE_UNICODE_TEST_DATABASE_URL for disposable pe_c0cf08_unicode_test database"]
    async fn postgres_moderation_unicode_migration_preserves_old_and_actual_values() {
        let config: tokio_postgres::Config = std::env::var("PE_UNICODE_TEST_DATABASE_URL")
            .expect("explicit disposable database URL required")
            .parse()
            .unwrap();
        assert_eq!(config.get_dbname(), Some("pe_c0cf08_unicode_test"));
        let (mut client, connection) = config.connect(tokio_postgres::NoTls).await.unwrap();
        let connection_task = tokio::spawn(async move { connection.await.unwrap() });
        let (action, state) = actual_unicode_6681_projection().await;
        for value in [&action, &state] {
            let error = client
                .query_one("SELECT $1::jsonb", &[value])
                .await
                .unwrap_err();
            let db_error = error
                .as_db_error()
                .expect("old storage rejects real U+0000");
            assert_eq!(db_error.code().code(), "22P05");
            assert!(db_error
                .detail()
                .unwrap()
                .contains("\\u0000 cannot be converted to text"));
        }
        let tx = client.transaction().await.unwrap();
        // Use transaction-local temporary tables only. The exact production
        // migration must never resolve to persistent tables in this test.
        tx.batch_execute("SET LOCAL search_path TO pg_temp, pg_catalog;
            CREATE TEMP TABLE contract_moderation_events (id integer PRIMARY KEY, action jsonb NOT NULL) ON COMMIT DROP;
            CREATE TEMP TABLE contract_moderation_identity_state (id integer PRIMARY KEY, state jsonb NOT NULL) ON COMMIT DROP;
            CREATE INDEX unicode_event_id ON contract_moderation_events(id);").await.unwrap();
        let old_action = json!({"type":"warn","reason":{"text":"previous","code":null}});
        let old_state = json!({"warnings":[{"reason":{"text":"previous","code":null}}]});
        tx.execute(
            "INSERT INTO contract_moderation_events VALUES (1,$1)",
            &[&old_action],
        )
        .await
        .unwrap();
        tx.execute(
            "INSERT INTO contract_moderation_identity_state VALUES (1,$1)",
            &[&old_state],
        )
        .await
        .unwrap();
        tx.batch_execute(include_str!(
            "../../../../migrations/V83__preserve_moderation_unicode.sql"
        ))
        .await
        .unwrap();
        tx.execute(
            "INSERT INTO contract_moderation_events VALUES (2,$1)",
            &[&action],
        )
        .await
        .unwrap();
        tx.execute(
            "INSERT INTO contract_moderation_identity_state VALUES (2,$1)",
            &[&state],
        )
        .await
        .unwrap();
        for (table, column, expected) in [
            (
                "contract_moderation_events",
                "action",
                [&old_action, &action],
            ),
            (
                "contract_moderation_identity_state",
                "state",
                [&old_state, &state],
            ),
        ] {
            let rows = tx
                .query(
                    &format!("SELECT {column}, pg_typeof({column})::text FROM {table} ORDER BY id"),
                    &[],
                )
                .await
                .unwrap();
            assert_eq!(rows.len(), 2);
            for (row, expected) in rows.iter().zip(expected) {
                assert_eq!(&row.get::<_, Value>(0), expected);
                assert_eq!(row.get::<_, String>(1), "json");
            }
        }
        let indexes: i64 = tx.query_one("SELECT count(*) FROM pg_index WHERE indrelid IN ('contract_moderation_events'::regclass, 'contract_moderation_identity_state'::regclass) AND indisvalid", &[]).await.unwrap().get(0);
        assert_eq!(indexes, 3, "primary and ordinary indexes remain valid");
        // Subsequent warnings must read and retain the NUL-containing reason.
        let retained: Value = tx
            .query_one(
                "SELECT state FROM contract_moderation_identity_state WHERE id=2",
                &[],
            )
            .await
            .unwrap()
            .get(0);
        let next = identity_effect(
            retained,
            &Action::Warn {
                identity_id: Identifier::default(),
                reason: ContractModerationReason::from_text("later"),
            },
            1791358528000,
            "moderator",
        )
        .unwrap();
        tx.execute(
            "UPDATE contract_moderation_identity_state SET state=$1 WHERE id=2",
            &[&next],
        )
        .await
        .unwrap();
        let roundtrip: Value = tx
            .query_one(
                "SELECT state FROM contract_moderation_identity_state WHERE id=2",
                &[],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(roundtrip, next);
        assert_eq!(roundtrip["warnings"][0], state["warnings"][0]);
        assert_eq!(roundtrip["warnings"].as_array().unwrap().len(), 2);
        tx.rollback().await.unwrap();
        drop(client);
        connection_task.await.unwrap();
    }

    #[tokio::test]
    async fn actual_restore_5332_ignores_unrelated_index_grammar_without_losing_document() {
        use dpp::serialization::PlatformSerializable;
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/sakura-restore-5332.json"
        ))
        .unwrap();
        let decoder = crate::decoder::decoder::StateTransitionDecoder::new();
        let bytes = STANDARD
            .decode(fixture["txBase64"].as_str().unwrap())
            .unwrap();
        assert_eq!(sha256::digest(&bytes).to_uppercase(), fixture["txSha256"]);
        let decoded = decoder.decode(bytes.clone()).await.unwrap();
        assert_eq!(decoded.serialize_to_bytes().unwrap(), bytes);
        let StateTransition::ContractUserModeration(ContractUserModerationTransition::V0(st)) =
            decoded
        else {
            panic!("expected moderation transition");
        };
        assert_eq!(st.data_contract_id.to_string(Base58), fixture["contractId"]);
        let Action::RestoreDocument {
            document_type_name,
            document,
        } = st.action
        else {
            panic!("expected restore");
        };
        assert_eq!(document_type_name, "post");
        let contract_bytes = STANDARD
            .decode(fixture["contractTransitionBase64"].as_str().unwrap())
            .unwrap();
        let contract_transition = decoder.decode(contract_bytes.clone()).await.unwrap();
        assert_eq!(
            contract_transition.serialize_to_bytes().unwrap(),
            contract_bytes
        );
        let format = match contract_transition {
            StateTransition::DataContractCreate(dpp::state_transition::data_contract_create_transition::DataContractCreateTransition::V1(v)) => v.data_contract,
            _ => panic!("expected retained contract create v1"),
        };
        let original_format = format.clone();
        let version = dpp::version::PlatformVersion::get(14).unwrap();
        // The old production path fails on `like`'s summableOffCountIndex,
        // although the restored `post` schema itself is supported unchanged.
        let old_error = dpp::data_contract::DataContract::try_from_platform_versioned(
            format.clone(),
            false,
            &mut vec![],
            version,
        )
        .unwrap_err();
        assert!(old_error.to_string().contains("unexpected property name"));
        assert!(
            restore_document_contract(format.clone(), "missing-type", version)
                .unwrap_err()
                .contains("missing restore document type")
        );
        let contract =
            restore_document_contract(format.clone(), &document_type_name, version).unwrap();
        assert_eq!(
            format, original_format,
            "retained contract must stay unchanged"
        );
        let document_type = contract
            .document_type_for_name(&document_type_name)
            .unwrap();
        let restored =
            dpp::document::Document::from_bytes(document.as_slice(), document_type, version)
                .unwrap();
        assert_eq!(
            restored.id().to_string(Base58),
            "CSK1tnMLbqonFyHnox9FvGcFjFqSCRMr4AHwZZQk5Fby"
        );
        assert_eq!(
            restored.owner_id().to_string(Base58),
            "GtZu3frUm7Dg78T3hxQrTpmyczMvuMQEmo2pNe7ZWmtU"
        );
        assert_eq!(restored.revision(), Some(1));
        assert_eq!(
            serde_json::to_value(restored.properties()).unwrap(),
            json!({
                "content": "to be removed and restored", "live": true
            })
        );
        assert_eq!(
            restored
                .serialize(document_type, &contract, version)
                .unwrap(),
            document.as_slice(),
            "complete restored document must round-trip byte-for-byte"
        );
        assert!(
            dpp::document::Document::from_bytes(&document.as_slice()[..20], document_type, version)
                .is_err(),
            "malformed restored bytes must still fail"
        );
    }

    #[tokio::test]
    async fn actual_moderation_fixtures_decode_losslessly_and_preserve_unknown_outcomes() {
        use dpp::serialization::PlatformSerializable;
        let fixtures: Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/sakura-moderation.json"
        ))
        .unwrap();
        let decoder = crate::decoder::decoder::StateTransitionDecoder::new();
        let mut direct = 0;
        for f in fixtures.as_array().unwrap() {
            let bytes = STANDARD.decode(f["txBase64"].as_str().unwrap()).unwrap();
            let decoded = decoder.decode(bytes.clone()).await.unwrap();
            assert_eq!(decoded.serialize_to_bytes().unwrap(), bytes);
            let StateTransition::ContractUserModeration(ContractUserModerationTransition::V0(st)) =
                decoded
            else {
                continue;
            };
            match f["height"].as_u64().unwrap() {
                703 => {
                    assert_eq!(
                        st.data_contract_id.to_string(Base58),
                        "9DvxGGj3sFfWFQRAsDokjbsV3banEnah364SyWDcvKxA"
                    );
                    let Action::DeleteDocument {
                        document_type_name, ..
                    } = st.action
                    else {
                        panic!("703 must delete a document")
                    };
                    assert_eq!(document_type_name, "post");
                    direct += 1;
                }
                709 | 710 => {
                    assert!(matches!(
                        st.action,
                        Action::DeleteSettledDocument { .. } | Action::ApproveTeamAction { .. }
                    ));
                    let outcome = projection_outcome(&st.action);
                    assert_eq!(outcome["status"], "unavailable");
                    assert!(outcome["documentDeleted"].is_null());
                    assert!(action_json(&st.action).is_object());
                }
                771 | 774 | 778 | 779 | 951 => {
                    identity_effect(json!({}), &st.action, 42, "moderator").unwrap();
                    direct += 1;
                }
                777 => {
                    assert!(matches!(st.action, Action::RestoreDocument { .. }));
                    direct += 1;
                }
                905 => {
                    assert!(matches!(st.action, Action::ChangeDocumentFields { .. }));
                    direct += 1;
                }
                _ => panic!("unexpected fixture"),
            }
        }
        assert_eq!(direct, 8);
    }
    #[test]
    fn ban_removes_suspension_without_losing_warnings() {
        let action = Action::Ban {
            identity_id: Identifier::default(),
            reason: ContractModerationReason::default(),
        };
        let next = identity_effect(
            json!({"suspension":{"until":999},"warnings":[{"reason":"previous"}]}),
            &action,
            100,
            "moderator",
        )
        .unwrap();
        assert!(next.get("suspension").is_none());
        assert_eq!(next["warnings"].as_array().unwrap().len(), 1);
        assert_eq!(next["ban"]["timestampMs"], 100);
    }
    #[test]
    fn moderator_changes_preserve_unmentioned_fields_and_remove_nulls() {
        let fields = std::collections::BTreeMap::from([
            ("remove".into(), dpp::platform_value::Value::Null),
            ("score".into(), dpp::platform_value::Value::U64(7)),
        ]);
        assert_eq!(
            changed_properties(
                json!({"remove":true,"unchanged":"x","$updatedAt":42}),
                &fields
            )
            .unwrap(),
            json!({"unchanged":"x","$updatedAt":42,"score":7})
        );
    }
}
