use crate::entities::document::Document;
use crate::entities::transfer::Transfer;
use crate::processor::psql::PSQLProcessor;
use crate::utils::build_dpns_alias;
use data_contracts::SystemDataContract;
use deadpool_postgres::Transaction;
use dpp::identifier::Identifier;
use dpp::platform_value::btreemap_extensions::BTreeValueMapPathHelper;
use dpp::platform_value::string_encoding::Encoding::Base58;
use dpp::state_transition::batch_transition::batched_transition::document_transfer_transition::v0::v0_methods::DocumentTransferTransitionV0Methods;
use dpp::state_transition::batch_transition::batched_transition::document_transition::{
    DocumentTransition, DocumentTransitionV0Methods,
};

impl PSQLProcessor {
    pub async fn handle_document_transition(
        &self,
        document_transition: DocumentTransition,
        owner_id: Identifier,
        st_hash: String,
        sql_transaction: &Transaction<'_>,
    ) -> () {
        let document = Document::from(document_transition.clone());
        let document_identifier = document.identifier.clone();

        match document_transition {
            DocumentTransition::Purchase(_) => {
                let current_document = self
                    .dao
                    .get_document_by_identifier(document.clone().identifier, sql_transaction)
                    .await
                    .unwrap()
                    .expect(&format!(
                        "Could not get Document with identifier {} from the database",
                        document.identifier
                    ));

                self.dao
                    .create_document(document.clone(), Some(st_hash.clone()), sql_transaction)
                    .await
                    .unwrap();

                let transfer = Transfer {
                    sender: document.clone().owner,
                    recipient: current_document.owner,
                    amount: document.clone().price.unwrap(),
                };
                self.dao
                    .create_transfer(transfer, st_hash.clone(), sql_transaction)
                    .await
                    .unwrap();
            }
            DocumentTransition::Transfer(ref transition) => {
                self.dao
                    .create_document(document.clone(), Some(st_hash.clone()), sql_transaction)
                    .await
                    .unwrap();

                self.dao
                    .set_state_transition_recipient(
                        transition.recipient_owner_id(),
                        st_hash.clone(),
                        sql_transaction,
                    )
                    .await
                    .unwrap();
            }
            _ => {
                self.dao
                    .create_document(document.clone(), Some(st_hash.clone()), sql_transaction)
                    .await
                    .unwrap();
            }
        }

        let document_type = document_transition.document_type_name();

        if document_type == "domain"
            && document_transition.data_contract_id() == SystemDataContract::DPNS.id()
        {
            match &document_transition {
                DocumentTransition::Create(_) | DocumentTransition::Replace(_) => {
                    let label = document_transition
                        .data()
                        .unwrap()
                        .get_str_at_path("label")
                        .unwrap();

                    let normalized_parent_domain_name = document_transition
                        .data()
                        .unwrap()
                        .get_str_at_path("parentDomainName")
                        .unwrap();

                    let identity_identifier = document_transition
                        .data()
                        .unwrap()
                        .get_optional_at_path("records.identity")
                        .unwrap()
                        .expect("Could not find DPNS domain document identity identifier");

                    let identity_identifier = Identifier::from_bytes(
                        &identity_identifier.clone().into_identifier_bytes().unwrap(),
                    )
                    .unwrap()
                    .to_string(Base58);
                    let identity = self
                        .dao
                        .get_identity_by_identifier(identity_identifier.clone(), sql_transaction)
                        .await
                        .unwrap()
                        .expect(&format!(
                            "Could not find identity with identifier {}",
                            identity_identifier
                        ));
                    let alias = format!("{}.{}", label, normalized_parent_domain_name);

                    self.dao
                        .create_identity_alias(identity, alias, st_hash.clone(), sql_transaction)
                        .await
                        .unwrap();
                }
                DocumentTransition::Transfer(transition) => {
                    let domain_document = self
                        .dao
                        .get_document_with_data_by_identifier(
                            document_identifier.clone(),
                            sql_transaction,
                        )
                        .await
                        .unwrap()
                        .expect(&format!(
                            "Could not get DPNS domain document with identifier {} from the database",
                            document_identifier
                        ));

                    self.dao
                        .update_identity_alias(
                            transition.recipient_owner_id(),
                            build_dpns_alias(domain_document),
                            st_hash.clone(),
                            sql_transaction,
                        )
                        .await
                        .unwrap();
                }
                DocumentTransition::Purchase(_) => {
                    let domain_document = self
                        .dao
                        .get_document_with_data_by_identifier(
                            document_identifier.clone(),
                            sql_transaction,
                        )
                        .await
                        .unwrap()
                        .expect(&format!(
                            "Could not get DPNS domain document with identifier {} from the database",
                            document_identifier
                        ));

                    self.dao
                        .update_identity_alias(
                            owner_id,
                            build_dpns_alias(domain_document),
                            st_hash.clone(),
                            sql_transaction,
                        )
                        .await
                        .unwrap();
                }
                DocumentTransition::Delete(_)
                | DocumentTransition::IndexOnlyDelete(_)
                | DocumentTransition::UpdatePrice(_) => {}
            }
        }

        self.handle_data_contract_transition(
            Some(st_hash.clone()),
            document.data_contract_identifier,
            sql_transaction,
        )
        .await;

        if document_type == "dataContracts"
            && document_transition.data_contract_id() == self.platform_explorer_identifier
        {
            let data_contract_identifier_str = document_transition
                .data()
                .unwrap()
                .get_str_at_path("identifier")
                .unwrap();

            let data_contract_identifier =
                Identifier::from_string(data_contract_identifier_str, Base58).unwrap();

            let data_contract = self
                .dao
                .get_data_contract_by_identifier(data_contract_identifier, sql_transaction)
                .await
                .expect(&format!(
                    "Could not get DataContract with identifier {} from the database",
                    data_contract_identifier_str
                ));

            if data_contract.is_some() {
                let data_contract = data_contract.unwrap();

                if data_contract.owner == owner_id {
                    let data_contract_name = document_transition
                        .data()
                        .unwrap()
                        .get_str_at_path("name")
                        .unwrap();

                    self.dao
                        .set_data_contract_name(
                            data_contract.clone(),
                            String::from(data_contract_name),
                            sql_transaction,
                        )
                        .await
                        .unwrap();
                }
            } else {
                println!("Failed to set custom data contract name for contract {}, owner of the tx {} does not match data contract", st_hash, document_identifier.to_string(Base58));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use dpp::serialization::PlatformSerializable;
    use dpp::state_transition::batch_transition::batched_transition::BatchedTransition;
    use dpp::state_transition::batch_transition::BatchTransition;
    use dpp::state_transition::StateTransition;
    use serde_json::Value;

    async fn actual_dpns_7386_alias() -> (String, String, String) {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/sakura-dpns-alias-7386.json"
        ))
        .unwrap();
        let result: crate::models::TDTxResult =
            serde_json::from_value(fixture["result"].clone()).unwrap();
        assert_eq!(
            result.code.unwrap_or(0),
            0,
            "this registration succeeded on chain"
        );
        let encoded = fixture["txBase64"].as_str().unwrap();
        let bytes = STANDARD.decode(encoded).unwrap();
        let hash = sha256::digest(&bytes).to_uppercase();
        assert_eq!(hash, fixture["txSha256"]);
        let decoded = crate::processor::psql::handlers::handle_block::decode_block_transaction(
            &crate::decoder::decoder::StateTransitionDecoder::new(),
            encoded,
            7386,
            0,
        )
        .await
        .unwrap();
        assert_eq!(decoded.serialize_to_bytes().unwrap(), bytes);
        let StateTransition::Batch(BatchTransition::V1(batch)) = decoded else {
            panic!("expected document batch");
        };
        assert_eq!(
            batch.transitions.len(),
            1,
            "retain every document transition"
        );
        let BatchedTransition::Document(transition) = batch.transitions.into_iter().next().unwrap()
        else {
            panic!("expected document transition");
        };
        assert!(matches!(&transition, DocumentTransition::Create(_)));
        assert_eq!(transition.document_type_name(), "domain");
        assert_eq!(transition.data_contract_id(), SystemDataContract::DPNS.id());
        let data = transition.data().unwrap();
        let label = data.get_str_at_path("label").unwrap();
        let parent = data.get_str_at_path("parentDomainName").unwrap();
        assert_eq!(label, fixture["expectedLabel"]);
        assert_eq!(parent, fixture["expectedParentDomainName"]);
        let identity = data
            .get_optional_at_path("records.identity")
            .unwrap()
            .unwrap();
        let identity = Identifier::from_bytes(&identity.clone().into_identifier_bytes().unwrap())
            .unwrap()
            .to_string(Base58);
        assert_eq!(identity, fixture["expectedIdentityIdentifier"]);
        // Match the create/replace projection and the transfer/purchase helper.
        let alias = format!("{}.{}", label, parent);
        assert_eq!(alias, fixture["expectedAlias"]);
        assert_eq!(alias.chars().count(), 67);
        assert_eq!(label.chars().count(), 62);
        let document = Document::from(transition);
        assert_eq!(build_dpns_alias(document), alias);
        (alias, identity, hash)
    }

    #[tokio::test]
    async fn actual_successful_dpns_7386_preserves_full_alias_and_wire_bytes() {
        actual_dpns_7386_alias().await;
    }

    #[tokio::test]
    #[ignore = "requires explicit PE_ALIAS_TEST_DATABASE_URL for disposable pe_1eb010_alias_test database"]
    async fn postgres_alias_migration_preserves_old_and_actual_values() {
        let config: tokio_postgres::Config = std::env::var("PE_ALIAS_TEST_DATABASE_URL")
            .expect("explicit disposable database URL required")
            .parse()
            .unwrap();
        assert_eq!(config.get_dbname(), Some("pe_1eb010_alias_test"));
        let (mut client, connection) = config.connect(tokio_postgres::NoTls).await.unwrap();
        let connection_task = tokio::spawn(async move { connection.await.unwrap() });
        let (alias, identity, hash) = actual_dpns_7386_alias().await;
        let tx = client.transaction().await.unwrap();
        // Only temporary shadow objects can resolve under this search path.
        tx.batch_execute(
            "SET LOCAL search_path TO pg_temp, pg_catalog;
            CREATE TEMP TABLE identity_aliases (
                id SERIAL PRIMARY KEY, identity_identifier varchar(44) NOT NULL,
                alias varchar(64) NOT NULL, state_transition_hash char(64) NOT NULL
            ) ON COMMIT DROP;
            CREATE INDEX identity_aliases_identifier ON identity_aliases(identity_identifier);",
        )
        .await
        .unwrap();
        let insert = "INSERT INTO identity_aliases(identity_identifier,alias,state_transition_hash) VALUES ($1,$2,$3)";
        let old_alias = "a".repeat(64);
        tx.execute(insert, &[&identity, &old_alias, &hash])
            .await
            .unwrap();
        tx.batch_execute("SAVEPOINT old_width").await.unwrap();
        let error = tx
            .execute(insert, &[&identity, &alias, &hash])
            .await
            .unwrap_err();
        assert_eq!(error.as_db_error().unwrap().code().code(), "22001");
        tx.batch_execute("ROLLBACK TO SAVEPOINT old_width; RELEASE SAVEPOINT old_width")
            .await
            .unwrap();
        tx.batch_execute(include_str!(
            "../../../../migrations/V84__widen_identity_alias.sql"
        ))
        .await
        .unwrap();
        tx.execute(insert, &[&identity, &alias, &hash])
            .await
            .unwrap();
        let rows = tx.query("SELECT identity_identifier, alias, state_transition_hash, pg_typeof(alias)::text FROM identity_aliases ORDER BY id", &[]).await.unwrap();
        assert_eq!(rows.len(), 2);
        for (row, expected) in rows.iter().zip([&old_alias, &alias]) {
            assert_eq!(row.get::<_, String>(0), identity);
            assert_eq!(&row.get::<_, String>(1), expected);
            assert_eq!(row.get::<_, String>(2), hash);
            assert_eq!(row.get::<_, String>(3), "text");
        }
        let indexes: i64 = tx.query_one("SELECT count(*) FROM pg_index WHERE indrelid='identity_aliases'::regclass AND indisvalid", &[]).await.unwrap().get(0);
        assert_eq!(indexes, 2, "primary and identity indexes remain valid");
        // Exercise the same exact-alias lookup/update used by transfers.
        let recipient = Identifier::default().to_string(Base58);
        assert_eq!(tx.execute("UPDATE identity_aliases SET identity_identifier=$1, state_transition_hash=$2 WHERE alias=$3", &[&recipient, &hash, &alias]).await.unwrap(), 1);
        let row = tx
            .query_one(
                "SELECT identity_identifier, alias FROM identity_aliases WHERE alias=$1",
                &[&alias],
            )
            .await
            .unwrap();
        assert_eq!(row.get::<_, String>(0), recipient);
        assert_eq!(row.get::<_, String>(1), alias);
        tx.rollback().await.unwrap();
        drop(client);
        connection_task.await.unwrap();
    }
}
