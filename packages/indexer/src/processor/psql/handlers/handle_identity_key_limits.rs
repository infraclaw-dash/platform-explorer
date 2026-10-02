//! Successful key-limit updates change only the supplied absolute limits.
//! Identity revisions and public keys are intentionally left untouched.
use super::super::{PSQLProcessor, ProcessorError};
use deadpool_postgres::Transaction;
use dpp::platform_value::string_encoding::Encoding::Base58;
use dpp::state_transition::identity_key_limits_update_transition::IdentityKeyLimitsUpdateTransition;
use serde_json::{json, Value};

fn limit_changes(total_budget: Option<u64>, expires_at: Option<u64>) -> Value {
    let mut changes = json!({});
    // Decimal strings retain u64 precision through JSON/JavaScript consumers.
    // Absent fields mean unchanged, not reset or unlimited.
    if let Some(budget) = total_budget {
        changes["totalBudget"] = json!(budget.to_string());
    }
    if let Some(expiry) = expires_at {
        changes["expiresAt"] = json!(expiry.to_string());
    }
    changes
}

impl PSQLProcessor {
    pub async fn handle_identity_key_limits(
        &self,
        transition: IdentityKeyLimitsUpdateTransition,
        hash: String,
        height: i32,
        index: usize,
        tx: &Transaction<'_>,
    ) -> Result<(), ProcessorError> {
        let IdentityKeyLimitsUpdateTransition::V0(st) = transition;
        let identity = st.identity_id.to_string(Base58);
        let key = i64::from(st.key_id);
        let changes = limit_changes(st.total_budget, st.expires_at);
        let failure = |error: tokio_postgres::Error| ProcessorError::TransactionError {
            height,
            index,
            stage: "identity key limits projection",
            detail: error.to_string(),
        };
        tx.execute(
            "INSERT INTO identity_key_limit_events(state_transition_hash,identity_identifier,key_id,changes) VALUES ($1,$2,$3,$4)",
            &[&hash, &identity, &key, &changes],
        ).await.map_err(failure)?;
        tx.execute(
            "INSERT INTO identity_key_limit_state(identity_identifier,key_id,limits,state_transition_hash) VALUES ($1,$2,$3,$4) ON CONFLICT(identity_identifier,key_id) DO UPDATE SET limits=identity_key_limit_state.limits || EXCLUDED.limits,state_transition_hash=EXCLUDED.state_transition_hash",
            &[&identity, &key, &changes, &hash],
        ).await.map_err(failure)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use dpp::serialization::PlatformSerializable;
    use dpp::state_transition::StateTransition;

    #[test]
    fn omitted_limit_is_unchanged_and_u64_is_exact() {
        let changes = limit_changes(Some(u64::MAX), None);
        assert_eq!(changes["totalBudget"], u64::MAX.to_string());
        assert!(changes.get("expiresAt").is_none());
        assert_eq!(limit_changes(None, Some(42)), json!({"expiresAt":"42"}));
    }

    #[tokio::test]
    async fn actual_key_limit_update_roundtrips_without_identity_revision() {
        let fixtures: Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/sakura-key-limits.json"
        )).unwrap();
        let fixture = &fixtures[0];
        assert_eq!(fixture["height"], 10162);
        let bytes = STANDARD.decode(fixture["txBase64"].as_str().unwrap()).unwrap();
        let decoder = crate::decoder::decoder::StateTransitionDecoder::new();
        let decoded = decoder.decode(bytes.clone()).await.unwrap();
        assert_eq!(decoded.serialize_to_bytes().unwrap(), bytes);
        let StateTransition::IdentityKeyLimitsUpdate(IdentityKeyLimitsUpdateTransition::V0(st)) = decoded else {
            panic!("expected actual key limit update");
        };
        let changes = limit_changes(st.total_budget, st.expires_at);
        assert!(changes.get("revision").is_none());
        assert!(changes.get("publicKeys").is_none());
        assert!(!changes.as_object().unwrap().is_empty());
    }
}
