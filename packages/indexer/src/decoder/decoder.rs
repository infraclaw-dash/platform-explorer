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
