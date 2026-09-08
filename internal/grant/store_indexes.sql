                CREATE INDEX IF NOT EXISTS idx_organization_memberships_org_user
                    ON organization_memberships (organization_id, user_id);
                CREATE UNIQUE INDEX IF NOT EXISTS idx_organization_party_links_org_relation
                    ON organization_party_links (organization_id, relation_type);
                CREATE INDEX IF NOT EXISTS idx_organization_party_links_party
                    ON organization_party_links (party_id, relation_type);
                CREATE UNIQUE INDEX IF NOT EXISTS idx_parties_business_no
                    ON parties (business_no);
                CREATE UNIQUE INDEX IF NOT EXISTS idx_user_identities_provider_external
                    ON user_identities (provider, external_user_id);
                CREATE INDEX IF NOT EXISTS idx_auth_identity_cache_expires_at
                    ON auth_identity_cache (expires_at);
                DROP INDEX IF EXISTS idx_applications_rauthy_client_id_unique;
                CREATE UNIQUE INDEX idx_applications_rauthy_client_id_unique
                    ON applications (rauthy_client_id)
                    WHERE rauthy_client_id IS NOT NULL
                      AND TRIM(rauthy_client_id) <> '';
                CREATE INDEX IF NOT EXISTS idx_group_memberships_group_member
                    ON group_memberships (group_id, organization_membership_id);
                CREATE INDEX IF NOT EXISTS idx_agreements_grantor_grantee
                    ON agreements (grantor_organization_id, grantee_organization_id, status);
                CREATE INDEX IF NOT EXISTS idx_service_providers_active
                    ON service_providers (active, id);
                CREATE INDEX IF NOT EXISTS idx_actions_code
                    ON actions (code);
                DROP INDEX IF EXISTS idx_actions_provider;
                CREATE UNIQUE INDEX IF NOT EXISTS idx_actions_provider_code_unique
                    ON actions (provider_id, code);
                CREATE UNIQUE INDEX IF NOT EXISTS idx_roles_global_code
                    ON roles (code)
                    WHERE organization_id IS NULL;
                CREATE INDEX IF NOT EXISTS idx_role_action_field_views_lookup
                    ON role_action_field_views (
                        role_id,
                        action_id,
                        resource_type,
                        schema_key
                    );
                CREATE INDEX IF NOT EXISTS idx_role_bindings_subject_resource_state
                    ON role_bindings (subject_type, subject_id, provider_id, resource_type, resource_id, state);
                DROP INDEX IF EXISTS idx_role_bindings_logical_unique;
                CREATE UNIQUE INDEX IF NOT EXISTS idx_role_bindings_logical_unique
                    ON role_bindings (
                        subject_type,
                        subject_id,
                        subject_organization_id,
                        provider_id,
                        resource_type,
                        resource_id,
                        role_id
                    );
                CREATE INDEX IF NOT EXISTS idx_application_grants_user_application
                    ON application_grants (user_id, application_id, organization_id, status);
                CREATE UNIQUE INDEX IF NOT EXISTS idx_application_grants_active_unique
                    ON application_grants (user_id, application_id, organization_id)
                    WHERE status = 'active';
                CREATE INDEX IF NOT EXISTS idx_provider_http_capabilities_match
                    ON provider_http_capabilities
                    (provider_id, generation, enabled, method, path_template);
                CREATE INDEX IF NOT EXISTS idx_provider_http_activations_provider
                    ON provider_http_capability_activations
                    (provider_id, activation_sequence);
                CREATE UNIQUE INDEX IF NOT EXISTS idx_provider_http_activations_receipt
                    ON provider_http_capability_activations (activation_receipt_id)
                    WHERE activation_receipt_id IS NOT NULL;
                CREATE INDEX IF NOT EXISTS idx_provider_pep_clients_provider_lifecycle
                    ON provider_pep_clients (provider_id, lifecycle, valid_until);
