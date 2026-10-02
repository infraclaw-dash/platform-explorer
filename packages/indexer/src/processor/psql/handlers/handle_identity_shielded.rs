//! Identity/pool movements expose a gross pool amount and an identity transfer.
//! Count the transition amount once, not once per projection table.
use super::super::{PSQLProcessor, ProcessorError};
use deadpool_postgres::Transaction;
use dpp::platform_value::string_encoding::Encoding::Base58;
use dpp::prelude::Identifier;

fn top_up_net(gross: u64, actions: usize, protocol: u32) -> Result<u64, String> {
    let version = dpp::version::PlatformVersion::get(protocol).map_err(|e| e.to_string())?;
    // This is the unchanged published helper used by Drive's transformer, not
    // an Explorer estimate from gas, a copied fee algorithm or proof validation.
    let fee = dpp::shielded::compute_shielded_identity_top_up_fee(actions, version)
        .map_err(|e| e.to_string())?;
    gross.checked_sub(fee).ok_or_else(|| "shielded identity top-up gross amount is below the protocol fee".into())
}

impl PSQLProcessor {
    pub async fn handle_identity_shielded_amount(
        &self,
        identity: Identifier,
        gross: u64,
        top_up_actions: Option<usize>,
        hash: String,
        height: i32,
        index: usize,
        tx: &Transaction<'_>,
    ) -> Result<(), ProcessorError> {
        let failure = |detail: String| ProcessorError::TransactionError {
            height,
            index,
            stage: "identity shielded projection",
            detail,
        };
        let identifier = identity.to_string(Base58);
        let (transfer_amount, sender, recipient) = match top_up_actions {
            Some(actions) => {
                let row = tx.query_one("SELECT app_version FROM blocks WHERE height=$1", &[&height])
                    .await.map_err(|e| failure(e.to_string()))?;
                let protocol = u32::try_from(row.get::<_, i32>(0)).map_err(|e| failure(e.to_string()))?;
                (top_up_net(gross, actions, protocol).map_err(failure)?, None, Some(identifier))
            }
            None => (gross, Some(identifier), None),
        };
        let gross = i64::try_from(gross).map_err(|e| failure(e.to_string()))?;
        let transfer_amount = i64::try_from(transfer_amount).map_err(|e| failure(e.to_string()))?;
        tx.execute("INSERT INTO shielded_transitions(state_transition_id,state_transition_type,amount) SELECT id,type,$2 FROM state_transitions WHERE hash=$1", &[&hash, &gross])
            .await.map_err(|e| failure(e.to_string()))?;
        tx.execute("INSERT INTO transfers(amount,sender,recipient,state_transition_hash) VALUES ($1,$2,$3,$4)", &[&transfer_amount, &sender, &recipient, &hash])
            .await.map_err(|e| failure(e.to_string()))?;
        // Amount is the signed gross pool movement; the transfer separately
        // carries net identity credit for type22, just as an unshield does.
        tx.execute("UPDATE state_transitions SET amount=$1,recipient=$2 WHERE hash=$3", &[&gross, &recipient, &hash])
            .await.map_err(|e| failure(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_up_distinguishes_gross_pool_amount_and_net_identity_credit() {
        let gross = 100_000_000_000;
        let net = top_up_net(gross, 2, 14).unwrap();
        assert!(net > 0 && net < gross);
        assert!(top_up_net(0, 2, 14).is_err(), "never saturate an invalid amount to a fabricated zero credit");
        assert!(top_up_net(gross, 2, u32::MAX).is_err(), "unknown protocol cannot use guessed fees");
    }
}
