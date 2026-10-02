use crate::decoder::decoder::StateTransitionDecoder;
use crate::entities::block::Block;
use crate::entities::validator::Validator;
use crate::processor::psql::{PSQLProcessor, ProcessorError};
use base64::engine::general_purpose;
use base64::Engine;
use dpp::state_transition::StateTransition;

/// Decode before opening the SQL transaction. A bad transaction must not panic,
/// be skipped, or leave a partially indexed block behind.
pub(crate) async fn decode_block_transaction(
    decoder: &StateTransitionDecoder,
    data: &str,
    height: i32,
    index: usize,
) -> Result<StateTransition, ProcessorError> {
    let bytes = general_purpose::STANDARD.decode(data).map_err(|error| {
        ProcessorError::TransactionError {
            height,
            index,
            stage: "base64 decode",
            detail: error.to_string(),
        }
    })?;
    decoder
        .decode(bytes)
        .await
        .map_err(|error| ProcessorError::TransactionError {
            height,
            index,
            stage: "state transition decode",
            detail: error.to_string(),
        })
}

impl PSQLProcessor {
    pub async fn handle_block(
        &self,
        block: Block,
        validators: Vec<Validator>,
    ) -> Result<(), ProcessorError> {
        let height = block.header.height;
        if self.dao.get_block_header_by_height(height).await?.is_some() {
            println!("Block at the height {height} has been already processed");
            return Ok(());
        }

        let mut transitions = Vec::with_capacity(block.txs.len());
        for (index, tx) in block.txs.iter().enumerate() {
            transitions
                .push(decode_block_transaction(&self.decoder, &tx.data, height, index).await?);
        }

        let mut client = self.dao.connection_pool.get().await?;
        let sql_transaction =
            client
                .transaction()
                .await
                .map_err(|error| ProcessorError::BlockDatabaseError {
                    height,
                    operation: "begin",
                    detail: error.to_string(),
                })?;

        // Initialization and block 1 are atomic too: a failed first block must
        // not commit initial rows separately and duplicate them on retry.
        if height == 1 {
            self.handle_init_chain(height, &sql_transaction).await?;
        } else if !self.system_contracts_seeded.get() {
            self.ensure_system_contracts(height, &sql_transaction)
                .await?;
        }

        for validator in validators {
            self.handle_validator(validator, &sql_transaction).await?;
        }
        let block_hash = self.dao.create_block(block.header, &sql_transaction).await;

        println!("Processing block at height {height}");
        for (index, (state_transition, tx)) in transitions.into_iter().zip(block.txs).enumerate() {
            self.handle_st(
                block_hash.clone(),
                height,
                index as u32,
                state_transition,
                tx,
                &sql_transaction,
            )
            .await?;
        }

        sql_transaction
            .commit()
            .await
            .map_err(|error| ProcessorError::BlockDatabaseError {
                height,
                operation: "commit",
                detail: error.to_string(),
            })?;
        self.system_contracts_seeded.set(true);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn invalid_base64_returns_block_and_transaction_context() {
        let error = decode_block_transaction(&StateTransitionDecoder::new(), "!not-base64", 325, 7)
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            ProcessorError::TransactionError {
                height: 325,
                index: 7,
                stage: "base64 decode",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn invalid_state_transition_returns_context_without_panicking() {
        let error = decode_block_transaction(&StateTransitionDecoder::new(), "AA==", 326, 2)
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            ProcessorError::TransactionError {
                height: 326,
                index: 2,
                stage: "state transition decode",
                ..
            }
        ));
    }
}
