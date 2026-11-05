ALTER TABLE provider_pep_clients ADD COLUMN authorization_mode TEXT NOT NULL DEFAULT 'application' CHECK (authorization_mode IN ('application','user'));
ALTER TABLE provider_pep_clients ADD COLUMN policy_revision INTEGER NOT NULL DEFAULT 1 CHECK (typeof(policy_revision)='integer' AND policy_revision BETWEEN 1 AND 9223372036854775807);
ALTER TABLE provider_pep_client_receipts ADD COLUMN authorization_mode TEXT NOT NULL DEFAULT 'application' CHECK (authorization_mode IN ('application','user'));
ALTER TABLE provider_pep_client_receipts ADD COLUMN policy_revision INTEGER NOT NULL DEFAULT 1 CHECK (typeof(policy_revision)='integer' AND policy_revision BETWEEN 1 AND 9223372036854775807);
