DROP INDEX IF EXISTS idx_user_identities_provider_external;
ALTER TABLE user_identities RENAME TO user_identities_v3;
CREATE TABLE user_identities (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    issuer TEXT NOT NULL DEFAULT '',
    collection_id TEXT NOT NULL DEFAULT '',
    external_user_id TEXT NOT NULL,
    email_snapshot TEXT,
    display_name_snapshot TEXT,
    last_seen_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK (
        (provider = 'pocketbase' AND issuer <> '' AND collection_id <> '') OR
        (provider = 'goauthy' AND issuer LIKE 'https://%' AND instr(substr(issuer, 9), '/') = 0 AND instr(issuer, '?') = 0 AND instr(issuer, '#') = 0 AND collection_id = '') OR
        (provider NOT IN ('pocketbase', 'goauthy') AND issuer = '' AND collection_id = '')
    )
);
INSERT INTO user_identities(id,user_id,provider,issuer,collection_id,external_user_id,email_snapshot,display_name_snapshot,last_seen_at,created_at,updated_at)
SELECT id,user_id,provider,issuer,collection_id,external_user_id,email_snapshot,display_name_snapshot,last_seen_at,created_at,updated_at
FROM user_identities_v3;
DROP TABLE user_identities_v3;
CREATE UNIQUE INDEX idx_user_identities_provider_external
    ON user_identities (provider, issuer, collection_id, external_user_id);
