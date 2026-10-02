use crate::processor::psql::{PSQLProcessor, ProcessorError};
use data_contracts::SystemDataContract;
use deadpool_postgres::Transaction;
use dpp::version::PlatformVersion;

impl PSQLProcessor {
    /// Fill missing protocol-14 system contracts on both a fresh index and a
    /// retained pre-upgrade index. Existing rows and human data are not replaced.
    pub async fn ensure_system_contracts(
        &self,
        height: i32,
        sql_transaction: &Transaction<'_>,
    ) -> Result<(), ProcessorError> {
        let version = PlatformVersion::get(14).expect("pinned DPP supports protocol 14");
        for contract in SystemDataContract::ALL {
            // Reserved discriminant, explicitly never registered by Platform.
            if contract == SystemDataContract::FeatureFlags {
                continue;
            }
            contract
                .source(version)
                .map_err(|error| ProcessorError::TransactionError {
                    height,
                    index: 0,
                    stage: "system contract initialization",
                    detail: format!("{contract:?}: {error}"),
                })?;
            if self
                .dao
                .get_data_contract_by_identifier(contract.id(), sql_transaction)
                .await?
                .is_none()
            {
                self.process_system_data_contract(contract, sql_transaction)
                    .await;
            }
        }
        Ok(())
    }

    pub async fn handle_init_chain(
        &self,
        height: i32,
        sql_transaction: &Transaction<'_>,
    ) -> Result<(), ProcessorError> {
        self.ensure_system_contracts(height, sql_transaction)
            .await?;
        self.process_state_transition_duplicates(sql_transaction)
            .await;
        Ok(())
    }
}
