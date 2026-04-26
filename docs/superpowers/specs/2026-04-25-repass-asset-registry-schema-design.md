# REPass Asset Registry Schema Design

## Decision

REPass v2 uses `asset_id` as the canonical resource identity for physical energy facilities.
The user-facing term can remain plant or 발전소, but the database authority is `assets`.

Country and market identifiers are not canonical resource IDs. They live in profiles,
registry projects, data sources, credentials, and certificates.

## Problem

The current schema treats `plants.id` and Korean RPS `facility_code` as the practical
center of the model. That works for a Korean RPS-only system, but it breaks when one
facility has multiple registry projects, utility meter identifiers, certificate systems,
or contract-specific data feeds.

The old design also mixes physical facility data with Korean RPS fields such as
`facility_code`, `contract_no`, `rec_start_date`, and `confirm_issued`.

## Target Model

Core tables:

- `assets`: physical facility, owned by an organization.
- `asset_profiles`: country or market-specific view of an asset.
- `registries`: external registry, utility, carbon, market, or portal system.
- `registry_accounts`: organization account in a registry.
- `registry_projects`: registry-side project/device/meter mapped to an asset.
- `data_sources`: source of measured, billed, registry, or manual data.
- `generation_data`: source-attributed generation readings.
- `energy_certificates`: serial-based energy certificate records.
- `auth_credentials`: encrypted credential abstraction for certificate, OAuth, API key, and login credentials.
- `credential_bindings`: binding between credentials and registry accounts.
- `country_capabilities`: country-level feature capability flags.

Authorization:

- `role_bindings.resource_type = 'asset'` and `resource_id = assets.id` for facility permissions.
- `effective_*_asset_*` replaces `effective_*_plant_*`.
- Registry account, credential, and data source checks are additional execution checks on top of asset permission.

## Compatibility

This is a breaking schema change because the service has not launched.
The first implementation can keep handler names and route names temporarily if needed,
but internal storage and permission semantics should move to `asset_id`.

Korean RPS maps as:

- `assets`: physical 발전소.
- `asset_profiles.profile_type = 'kr_rps'`.
- `asset_profiles.external_id = facility_code`.
- `asset_profiles.metadata_json`: KR RPS fields that are not global.
- `registries.provider = 'korea_energy_rps'`.
- `registry_projects.external_project_id = facility_code`.
- Korean 공동인증서 becomes `auth_credentials.credential_type = 'certificate'`.

## Migration Shape

1. Add asset-centered schema and schema tests.
2. Move permission cache names and resource validation from plant to asset.
3. Update domain types and persistence functions.
4. Keep Korean RPS adapter as the first country adapter.
5. Update API naming after the storage boundary is stable.

## Non-Goals

- Do not build every Southeast Asian adapter in this slice.
- Do not model every market-specific certificate lifecycle yet.
- Do not add unbounded generic metadata as the only representation for typed business fields.
