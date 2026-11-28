CREATE TABLE provider_grpc_capability_sets (
    provider_id TEXT NOT NULL REFERENCES service_providers(id),
    generation INTEGER NOT NULL CHECK (typeof(generation)='integer' AND generation>0),
    catalog_digest TEXT NOT NULL,
    canonical_json TEXT NOT NULL CHECK (length(CAST(canonical_json AS BLOB))<=49152 AND json_valid(canonical_json)),
    created_at TEXT NOT NULL,
    PRIMARY KEY (provider_id, generation),
    UNIQUE (provider_id, generation, catalog_digest)
);

CREATE TABLE provider_grpc_capability_active (
    provider_id TEXT PRIMARY KEY NOT NULL,
    generation INTEGER NOT NULL,
    catalog_digest TEXT NOT NULL,
    activated_at TEXT NOT NULL,
    FOREIGN KEY (provider_id, generation, catalog_digest)
        REFERENCES provider_grpc_capability_sets(provider_id, generation, catalog_digest)
);

CREATE TRIGGER provider_grpc_active_action_delete BEFORE DELETE ON actions
WHEN EXISTS (
    SELECT 1 FROM provider_grpc_capability_active p
    JOIN provider_grpc_capability_sets s USING (provider_id, generation, catalog_digest)
    JOIN json_each(s.canonical_json,'$.capabilities') c
    WHERE p.provider_id=OLD.provider_id AND c.value->>'actionId'=OLD.id
)
BEGIN
    SELECT RAISE(ABORT, 'action is referenced by an active gRPC capability catalog');
END;

CREATE TRIGGER provider_grpc_active_action_rekey BEFORE UPDATE OF id, provider_id ON actions
WHEN (OLD.id IS NOT NEW.id OR OLD.provider_id IS NOT NEW.provider_id) AND EXISTS (
    SELECT 1 FROM provider_grpc_capability_active p
    JOIN provider_grpc_capability_sets s USING (provider_id, generation, catalog_digest)
    JOIN json_each(s.canonical_json,'$.capabilities') c
    WHERE p.provider_id=OLD.provider_id AND c.value->>'actionId'=OLD.id
)
BEGIN
    SELECT RAISE(ABORT, 'action is referenced by an active gRPC capability catalog');
END;
