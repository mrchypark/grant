use super::core::now_string;
use super::*;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

const CATALOG_SCHEMA_VERSION: u64 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpCapabilityAccessMode {
    PublicAuthn,
    Authorized,
}

impl HttpCapabilityAccessMode {
    fn as_str(&self) -> &'static str {
        match self {
            Self::PublicAuthn => "public_authn",
            Self::Authorized => "authorized",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HttpCapabilityResource {
    #[serde(rename = "type")]
    pub resource_type: String,
    pub id: HttpResourceExtractor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum HttpResourceExtractor {
    PathSegment { name: String, format: String },
    QueryScalar { name: String, format: String },
    JsonPointer { pointer: String, format: String },
    Constant { value: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HttpCapability {
    pub capability_id: String,
    pub version: u64,
    pub enabled: bool,
    pub access_mode: HttpCapabilityAccessMode,
    pub method: String,
    pub path_template: String,
    pub action_id: Option<String>,
    pub resource: Option<HttpCapabilityResource>,
    pub query: HttpQueryPolicy,
    pub request: HttpRequestPolicy,
    pub response: HttpResponsePolicy,
    pub contract_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase", deny_unknown_fields)]
pub enum HttpQueryPolicy {
    None {},
    Exact { parameters: Vec<HttpQueryParameter> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HttpQueryParameter {
    pub name: String,
    pub format: String,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpRequestBodyMode {
    None,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HttpRequestPolicy {
    pub body: HttpRequestBodyMode,
    pub max_bytes: u64,
    pub forward_headers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpRedirectPolicy {
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HttpResponsePolicy {
    pub max_bytes: u64,
    pub forward_headers: Vec<String>,
    pub redirects: HttpRedirectPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StageHttpCapabilityGeneration {
    pub schema_version: u64,
    pub generation: u64,
    pub expected_latest_generation: Option<u64>,
    pub capabilities: Vec<HttpCapability>,
    pub catalog_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedHttpCapabilityGeneration {
    pub staged: bool,
    pub provider_id: String,
    pub generation: u64,
    pub catalog_digest: String,
    pub total_count: usize,
    pub enabled_count: usize,
    pub disabled_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivateHttpCapabilityGeneration {
    pub expected_generation: Option<u64>,
    pub expected_catalog_digest: Option<String>,
    pub target_generation: u64,
    pub target_catalog_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpCapabilityActivation {
    pub provider_id: String,
    pub activation_receipt_id: String,
    pub activation_sequence: u64,
    pub from_generation: Option<u64>,
    pub to_generation: u64,
    pub catalog_digest: String,
    pub activated_at: String,
    pub activated_by: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderPepClientLifecycle {
    Active,
    Disabled,
    Deleted,
}

impl ProviderPepClientLifecycle {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled",
            Self::Deleted => "deleted",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpsertProviderPepClient {
    pub application_id: String,
    pub lifecycle: ProviderPepClientLifecycle,
    pub valid_until: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderPepClient {
    pub pep_client_id: String,
    pub application_id: String,
    pub provider_id: String,
    pub lifecycle: String,
    pub valid_until: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveHttpCapabilityCatalog {
    pub schema_version: u64,
    pub provider_id: String,
    pub pep_client_id: String,
    pub generation: u64,
    pub catalog_digest: String,
    pub capabilities: Vec<HttpCapability>,
    pub issued_at: String,
    pub expires_at: String,
}

#[derive(Debug)]
struct CapabilitySetRow {
    catalog_digest: String,
    canonical_json: String,
}

impl From<&mut Row<'_>> for CapabilitySetRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            catalog_digest: row.get("catalog_digest"),
            canonical_json: row.get("canonical_json"),
        }
    }
}

#[derive(Debug)]
struct ActiveCapabilityRow {
    generation: i64,
    catalog_digest: String,
}

#[derive(Debug)]
struct CapabilityActivationRow {
    activation_sequence: i64,
}

impl From<&mut Row<'_>> for CapabilityActivationRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            activation_sequence: row.get("activation_sequence"),
        }
    }
}

impl From<&mut Row<'_>> for ActiveCapabilityRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            generation: row.get("generation"),
            catalog_digest: row.get("catalog_digest"),
        }
    }
}

#[derive(Debug)]
struct PepClientRow {
    pep_client_id: String,
    application_id: String,
    provider_id: String,
    lifecycle: String,
    valid_until: Option<String>,
    created_at: String,
    updated_at: String,
}

impl From<&mut Row<'_>> for PepClientRow {
    fn from(row: &mut Row<'_>) -> Self {
        Self {
            pep_client_id: row.get("pep_client_id"),
            application_id: row.get("application_id"),
            provider_id: row.get("provider_id"),
            lifecycle: row.get("lifecycle"),
            valid_until: row.get("valid_until"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        }
    }
}

impl From<PepClientRow> for ProviderPepClient {
    fn from(row: PepClientRow) -> Self {
        Self {
            pep_client_id: row.pep_client_id,
            application_id: row.application_id,
            provider_id: row.provider_id,
            lifecycle: row.lifecycle,
            valid_until: row.valid_until,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

impl Database {
    pub async fn stage_http_capability_generation(
        &self,
        provider_id: &str,
        request: StageHttpCapabilityGeneration,
    ) -> Result<StagedHttpCapabilityGeneration> {
        validate_generation(provider_id, &request)?;
        let canonical = canonical_generation_json(provider_id, &request)?;
        let actual_digest = sha256_digest(canonical.as_bytes());
        if request.catalog_digest != actual_digest {
            anyhow::bail!("catalogDigest does not match canonical generation");
        }
        let existing: Vec<CapabilitySetRow> = self
            .query_rows_with(
                ReadConsistency::Strong,
                "SELECT catalog_digest,canonical_json FROM provider_http_capability_sets WHERE provider_id=$1 AND generation=$2",
                params!(provider_id.to_owned(), request.generation as i64),
                "HTTP capability generation",
            )
            .await?;
        if let Some(existing) = existing.into_iter().next() {
            if existing.catalog_digest != request.catalog_digest
                || existing.canonical_json != canonical
            {
                anyhow::bail!("HTTP capability generation conflict");
            }
            return Ok(staged_result(false, provider_id, &request));
        }

        let assertion = Uuid::new_v4().simple().to_string();
        let created_at = now_string();
        let mut statements = vec![
            (
                Cow::Borrowed(
                    "INSERT INTO provider_http_capability_assertions (id,valid) SELECT $1,CASE WHEN
                     EXISTS (SELECT 1 FROM service_providers WHERE id=$2 AND active=1)
                     AND COALESCE((SELECT MAX(generation) FROM provider_http_capability_sets WHERE provider_id=$2),0)+1=$3
                     AND (($4 IS NULL AND NOT EXISTS (SELECT 1 FROM provider_http_capability_sets WHERE provider_id=$2))
                          OR $4=(SELECT MAX(generation) FROM provider_http_capability_sets WHERE provider_id=$2))
                     THEN 1 ELSE 0 END",
                ),
                params!(assertion.clone(), provider_id.to_owned(), request.generation as i64, request.expected_latest_generation.map(|v| v as i64)),
            ),
            (
                Cow::Borrowed("INSERT INTO provider_http_capability_sets (provider_id,generation,catalog_digest,canonical_json,created_at) VALUES ($1,$2,$3,$4,$5)"),
                params!(provider_id.to_owned(), request.generation as i64, request.catalog_digest.clone(), canonical, created_at),
            ),
        ];
        for capability in &request.capabilities {
            statements.push((
                Cow::Borrowed("INSERT INTO provider_http_capabilities (provider_id,generation,capability_id,version,enabled,access_mode,method,path_template,action_id,resource_type,extractor_json,query_policy_json,request_policy_json,response_policy_json,contract_digest) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)"),
                params!(
                    provider_id.to_owned(),
                    request.generation as i64,
                    capability.capability_id.clone(),
                    capability.version as i64,
                    if capability.enabled { 1_i64 } else { 0_i64 },
                    capability.access_mode.as_str().to_owned(),
                    capability.method.clone(),
                    capability.path_template.clone(),
                    capability.action_id.clone(),
                    capability.resource.as_ref().map(|r| r.resource_type.clone()),
                    serde_json::to_string(&capability.resource.as_ref().map(|r| &r.id))?,
                    serde_json::to_string(&capability.query)?,
                    serde_json::to_string(&capability.request)?,
                    serde_json::to_string(&capability.response)?,
                    capability.contract_digest.clone(),
                ),
            ));
        }
        statements.push((
            Cow::Borrowed("DELETE FROM provider_http_capability_assertions WHERE id=$1"),
            params!(assertion),
        ));
        self.runtime
            .transaction(statements)
            .await
            .context("stage HTTP capability generation")?;
        Ok(staged_result(true, provider_id, &request))
    }

    pub async fn upsert_provider_pep_client(
        &self,
        provider_id: &str,
        pep_client_id: &str,
        input: UpsertProviderPepClient,
        changed_by: &str,
    ) -> Result<ProviderPepClient> {
        require_token("providerId", provider_id)?;
        require_token("pepClientId", pep_client_id)?;
        require_token("applicationId", &input.application_id)?;
        validate_expiry(input.valid_until.as_deref())?;
        let now = now_string();
        let receipt_id = Uuid::new_v4().simple().to_string();
        self.runtime.transaction([
            (
                Cow::Borrowed("INSERT INTO provider_http_capability_assertions (id,valid) SELECT $1,CASE WHEN EXISTS (SELECT 1 FROM service_providers WHERE id=$2 AND active=1) AND EXISTS (SELECT 1 FROM applications WHERE id=$3 AND active=1 AND provisioning_status='ready') AND NOT EXISTS (SELECT 1 FROM provider_pep_clients WHERE pep_client_id=$4 AND (provider_id<>$2 OR application_id<>$3)) AND NOT EXISTS (SELECT 1 FROM provider_pep_clients WHERE application_id=$3 AND pep_client_id<>$4) THEN 1 ELSE 0 END"),
                params!(receipt_id.clone(), provider_id.to_owned(), input.application_id.clone(), pep_client_id.to_owned()),
            ),
            (
                Cow::Borrowed("INSERT INTO provider_pep_clients (pep_client_id,application_id,provider_id,lifecycle,valid_until,created_at,updated_at) VALUES ($1,$2,$3,$4,$5,$6,$6) ON CONFLICT(pep_client_id) DO UPDATE SET lifecycle=excluded.lifecycle,valid_until=excluded.valid_until,updated_at=excluded.updated_at WHERE provider_pep_clients.application_id=excluded.application_id AND provider_pep_clients.provider_id=excluded.provider_id"),
                params!(pep_client_id.to_owned(), input.application_id.clone(), provider_id.to_owned(), input.lifecycle.as_str().to_owned(), input.valid_until.clone(), now.clone()),
            ),
            (
                Cow::Borrowed("INSERT INTO provider_pep_client_receipts (receipt_id,pep_client_id,application_id,provider_id,lifecycle,valid_until,changed_at,changed_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)"),
                params!(receipt_id.clone(), pep_client_id.to_owned(), input.application_id, provider_id.to_owned(), input.lifecycle.as_str().to_owned(), input.valid_until, now, changed_by.to_owned()),
            ),
            (Cow::Borrowed("DELETE FROM provider_http_capability_assertions WHERE id=$1"), params!(receipt_id)),
        ]).await.context("upsert provider PEP client")?;
        self.provider_pep_client(provider_id, pep_client_id).await
    }

    pub async fn activate_http_capability_generation(
        &self,
        provider_id: &str,
        input: ActivateHttpCapabilityGeneration,
        activated_by: &str,
    ) -> Result<HttpCapabilityActivation> {
        validate_digest("targetCatalogDigest", &input.target_catalog_digest)?;
        if input.expected_generation.is_some() != input.expected_catalog_digest.is_some() {
            anyhow::bail!("expectedGeneration and expectedCatalogDigest must be supplied together");
        }
        if let Some(digest) = input.expected_catalog_digest.as_deref() {
            validate_digest("expectedCatalogDigest", digest)?;
        }
        let now = now_string();
        let assertion = Uuid::new_v4().simple().to_string();
        let activation_receipt_id = Uuid::new_v4().simple().to_string();
        self.runtime.transaction([
            (
                Cow::Borrowed("INSERT INTO provider_http_capability_assertions (id,valid) SELECT $1,CASE WHEN EXISTS (SELECT 1 FROM provider_http_capability_sets WHERE provider_id=$2 AND generation=$3 AND catalog_digest=$4) AND (($5 IS NULL AND NOT EXISTS (SELECT 1 FROM provider_http_capability_active WHERE provider_id=$2)) OR EXISTS (SELECT 1 FROM provider_http_capability_active WHERE provider_id=$2 AND generation=$5 AND catalog_digest=$6)) AND EXISTS (SELECT 1 FROM provider_pep_clients pc JOIN applications a ON a.id=pc.application_id WHERE pc.provider_id=$2 AND pc.lifecycle='active' AND (pc.valid_until IS NULL OR pc.valid_until>$7) AND a.active=1 AND a.provisioning_status='ready') THEN 1 ELSE 0 END"),
                params!(assertion.clone(), provider_id.to_owned(), input.target_generation as i64, input.target_catalog_digest.clone(), input.expected_generation.map(|v| v as i64), input.expected_catalog_digest.clone(), now.clone()),
            ),
            (
                Cow::Borrowed("INSERT INTO provider_http_capability_activations (activation_receipt_id,provider_id,activation_sequence,from_generation,to_generation,catalog_digest,activated_at,activated_by) VALUES ($1,$2,COALESCE((SELECT MAX(activation_sequence) FROM provider_http_capability_activations WHERE provider_id=$2),0)+1,(SELECT generation FROM provider_http_capability_active WHERE provider_id=$2),$3,$4,$5,$6)"),
                params!(activation_receipt_id.clone(), provider_id.to_owned(), input.target_generation as i64, input.target_catalog_digest.clone(), now.clone(), activated_by.to_owned()),
            ),
            (
                Cow::Borrowed("INSERT INTO provider_http_capability_active (provider_id,generation,catalog_digest,activated_at,activated_by) VALUES ($1,$2,$3,$4,$5) ON CONFLICT(provider_id) DO UPDATE SET generation=excluded.generation,catalog_digest=excluded.catalog_digest,activated_at=excluded.activated_at,activated_by=excluded.activated_by"),
                params!(provider_id.to_owned(), input.target_generation as i64, input.target_catalog_digest.clone(), now.clone(), activated_by.to_owned()),
            ),
            (Cow::Borrowed("DELETE FROM provider_http_capability_assertions WHERE id=$1"), params!(assertion)),
        ]).await.context("activate HTTP capability generation")?;
        let inserted: CapabilityActivationRow = self.query_row_with(
            ReadConsistency::Strong,
            "SELECT activation_sequence FROM provider_http_capability_activations WHERE activation_receipt_id=$1 AND provider_id=$2",
            params!(activation_receipt_id.clone(), provider_id.to_owned()),
            "HTTP capability activation receipt",
        ).await?;
        Ok(HttpCapabilityActivation {
            provider_id: provider_id.to_owned(),
            activation_receipt_id,
            activation_sequence: inserted.activation_sequence as u64,
            from_generation: input.expected_generation,
            to_generation: input.target_generation,
            catalog_digest: input.target_catalog_digest,
            activated_at: now,
            activated_by: activated_by.to_owned(),
        })
    }

    pub async fn active_http_capability_catalog(
        &self,
        provider_id: &str,
        pep_client_id: &str,
    ) -> Result<ActiveHttpCapabilityCatalog> {
        let pep = self.provider_pep_client(provider_id, pep_client_id).await?;
        require_active_pep(&pep)?;
        let active: Vec<ActiveCapabilityRow> = self.query_rows_with(
            ReadConsistency::Strong,
            "SELECT generation,catalog_digest FROM provider_http_capability_active WHERE provider_id=$1",
            params!(provider_id.to_owned()),
            "active HTTP capability pointer",
        ).await?;
        let active = active.into_iter().next().ok_or_else(|| {
            anyhow!("HTTP capability catalog is not active for provider {provider_id}")
        })?;
        let set: CapabilitySetRow = self.query_row_with(
            ReadConsistency::Strong,
            "SELECT catalog_digest,canonical_json FROM provider_http_capability_sets WHERE provider_id=$1 AND generation=$2",
            params!(provider_id.to_owned(), active.generation),
            "active HTTP capability generation",
        ).await?;
        if set.catalog_digest != active.catalog_digest {
            anyhow::bail!("active HTTP capability pointer digest mismatch");
        }
        let stored: CanonicalGeneration = serde_json::from_str(&set.canonical_json)
            .context("decode active HTTP capability generation")?;
        let issued_at = Utc::now();
        Ok(ActiveHttpCapabilityCatalog {
            schema_version: CATALOG_SCHEMA_VERSION,
            provider_id: provider_id.to_owned(),
            pep_client_id: pep_client_id.to_owned(),
            generation: active.generation as u64,
            catalog_digest: active.catalog_digest,
            capabilities: stored.capabilities,
            issued_at: issued_at.to_rfc3339(),
            expires_at: (issued_at + chrono::Duration::seconds(30)).to_rfc3339(),
        })
    }

    pub async fn provider_pep_client(
        &self,
        provider_id: &str,
        pep_client_id: &str,
    ) -> Result<ProviderPepClient> {
        let rows: Vec<PepClientRow> = self.query_rows_with(
            ReadConsistency::Strong,
            "SELECT pep_client_id,application_id,provider_id,lifecycle,valid_until,created_at,updated_at FROM provider_pep_clients WHERE provider_id=$1 AND pep_client_id=$2",
            params!(provider_id.to_owned(), pep_client_id.to_owned()),
            "provider PEP client",
        ).await?;
        rows.into_iter()
            .next()
            .map(Into::into)
            .ok_or_else(|| anyhow!("provider PEP client not found"))
    }

    pub async fn provider_pep_client_for_application(
        &self,
        provider_id: &str,
        application_id: &str,
    ) -> Result<ProviderPepClient> {
        let pep = self
            .find_provider_pep_client_for_application(provider_id, application_id)
            .await?
            .ok_or_else(|| anyhow!("provider PEP client application binding not found"))?;
        require_active_pep(&pep)?;
        Ok(pep)
    }

    pub async fn find_provider_pep_client_for_application(
        &self,
        provider_id: &str,
        application_id: &str,
    ) -> Result<Option<ProviderPepClient>> {
        let rows: Vec<PepClientRow> = self.query_rows_with(
            ReadConsistency::Strong,
            "SELECT pc.pep_client_id,pc.application_id,pc.provider_id,pc.lifecycle,pc.valid_until,pc.created_at,pc.updated_at
             FROM provider_pep_clients pc
             JOIN applications a ON a.id=pc.application_id
             WHERE pc.provider_id=$1 AND pc.application_id=$2
               AND a.active=1 AND a.provisioning_status='ready'",
            params!(provider_id.to_owned(), application_id.to_owned()),
            "provider PEP client application binding",
        ).await?;
        Ok(rows.into_iter().next().map(Into::into))
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CanonicalGeneration {
    schema_version: u64,
    provider_id: String,
    generation: u64,
    capabilities: Vec<HttpCapability>,
}

fn canonical_generation_json(
    provider_id: &str,
    request: &StageHttpCapabilityGeneration,
) -> Result<String> {
    Ok(serde_json::to_string(&CanonicalGeneration {
        schema_version: request.schema_version,
        provider_id: provider_id.to_owned(),
        generation: request.generation,
        capabilities: request.capabilities.clone(),
    })?)
}

pub fn http_capability_catalog_digest(
    provider_id: &str,
    request: &StageHttpCapabilityGeneration,
) -> Result<String> {
    Ok(sha256_digest(
        canonical_generation_json(provider_id, request)?.as_bytes(),
    ))
}

pub fn http_capability_contract_digest(capability: &HttpCapability) -> Result<String> {
    let mut value = serde_json::to_value(capability)?;
    value
        .as_object_mut()
        .expect("capability serializes as object")
        .remove("contractDigest");
    Ok(sha256_digest(serde_json::to_string(&value)?.as_bytes()))
}

fn validate_generation(provider_id: &str, request: &StageHttpCapabilityGeneration) -> Result<()> {
    require_token("providerId", provider_id)?;
    if request.schema_version != CATALOG_SCHEMA_VERSION {
        anyhow::bail!("schemaVersion must be 1");
    }
    if request.generation == 0 {
        anyhow::bail!("generation must be greater than zero");
    }
    validate_digest("catalogDigest", &request.catalog_digest)?;
    let mut identities = HashSet::new();
    for capability in &request.capabilities {
        validate_capability(capability)?;
        if !identities.insert((capability.capability_id.clone(), capability.version)) {
            anyhow::bail!("duplicate HTTP capability identity");
        }
    }
    for (index, left) in request
        .capabilities
        .iter()
        .enumerate()
        .filter(|(_, value)| value.enabled)
    {
        for right in request
            .capabilities
            .iter()
            .skip(index + 1)
            .filter(|value| value.enabled)
        {
            if templates_overlap(left, right) {
                anyhow::bail!("ambiguous enabled HTTP capability templates");
            }
        }
    }
    Ok(())
}

fn validate_capability(capability: &HttpCapability) -> Result<()> {
    require_token("capabilityId", &capability.capability_id)?;
    if capability.version == 0 {
        anyhow::bail!("capability version must be greater than zero");
    }
    if capability.method.is_empty()
        || !capability
            .method
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b == b'-')
    {
        anyhow::bail!("method must be one exact uppercase token");
    }
    validate_path_template(&capability.path_template)?;
    validate_http_policies(capability)?;
    match capability.access_mode {
        HttpCapabilityAccessMode::PublicAuthn => {
            if capability.action_id.is_some() || capability.resource.is_some() {
                anyhow::bail!("public_authn capability must not carry action or resource");
            }
            if !path_template_variables(&capability.path_template).is_empty() {
                anyhow::bail!("public_authn path must not contain variables");
            }
        }
        HttpCapabilityAccessMode::Authorized => {
            require_token(
                "actionId",
                capability.action_id.as_deref().unwrap_or_default(),
            )?;
            let resource = capability
                .resource
                .as_ref()
                .ok_or_else(|| anyhow!("authorized capability requires resource"))?;
            require_token("resource.type", &resource.resource_type)?;
            validate_extractor(&resource.id, &capability.path_template)?;
            validate_extractor_policy_binding(capability, &resource.id)?;
        }
    }
    validate_digest("contractDigest", &capability.contract_digest)?;
    if capability.contract_digest != http_capability_contract_digest(capability)? {
        anyhow::bail!("contractDigest does not match canonical capability");
    }
    Ok(())
}

fn validate_extractor_policy_binding(
    capability: &HttpCapability,
    extractor: &HttpResourceExtractor,
) -> Result<()> {
    let variables = path_template_variables(&capability.path_template);
    match extractor {
        HttpResourceExtractor::PathSegment { name, .. } => {
            if variables != [name.as_str()] {
                anyhow::bail!("path template must contain exactly the modeled resource variable");
            }
        }
        HttpResourceExtractor::QueryScalar { name, format } => {
            if !variables.is_empty() {
                anyhow::bail!("non-path extractor capability must use a literal path");
            }
            let HttpQueryPolicy::Exact { parameters } = &capability.query else {
                anyhow::bail!("query extractor requires an exact query policy");
            };
            if !parameters.iter().any(|parameter| {
                parameter.name == *name && parameter.format == *format && parameter.required
            }) {
                anyhow::bail!("query extractor must bind one required exact query parameter");
            }
        }
        HttpResourceExtractor::JsonPointer { .. } => {
            if !variables.is_empty() {
                anyhow::bail!("non-path extractor capability must use a literal path");
            }
            if capability.request.body != HttpRequestBodyMode::Json {
                anyhow::bail!("JSON pointer extractor requires a JSON request policy");
            }
        }
        HttpResourceExtractor::Constant { .. } => {
            if !variables.is_empty() {
                anyhow::bail!("non-path extractor capability must use a literal path");
            }
        }
    }
    Ok(())
}

fn validate_http_policies(capability: &HttpCapability) -> Result<()> {
    let mut query_names = HashSet::new();
    match &capability.query {
        HttpQueryPolicy::None {} => {}
        HttpQueryPolicy::Exact { parameters } => {
            if parameters.is_empty() {
                anyhow::bail!("exact query policy requires at least one parameter");
            }
            for parameter in parameters {
                require_token("query parameter name", &parameter.name)?;
                validate_format_name(&parameter.format)?;
                if !query_names.insert(parameter.name.as_str()) {
                    anyhow::bail!("duplicate query policy parameter");
                }
            }
            if !parameters
                .windows(2)
                .all(|pair| pair[0].name < pair[1].name)
            {
                anyhow::bail!("query policy parameters must be sorted");
            }
        }
    }
    match capability.request.body {
        HttpRequestBodyMode::None if capability.request.max_bytes != 0 => {
            anyhow::bail!("bodyless request policy must have maxBytes zero");
        }
        HttpRequestBodyMode::Json if capability.request.max_bytes == 0 => {
            anyhow::bail!("JSON request policy requires a positive maxBytes");
        }
        _ => {}
    }
    validate_header_policy(&capability.request.forward_headers, true)?;
    if capability.response.max_bytes == 0 {
        anyhow::bail!("response policy requires a positive maxBytes");
    }
    validate_header_policy(&capability.response.forward_headers, false)?;
    Ok(())
}

fn validate_header_policy(headers: &[String], request: bool) -> Result<()> {
    const REQUEST: &[&str] = &["accept", "content-type", "if-match", "if-none-match"];
    const RESPONSE: &[&str] = &["cache-control", "content-type", "etag", "last-modified"];
    let allowed = if request { REQUEST } else { RESPONSE };
    let mut seen = HashSet::new();
    for header in headers {
        if !allowed.contains(&header.as_str()) || !seen.insert(header.as_str()) {
            anyhow::bail!("unsupported or duplicate HTTP header policy");
        }
    }
    if !headers.windows(2).all(|pair| pair[0] < pair[1]) {
        anyhow::bail!("HTTP header policy must be sorted");
    }
    Ok(())
}

fn validate_format_name(format: &str) -> Result<()> {
    if matches!(
        format,
        "pocketbaseId" | "uuid" | "xid" | "slug" | "asciiToken"
    ) {
        Ok(())
    } else {
        anyhow::bail!("unsupported scalar format")
    }
}

fn validate_extractor(extractor: &HttpResourceExtractor, path_template: &str) -> Result<()> {
    match extractor {
        HttpResourceExtractor::PathSegment { name, format } => {
            require_token("path extractor name", name)?;
            validate_format(format)?;
            if !path_template
                .split('/')
                .any(|segment| segment == format!("{{{name}}}"))
            {
                anyhow::bail!("path extractor does not reference a template variable");
            }
        }
        HttpResourceExtractor::QueryScalar { name, format } => {
            require_token("query extractor name", name)?;
            validate_format(format)?;
        }
        HttpResourceExtractor::JsonPointer { pointer, format } => {
            if !pointer.starts_with('/') || pointer.contains("//") {
                anyhow::bail!("jsonPointer must be one fixed RFC 6901 pointer");
            }
            validate_format(format)?;
        }
        HttpResourceExtractor::Constant { value } => {
            require_token("constant resource", value)?;
        }
    }
    Ok(())
}

fn validate_format(format: &str) -> Result<()> {
    if matches!(
        format,
        "pocketbaseId" | "uuid" | "xid" | "slug" | "asciiToken"
    ) {
        Ok(())
    } else {
        anyhow::bail!("unsupported scalar format: {format}")
    }
}

fn validate_path_template(path: &str) -> Result<()> {
    if !path.starts_with('/')
        || path.contains("//")
        || path.contains('\\')
        || path.bytes().any(|b| b.is_ascii_control())
    {
        anyhow::bail!("invalid pathTemplate");
    }
    for segment in path.split('/').skip(1) {
        if segment.is_empty() || segment == "." || segment == ".." {
            anyhow::bail!("invalid pathTemplate segment");
        }
        let braces = segment.starts_with('{') && segment.ends_with('}');
        if segment.contains('{') || segment.contains('}') {
            if !braces || segment[1..segment.len() - 1].contains(['{', '}']) {
                anyhow::bail!("path variable must occupy one whole segment");
            }
            require_token("path variable", &segment[1..segment.len() - 1])?;
        }
    }
    Ok(())
}

fn path_template_variables(path: &str) -> Vec<&str> {
    path.split('/')
        .filter_map(|segment| segment.strip_prefix('{')?.strip_suffix('}'))
        .collect()
}

fn templates_overlap(left: &HttpCapability, right: &HttpCapability) -> bool {
    if left.method != right.method {
        return false;
    }
    let left = left.path_template.split('/').collect::<Vec<_>>();
    let right = right.path_template.split('/').collect::<Vec<_>>();
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(a, b)| a == &b || a.starts_with('{') || b.starts_with('{'))
}

fn require_token(field: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > 256
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':' | b'/'))
    {
        anyhow::bail!("{field} must be a bounded canonical token");
    }
    Ok(())
}

fn validate_digest(field: &str, value: &str) -> Result<()> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        anyhow::bail!("{field} must be sha256 digest");
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        anyhow::bail!("{field} must be sha256 digest");
    }
    Ok(())
}

fn validate_expiry(value: Option<&str>) -> Result<()> {
    if let Some(value) = value {
        DateTime::parse_from_rfc3339(value).context("validUntil must be RFC3339")?;
    }
    Ok(())
}

fn require_active_pep(pep: &ProviderPepClient) -> Result<()> {
    if pep.lifecycle != "active" {
        anyhow::bail!("provider PEP client is not active");
    }
    if let Some(valid_until) = pep.valid_until.as_deref()
        && DateTime::parse_from_rfc3339(valid_until)? <= Utc::now()
    {
        anyhow::bail!("provider PEP client is expired");
    }
    Ok(())
}

fn staged_result(
    staged: bool,
    provider_id: &str,
    request: &StageHttpCapabilityGeneration,
) -> StagedHttpCapabilityGeneration {
    let enabled_count = request
        .capabilities
        .iter()
        .filter(|capability| capability.enabled)
        .count();
    StagedHttpCapabilityGeneration {
        staged,
        provider_id: provider_id.to_owned(),
        generation: request.generation,
        catalog_digest: request.catalog_digest.clone(),
        total_count: request.capabilities.len(),
        enabled_count,
        disabled_count: request.capabilities.len() - enabled_count,
    }
}

fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authorized(path: &str, capability_id: &str) -> HttpCapability {
        let mut capability = HttpCapability {
            capability_id: capability_id.to_owned(),
            version: 1,
            enabled: true,
            access_mode: HttpCapabilityAccessMode::Authorized,
            method: "GET".to_owned(),
            path_template: path.to_owned(),
            action_id: Some("patch-data.plant.read".to_owned()),
            resource: Some(HttpCapabilityResource {
                resource_type: "plant".to_owned(),
                id: HttpResourceExtractor::PathSegment {
                    name: "plantId".to_owned(),
                    format: "pocketbaseId".to_owned(),
                },
            }),
            query: HttpQueryPolicy::None {},
            request: HttpRequestPolicy {
                body: HttpRequestBodyMode::None,
                max_bytes: 0,
                forward_headers: vec![],
            },
            response: HttpResponsePolicy {
                max_bytes: 4_194_304,
                forward_headers: vec![],
                redirects: HttpRedirectPolicy::Deny,
            },
            contract_digest:
                "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_owned(),
        };
        capability.contract_digest = http_capability_contract_digest(&capability).unwrap();
        capability
    }

    #[test]
    fn rejects_ambiguous_enabled_templates() {
        let first = authorized("/api/collections/plants/records/{plantId}", "one");
        let mut second = authorized("/api/collections/plants/records/{plantId}", "two");
        second.contract_digest = http_capability_contract_digest(&second).unwrap();
        let mut request = StageHttpCapabilityGeneration {
            schema_version: 1,
            generation: 1,
            expected_latest_generation: None,
            capabilities: vec![first, second],
            catalog_digest:
                "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_owned(),
        };
        request.catalog_digest = http_capability_catalog_digest("patch-data", &request).unwrap();
        assert!(validate_generation("patch-data", &request).is_err());
    }

    #[test]
    fn disabled_overlap_is_retained_but_not_ambiguous() {
        let first = authorized("/api/collections/plants/records/{plantId}", "one");
        let mut second = authorized("/api/collections/plants/records/{plantId}", "two");
        second.enabled = false;
        second.contract_digest = http_capability_contract_digest(&second).unwrap();
        let mut request = StageHttpCapabilityGeneration {
            schema_version: 1,
            generation: 1,
            expected_latest_generation: None,
            capabilities: vec![first, second],
            catalog_digest:
                "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_owned(),
        };
        request.catalog_digest = http_capability_catalog_digest("patch-data", &request).unwrap();
        validate_generation("patch-data", &request).unwrap();
    }

    #[test]
    fn public_authn_cannot_carry_policy_resource() {
        let mut capability = authorized("/api/collections/users/auth-refresh", "auth-refresh");
        capability.access_mode = HttpCapabilityAccessMode::PublicAuthn;
        capability.contract_digest = http_capability_contract_digest(&capability).unwrap();
        assert!(validate_capability(&capability).is_err());
    }

    #[test]
    fn catalog_digest_binds_provider_and_generation() {
        let capability = authorized("/api/collections/plants/records/{plantId}", "one");
        let request = StageHttpCapabilityGeneration {
            schema_version: 1,
            generation: 1,
            expected_latest_generation: None,
            capabilities: vec![capability],
            catalog_digest:
                "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_owned(),
        };
        assert_ne!(
            http_capability_catalog_digest("patch-data", &request).unwrap(),
            http_capability_catalog_digest("other", &request).unwrap()
        );
    }

    #[test]
    fn rejects_unmodeled_or_reserved_path_variables() {
        for path in [
            "/api/collections/{collection}/records/{plantId}",
            "/api/collections/plants/records/{plantId}/{other}",
        ] {
            let capability = authorized(path, "plant.get");
            assert!(validate_capability(&capability).is_err(), "accepted {path}");
        }
    }

    #[test]
    fn capability_and_nested_policies_reject_unknown_fields() {
        let capability = authorized("/api/collections/plants/records/{plantId}", "plant.get");
        let mut value = serde_json::to_value(capability).unwrap();
        value["unknown"] = serde_json::json!(true);
        assert!(serde_json::from_value::<HttpCapability>(value).is_err());

        let mut value = serde_json::to_value(authorized(
            "/api/collections/plants/records/{plantId}",
            "plant.get",
        ))
        .unwrap();
        value["query"]["unknown"] = serde_json::json!(true);
        assert!(serde_json::from_value::<HttpCapability>(value).is_err());

        for policy in ["request", "response"] {
            let mut value = serde_json::to_value(authorized(
                "/api/collections/plants/records/{plantId}",
                "plant.get",
            ))
            .unwrap();
            value[policy]["unknown"] = serde_json::json!(true);
            assert!(
                serde_json::from_value::<HttpCapability>(value).is_err(),
                "unknown {policy} policy field was accepted"
            );
        }
    }
}
