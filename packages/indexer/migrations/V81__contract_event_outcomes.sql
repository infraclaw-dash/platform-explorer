-- Nullable outcomes are intentional: a successful signed event is not a
-- settlement receipt. Original bytes/status/gas remain in state_transitions.
ALTER TABLE contract_moderation_events ADD COLUMN outcome jsonb;

CREATE TABLE contract_fee_claim_events (
    state_transition_hash char(64) PRIMARY KEY REFERENCES state_transitions(hash),
    contract_identifier varchar(44) NOT NULL,
    claimant_identifier varchar(44) NOT NULL,
    pot text NOT NULL CHECK (pot IN ('owner', 'moderators')),
    amount numeric(20,0),
    recipients jsonb,
    outcome jsonb NOT NULL
);
CREATE INDEX contract_fee_claim_events_contract ON contract_fee_claim_events(contract_identifier);
