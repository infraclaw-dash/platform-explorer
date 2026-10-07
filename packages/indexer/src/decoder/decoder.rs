use dpp::state_transition::state_transition_factory::StateTransitionFactory;
use dpp::state_transition::StateTransition;
use dpp::ProtocolError;

pub struct StateTransitionDecoder {
    st_factory: StateTransitionFactory,
}

impl StateTransitionDecoder {
    pub async fn decode(&self, data: Vec<u8>) -> Result<StateTransition, ProtocolError> {
        let array = data.as_slice();

        let result = self.st_factory.create_from_buffer(array);

        return result;
    }

    pub fn new() -> StateTransitionDecoder {
        let st_factory = StateTransitionFactory {};

        return StateTransitionDecoder { st_factory };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::document::Document;
    use crate::processor::psql::handlers::handle_block::decode_block_transaction;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use dpp::serialization::PlatformSerializable;
    use dpp::state_transition::batch_transition::batched_transition::BatchedTransition;
    use dpp::state_transition::batch_transition::document_base_transition::document_base_transition_trait::DocumentBaseTransitionAccessors;
    use dpp::state_transition::batch_transition::document_base_transition::DocumentBaseTransition;
    use dpp::state_transition::batch_transition::BatchTransition;

    #[tokio::test]
    async fn sakura_tail_7392_through_7505_preserves_every_wire_transaction() {
        let fixtures: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/sakura-tail-7392-7505.json"
        ))
        .unwrap();
        let fixtures = fixtures.as_array().unwrap();
        assert_eq!(fixtures.len(), 116);
        for fixture in fixtures {
            let height = fixture["height"].as_i64().unwrap() as i32;
            let index = fixture["index"].as_u64().unwrap() as usize;
            let encoded = fixture["txBase64"].as_str().unwrap();
            let bytes = STANDARD.decode(encoded).unwrap();
            assert_eq!(sha256::digest(&bytes).to_uppercase(), fixture["txSha256"]);
            let transition =
                decode_block_transaction(&StateTransitionDecoder::new(), encoded, height, index)
                    .await
                    .unwrap();
            assert_eq!(
                transition.serialize_to_bytes().unwrap(),
                bytes,
                "wire data at {height}/{index}"
            );
            let result: crate::models::TDTxResult =
                serde_json::from_value(fixture["result"].clone()).unwrap();
            println!(
                "captured {height}/{index} code={} transition={}",
                result.code.unwrap_or(0),
                transition.state_transition_type()
            );
        }
    }

    #[tokio::test]
    async fn sakura_7392_token_configuration_v1_preserves_pool_and_wire_bytes() {
        use dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
        use dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
        use dpp::data_contract::associated_token::token_configuration_localization::accessors::v0::TokenConfigurationLocalizationV0Getters;
        use dpp::data_contract::TokenConfiguration;
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/sakura-token-config-7392.json"
        ))
        .unwrap();
        let result: crate::models::TDTxResult =
            serde_json::from_value(fixture["result"].clone()).unwrap();
        assert_eq!(
            result.code.unwrap_or(0),
            0,
            "contract creation succeeded on chain"
        );
        let encoded = fixture["txBase64"].as_str().unwrap();
        let bytes = STANDARD.decode(encoded).unwrap();
        assert_eq!(sha256::digest(&bytes).to_uppercase(), fixture["txSha256"]);
        let decoded = decode_block_transaction(&StateTransitionDecoder::new(), encoded, 7392, 0)
            .await
            .unwrap();
        assert_eq!(decoded.serialize_to_bytes().unwrap(), bytes);
        let StateTransition::DataContractCreate(transition) = decoded else {
            panic!("expected data contract creation");
        };
        // The production conversion must retain the complete token configuration,
        // including V1 fields, rather than coerce it to the old V0 representation.
        let contract = crate::entities::data_contract::DataContract::from(transition);
        let tokens = contract.tokens.unwrap();
        assert_eq!(tokens.len(), 1);
        let token = tokens.get(&0).unwrap();
        let TokenConfiguration::V1(pool) = token else {
            panic!("expected V1 token configuration");
        };
        assert!(pool.has_shielded_pool);
        assert_eq!(token.base_supply(), 1_000_000);
        assert_eq!(
            token
                .conventions()
                .localizations()
                .get("en")
                .unwrap()
                .singular_form(),
            "qapool"
        );
        assert_eq!(
            token
                .conventions()
                .localizations()
                .get("en")
                .unwrap()
                .plural_form(),
            "qapools"
        );
        let json = serde_json::to_value(token).unwrap();
        assert_eq!(json["$formatVersion"], "1");
        assert_eq!(json["hasShieldedPool"], true);
        assert!(contract.schema.unwrap().get("n").is_some());
    }

    #[tokio::test]
    async fn sakura_block_325_decodes_v2_and_preserves_transaction_and_document() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/sakura-block-325.json"))
                .unwrap();
        assert_eq!(fixture["protocolVersion"], 14);
        assert_eq!(fixture["transactions"], 1);
        let encoded = fixture["txBase64"][0].as_str().unwrap();
        let bytes = STANDARD.decode(encoded).unwrap();
        assert_eq!(
            sha256::digest(&bytes).to_uppercase(),
            fixture["txSha256"].as_str().unwrap()
        );
        // Use precisely the block handler's decoding path, not a permissive test decoder.
        let state_transition =
            decode_block_transaction(&StateTransitionDecoder::new(), encoded, 325, 0)
                .await
                .unwrap();
        assert_eq!(state_transition.serialize_to_bytes().unwrap(), bytes);
        let StateTransition::Batch(BatchTransition::V1(batch)) = state_transition else {
            panic!("expected batch v1");
        };
        assert_eq!(
            batch.transitions.len(),
            1,
            "no transactions may be discarded"
        );
        let BatchedTransition::Document(document_transition) =
            batch.transitions.into_iter().next().unwrap()
        else {
            panic!("expected document transition");
        };
        let dpp::state_transition::batch_transition::batched_transition::document_transition::DocumentTransition::Create(ref create) = document_transition else { panic!("expected create"); };
        assert!(matches!(create.base(), DocumentBaseTransition::V2(_)));
        // Exercise the production entity conversion used by the document handler.
        let document = Document::from(document_transition);
        assert_eq!(document.document_type_name, "preorder");
        assert!(!document.deleted);
        assert_eq!(document.revision, Some(1));
        assert!(document.data.unwrap().get("saltedDomainHash").is_some());
    }
    #[tokio::test]
    async fn index_only_delete_retains_the_full_property_tuple_and_tombstone() {
        use dpp::state_transition::batch_transition::batched_transition::document_transition::DocumentTransition;
        use dpp::state_transition::batch_transition::batched_transition::document_index_only_delete_transition::v0::DocumentIndexOnlyDeleteTransitionV0;
        use dpp::state_transition::batch_transition::document_create_transition::v0::v0_methods::DocumentCreateTransitionV0Methods;
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/sakura-block-325.json"))
                .unwrap();
        let st = StateTransitionDecoder::new()
            .decode(
                STANDARD
                    .decode(fixture["txBase64"][0].as_str().unwrap())
                    .unwrap(),
            )
            .await
            .unwrap();
        let StateTransition::Batch(BatchTransition::V1(mut batch)) = st else {
            panic!("expected batch");
        };
        let BatchedTransition::Document(DocumentTransition::Create(create)) =
            batch.transitions.remove(0)
        else {
            panic!("expected create");
        };
        let deletion = DocumentIndexOnlyDeleteTransitionV0 {
            base: create.base().clone(),
            data: create.data().clone(),
        };
        let document = Document::from(DocumentTransition::IndexOnlyDelete(deletion.into()));
        assert!(document.deleted);
        assert_eq!(document.document_type_name, "preorder");
        assert!(document.data.unwrap().get("saltedDomainHash").is_some());
        assert_eq!(document.transition_type as i32, 7);
    }
}
