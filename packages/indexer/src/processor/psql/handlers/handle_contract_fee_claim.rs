//! Fee-claim events do not encode a payout amount. Preserve the successful claim
//! without fabricating transfers or confusing gas_used with claimed credits.
use super::super::{PSQLProcessor, ProcessorError};
use deadpool_postgres::Transaction;
use dpp::data_contract::document_type::action_fees::ContractFeePot;
use dpp::platform_value::string_encoding::Encoding::Base58;
use dpp::state_transition::contract_fee_claim_transition::ContractFeeClaimTransition;
use serde_json::{json, Value};

fn claim_outcome() -> Value {
    json!({
        "status": "unavailable",
        "reason": "Signed claim and transaction result do not include payout amounts or recipients"
    })
}

impl PSQLProcessor {
    pub async fn handle_contract_fee_claim(
        &self,
        transition: ContractFeeClaimTransition,
        hash: String,
        height: i32,
        index: usize,
        tx: &Transaction<'_>,
    ) -> Result<(), ProcessorError> {
        let ContractFeeClaimTransition::V0(st) = transition;
        let contract = st.data_contract_id.to_string(Base58);
        let claimant = st.owner_id.to_string(Base58);
        let pot = match st.pot {
            ContractFeePot::Owner => "owner",
            ContractFeePot::Moderators => "moderators",
        };
        let outcome = claim_outcome();
        tx.execute(
            "INSERT INTO contract_fee_claim_events(state_transition_hash,contract_identifier,claimant_identifier,pot,amount,recipients,outcome) VALUES ($1,$2,$3,$4,NULL,NULL,$5)",
            &[&hash, &contract, &claimant, &pot, &outcome],
        )
        .await
        .map_err(|error| ProcessorError::TransactionError {
            height,
            index,
            stage: "fee claim projection",
            detail: error.to_string(),
        })?;
        self.handle_data_contract_transition(Some(hash), st.data_contract_id, tx)
            .await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use dpp::serialization::PlatformSerializable;
    use dpp::state_transition::StateTransition;

    #[tokio::test]
    async fn actual_fee_claim_preserves_signed_event_without_guessing_payout() {
        let fixtures: Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/sakura-moderation.json"
        ))
        .unwrap();
        let fixture = fixtures.as_array().unwrap().iter()
            .find(|f| f["height"] == 950).unwrap();
        let bytes = STANDARD.decode(fixture["txBase64"].as_str().unwrap()).unwrap();
        let decoder = crate::decoder::decoder::StateTransitionDecoder::new();
        let decoded = decoder.decode(bytes.clone()).await.unwrap();
        assert_eq!(decoded.serialize_to_bytes().unwrap(), bytes);
        let StateTransition::ContractFeeClaim(ContractFeeClaimTransition::V0(st)) = decoded else {
            panic!("fixture must contain a fee claim");
        };
        assert_eq!(st.data_contract_id.to_string(Base58), "C94i8HwriER7YNcYX24MRw3vKxd5hhsQsQ5QqYa51P4Y");
        assert!(matches!(st.pot, ContractFeePot::Moderators));
        assert_eq!(claim_outcome()["status"], "unavailable");
    }
}
