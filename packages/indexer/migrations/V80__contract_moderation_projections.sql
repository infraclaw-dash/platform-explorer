-- Explorer-owned projections only. Existing rows/history are not rewritten.
CREATE TABLE contract_moderation_events (
    state_transition_hash char(64) PRIMARY KEY REFERENCES state_transitions(hash),
    contract_identifier varchar(44) NOT NULL,
    moderator_identifier varchar(44) NOT NULL,
    timestamp_ms bigint NOT NULL,
    action jsonb NOT NULL
);
CREATE INDEX contract_moderation_events_contract ON contract_moderation_events(contract_identifier);

CREATE TABLE contract_moderation_identity_state (
    contract_identifier varchar(44) NOT NULL,
    identity_identifier varchar(44) NOT NULL,
    state jsonb NOT NULL,
    state_transition_hash char(64) NOT NULL REFERENCES state_transitions(hash),
    PRIMARY KEY (contract_identifier, identity_identifier)
);

-- This is Explorer history, not a claim to reproduce a Drive removal proof.
CREATE TABLE contract_moderation_document_removals (
    contract_identifier varchar(44) NOT NULL,
    document_type_name text NOT NULL,
    document_identifier varchar(44) NOT NULL,
    deleted_by_transition char(64) NOT NULL REFERENCES state_transitions(hash),
    removed_document_row_id integer NOT NULL REFERENCES documents(id),
    restored_by_transition char(64) REFERENCES state_transitions(hash),
    PRIMARY KEY (contract_identifier, document_type_name, document_identifier, deleted_by_transition)
);
ALTER TABLE documents ADD COLUMN moderated_at_ms bigint;
ALTER TABLE documents ADD COLUMN moderated_by varchar(44);
