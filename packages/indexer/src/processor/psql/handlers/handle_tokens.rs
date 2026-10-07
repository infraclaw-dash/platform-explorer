use deadpool_postgres::Transaction;
use dpp::prelude::Identifier;
use dpp::state_transition::batch_transition::batched_transition::token_transition::{TokenTransition, TokenTransitionV0Methods};
use dpp::state_transition::batch_transition::token_base_transition::v0::v0_methods::TokenBaseTransitionV0Methods;
use dpp::state_transition::batch_transition::token_burn_transition::v0::v0_methods::TokenBurnTransitionV0Methods;
use dpp::state_transition::batch_transition::token_claim_transition::v0::v0_methods::TokenClaimTransitionV0Methods;
use dpp::state_transition::batch_transition::token_config_update_transition::v0::v0_methods::TokenConfigUpdateTransitionV0Methods;
use dpp::state_transition::batch_transition::token_destroy_frozen_funds_transition::v0::v0_methods::TokenDestroyFrozenFundsTransitionV0Methods;
use dpp::state_transition::batch_transition::token_direct_purchase_transition::v0::v0_methods::TokenDirectPurchaseTransitionV0Methods;
use dpp::state_transition::batch_transition::token_emergency_action_transition::v0::v0_methods::TokenEmergencyActionTransitionV0Methods;
use dpp::state_transition::batch_transition::token_freeze_transition::v0::v0_methods::TokenFreezeTransitionV0Methods;
use dpp::state_transition::batch_transition::token_mint_transition::v0::v0_methods::TokenMintTransitionV0Methods;
use dpp::state_transition::batch_transition::token_set_price_for_direct_purchase_transition::v0::v0_methods::TokenSetPriceForDirectPurchaseTransitionV0Methods;
use dpp::state_transition::batch_transition::token_transfer_transition::v0::v0_methods::TokenTransferTransitionV0Methods;
use dpp::state_transition::batch_transition::token_unfreeze_transition::v0::v0_methods::TokenUnfreezeTransitionV0Methods;
use crate::processor::psql::PSQLProcessor;

/// Public token-pool fields only. Private transfer amounts and claim outcomes
/// are not observable from the signed transition and must remain unknown.
fn pool_token_projection(
    transition: &TokenTransition,
) -> Option<(Option<u64>, Option<String>, Option<Identifier>)> {
    use dpp::state_transition::batch_transition::batched_transition::{
        token_burn_from_pool_transition::TokenBurnFromPoolTransition,
        token_claim_to_pool_transition::TokenClaimToPoolTransition,
        token_direct_purchase_to_pool_transition::TokenDirectPurchaseToPoolTransition,
        token_mint_to_pool_transition::TokenMintToPoolTransition,
        token_shield_transition::TokenShieldTransition,
        token_unshield_transition::TokenUnshieldTransition,
    };
    match transition {
        TokenTransition::Shield(TokenShieldTransition::V0(st)) => {
            Some((Some(st.amount), None, None))
        }
        TokenTransition::Unshield(TokenUnshieldTransition::V0(st)) => {
            Some((Some(st.amount), None, Some(st.recipient_id)))
        }
        TokenTransition::ShieldedTransfer(_) => Some((None, None, None)),
        TokenTransition::MintToPool(TokenMintToPoolTransition::V0(st)) => {
            Some((Some(st.amount), st.public_note.clone(), None))
        }
        TokenTransition::BurnFromPool(TokenBurnFromPoolTransition::V0(st)) => {
            Some((Some(st.amount), st.public_note.clone(), None))
        }
        TokenTransition::ClaimToPool(TokenClaimToPoolTransition::V0(st)) => {
            Some((None, st.public_note.clone(), None))
        }
        TokenTransition::DirectPurchaseToPool(TokenDirectPurchaseToPoolTransition::V0(st)) => {
            Some((Some(st.token_count), None, None))
        }
        _ => None,
    }
}

impl PSQLProcessor {
    pub async fn handle_token_transition(
        &self,
        transition: TokenTransition,
        owner_id: Identifier,
        st_hash: String,
        sql_transaction: &Transaction<'_>,
    ) -> () {
        let data_contract_identifier = transition.base().data_contract_id();

        self.handle_data_contract_transition(
            Some(st_hash.clone()),
            data_contract_identifier,
            sql_transaction,
        )
        .await;

        self.dao
            .token_holder(owner_id, transition.token_id(), &sql_transaction)
            .await
            .unwrap();

        match transition.clone() {
            TokenTransition::Shield(_)
            | TokenTransition::Unshield(_)
            | TokenTransition::ShieldedTransfer(_)
            | TokenTransition::MintToPool(_)
            | TokenTransition::BurnFromPool(_)
            | TokenTransition::ClaimToPool(_)
            | TokenTransition::DirectPurchaseToPool(_) => {
                let (amount, note, recipient) = pool_token_projection(&transition).unwrap();
                // Preserve action discriminant/token link and all original proof
                // bytes in the enclosing state transition, as for existing tokens.
                self.dao
                    .token_transition(
                        transition.clone(),
                        amount,
                        note.as_ref(),
                        owner_id,
                        recipient,
                        st_hash.clone(),
                        sql_transaction,
                    )
                    .await
                    .unwrap();
                if let Some(recipient) = recipient {
                    self.dao
                        .token_holder(recipient, transition.token_id(), sql_transaction)
                        .await
                        .unwrap();
                    self.dao
                        .set_state_transition_recipient(recipient, st_hash.clone(), sql_transaction)
                        .await
                        .unwrap();
                }
            }
            TokenTransition::Mint(mint) => {
                self.dao
                    .token_transition(
                        transition.clone(),
                        Some(mint.amount()),
                        mint.public_note(),
                        owner_id,
                        mint.issued_to_identity_id(),
                        st_hash.clone(),
                        sql_transaction,
                    )
                    .await
                    .unwrap();

                if mint.issued_to_identity_id().is_some() {
                    self.dao
                        .token_holder(
                            mint.issued_to_identity_id().unwrap(),
                            transition.token_id(),
                            &sql_transaction,
                        )
                        .await
                        .unwrap();

                    self.dao
                        .set_state_transition_recipient(
                            mint.issued_to_identity_id().unwrap(),
                            st_hash.clone(),
                            sql_transaction,
                        )
                        .await
                        .unwrap();
                }
            }
            TokenTransition::Burn(burn) => self
                .dao
                .token_transition(
                    transition.clone(),
                    Some(burn.burn_amount()),
                    burn.public_note(),
                    owner_id,
                    None,
                    st_hash.clone(),
                    sql_transaction,
                )
                .await
                .unwrap(),
            TokenTransition::Transfer(transfer) => {
                self.dao
                    .token_transition(
                        transition.clone(),
                        Some(transfer.amount()),
                        transfer.public_note(),
                        owner_id,
                        Some(transfer.recipient_id()),
                        st_hash.clone(),
                        sql_transaction,
                    )
                    .await
                    .unwrap();

                self.dao
                    .token_holder(
                        transfer.recipient_id(),
                        transition.token_id(),
                        &sql_transaction,
                    )
                    .await
                    .unwrap();

                self.dao
                    .set_state_transition_recipient(
                        transfer.recipient_id(),
                        st_hash.clone(),
                        sql_transaction,
                    )
                    .await
                    .unwrap();
            }
            TokenTransition::Freeze(freeze) => {
                self.dao
                    .token_transition(
                        transition.clone(),
                        None,
                        freeze.public_note(),
                        owner_id,
                        Some(freeze.frozen_identity_id()),
                        st_hash.clone(),
                        sql_transaction,
                    )
                    .await
                    .unwrap();

                self.dao
                    .token_holder(
                        freeze.frozen_identity_id(),
                        transition.token_id(),
                        &sql_transaction,
                    )
                    .await
                    .unwrap();
            }
            TokenTransition::Unfreeze(unfreeze) => {
                self.dao
                    .token_transition(
                        transition.clone(),
                        None,
                        unfreeze.public_note(),
                        owner_id,
                        Some(unfreeze.frozen_identity_id()),
                        st_hash.clone(),
                        sql_transaction,
                    )
                    .await
                    .unwrap();

                self.dao
                    .token_holder(
                        unfreeze.frozen_identity_id(),
                        transition.token_id(),
                        &sql_transaction,
                    )
                    .await
                    .unwrap();
            }
            TokenTransition::DestroyFrozenFunds(destroy_frozen_funds) => self
                .dao
                .token_transition(
                    transition.clone(),
                    None,
                    destroy_frozen_funds.public_note(),
                    owner_id,
                    Some(destroy_frozen_funds.frozen_identity_id()),
                    st_hash.clone(),
                    sql_transaction,
                )
                .await
                .unwrap(),
            TokenTransition::EmergencyAction(emergency_action) => self
                .dao
                .token_transition(
                    transition.clone(),
                    None,
                    emergency_action.public_note(),
                    owner_id,
                    None,
                    st_hash.clone(),
                    sql_transaction,
                )
                .await
                .unwrap(),
            TokenTransition::ConfigUpdate(config_update) => self
                .dao
                .token_transition(
                    transition.clone(),
                    None,
                    config_update.public_note(),
                    owner_id,
                    None,
                    st_hash.clone(),
                    sql_transaction,
                )
                .await
                .unwrap(),
            TokenTransition::Claim(claim) => self
                .dao
                .token_transition(
                    transition.clone(),
                    None,
                    claim.public_note(),
                    owner_id,
                    None,
                    st_hash.clone(),
                    sql_transaction,
                )
                .await
                .unwrap(),
            TokenTransition::DirectPurchase(purchase) => self
                .dao
                .token_transition(
                    transition.clone(),
                    Some(purchase.token_count()),
                    None,
                    owner_id,
                    None,
                    st_hash.clone(),
                    sql_transaction,
                )
                .await
                .unwrap(),
            TokenTransition::SetPriceForDirectPurchase(set_price) => self
                .dao
                .token_transition(
                    transition.clone(),
                    None,
                    set_price.public_note(),
                    owner_id,
                    None,
                    st_hash.clone(),
                    sql_transaction,
                )
                .await
                .unwrap(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpp::state_transition::batch_transition::batched_transition::{
        token_claim_to_pool_transition::TokenClaimToPoolTransition,
        token_shielded_transfer_transition::TokenShieldedTransferTransition,
        token_unshield_transition::{TokenUnshieldTransition, TokenUnshieldTransitionV0},
    };

    #[test]
    fn token_pool_projection_distinguishes_public_values_from_hidden_amounts() {
        let recipient = Identifier::new([9; 32]);
        let unshield =
            TokenTransition::Unshield(TokenUnshieldTransition::V0(TokenUnshieldTransitionV0 {
                amount: 123,
                recipient_id: recipient,
                ..Default::default()
            }));
        assert_eq!(
            pool_token_projection(&unshield),
            Some((Some(123), None, Some(recipient)))
        );
        assert_eq!(
            pool_token_projection(&TokenTransition::ShieldedTransfer(
                TokenShieldedTransferTransition::default()
            )),
            Some((None, None, None))
        );
        assert_eq!(
            pool_token_projection(&TokenTransition::ClaimToPool(
                TokenClaimToPoolTransition::default()
            )),
            Some((None, None, None))
        );
    }
}
