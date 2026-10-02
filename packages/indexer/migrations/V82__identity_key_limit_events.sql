-- Limits updates do not bump identity revision or replace keys. Keep their
-- signed changes separately from identity create/update history.
CREATE TABLE identity_key_limit_events (
    state_transition_hash char(64) PRIMARY KEY REFERENCES state_transitions(hash),
    identity_identifier varchar(44) NOT NULL,
    key_id bigint NOT NULL,
    changes jsonb NOT NULL
);
CREATE INDEX identity_key_limit_events_identity ON identity_key_limit_events(identity_identifier);

CREATE TABLE identity_key_limit_state (
    identity_identifier varchar(44) NOT NULL,
    key_id bigint NOT NULL,
    limits jsonb NOT NULL,
    state_transition_hash char(64) NOT NULL REFERENCES state_transitions(hash),
    PRIMARY KEY (identity_identifier, key_id)
);
