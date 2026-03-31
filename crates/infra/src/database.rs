use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use domain::{Business, DocumentArtifact, DocumentCategory, GenerationRecord, Plant, RateRecord, RecRecord, SyncMode, SyncRun, User};
use hiqlite::Client;
use serde::Deserialize;
use uuid::Uuid;

macro_rules! params {
    ($($value:expr),* $(,)?) => {
        vec![$(::hiqlite::Param::from($value)),*]
    };
}

#[derive(Clone)]
pub struct Database {
    client: Client,
    bootstrap_user_id: String,
    bootstrap_user_name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StoredCertificate {
    pub id: String,
    pub business_id: String,
    pub common_name: String,
    pub encrypted_blob: String,
    pub active: i64,
    pub last_used_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct StoredDocument {
    pub artifact: DocumentArtifact,
    pub object_key: String,
    pub content_type: String,
}

#[derive(Debug, Clone)]
pub struct BackupEntry {
    pub name: String,
    pub last_modified: i64,
    pub size: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct PlantSummary {
    pub plant: Plant,
    pub last_sync: Option<SyncRun>,
    pub generation_count: i64,
    pub generation_total_kwh: f64,
    pub rates_count: i64,
    pub rec_count: i64,
    pub rec_total: i64,
    pub document_count: i64,
    pub document_categories: Vec<(DocumentCategory, i64)>,
}

#[derive(Debug, Clone, Deserialize)]
struct CountRow {
    count: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct CountSumRow {
    count: i64,
    total: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
struct CountSumIntRow {
    count: i64,
    total: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
struct PlantRow {
    id: String,
    name: String,
    business_no: String,
    contract_no: String,
    address: String,
    installed_kw: f64,
    rec_start_date: String,
    confirm_issued: String,
}

#[derive(Debug, Clone, Deserialize)]
struct SyncRunRow {
    id: String,
    business_no: String,
    plant_id: Option<String>,
    mode: String,
    status: String,
    started_at: String,
    finished_at: Option<String>,
    duration_ms: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct GenerationRow {
    month: String,
    generation_kwh: f64,
    self_consumption_kwh: Option<f64>,
    supply_kwh: f64,
    application_deadline: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RateRow {
    month: String,
    self_rate: Option<f64>,
    national_average: Option<f64>,
    nearby_average: Option<f64>,
    nearby_highest: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
struct RecRow {
    month: String,
    application_no: String,
    supply_kwh: i64,
    rec_quantity: i64,
    fee_krw: i64,
    issued_date: String,
    expiry_date: String,
    virtual_account: Option<String>,
    bank_name: Option<String>,
    status: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct DocumentRow {
    id: String,
    category: String,
    file_class: String,
    file_sec_name: String,
    file_name: String,
    required: i64,
    source_sha256: Option<String>,
    output_sha256: Option<String>,
    object_key: String,
    content_type: String,
}

#[derive(Debug, Clone, Deserialize)]
struct CategoryCountRow {
    category: String,
    count: i64,
}

impl Database {
    pub fn new(client: Client, bootstrap_user_id: String, bootstrap_user_name: String) -> Self {
        Self {
            client,
            bootstrap_user_id,
            bootstrap_user_name,
        }
    }

    pub async fn ensure_schema(&self) -> Result<()> {
        self.client
            .batch(
                r#"
                CREATE TABLE IF NOT EXISTS users (
                    id TEXT PRIMARY KEY,
                    email TEXT,
                    display_name TEXT NOT NULL,
                    active INTEGER NOT NULL DEFAULT 1,
                    created_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS legal_entities (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    representative_name TEXT
                );
                CREATE TABLE IF NOT EXISTS businesses (
                    id TEXT PRIMARY KEY,
                    display_name TEXT NOT NULL,
                    business_no TEXT NOT NULL UNIQUE,
                    common_name TEXT,
                    legal_entity_id TEXT,
                    active INTEGER NOT NULL DEFAULT 1,
                    ente_id TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS business_memberships (
                    user_id TEXT NOT NULL,
                    business_id TEXT NOT NULL,
                    role TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    PRIMARY KEY (user_id, business_id)
                );
                CREATE TABLE IF NOT EXISTS certificates (
                    id TEXT PRIMARY KEY,
                    business_id TEXT NOT NULL,
                    common_name TEXT NOT NULL,
                    encrypted_blob TEXT NOT NULL,
                    active INTEGER NOT NULL DEFAULT 1,
                    last_used_at TEXT,
                    created_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_certificates_business_active
                    ON certificates (business_id, active, created_at DESC);
                CREATE TABLE IF NOT EXISTS plants (
                    id TEXT PRIMARY KEY,
                    business_id TEXT NOT NULL,
                    name TEXT NOT NULL,
                    business_no TEXT NOT NULL,
                    contract_no TEXT NOT NULL,
                    address TEXT NOT NULL,
                    installed_kw REAL NOT NULL,
                    rec_start_date TEXT NOT NULL,
                    confirm_issued TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS sync_runs (
                    id TEXT PRIMARY KEY,
                    business_id TEXT NOT NULL,
                    plant_id TEXT,
                    mode TEXT NOT NULL,
                    status TEXT NOT NULL,
                    started_at TEXT NOT NULL,
                    finished_at TEXT,
                    duration_ms INTEGER NOT NULL DEFAULT 0,
                    error_message TEXT
                );
                CREATE INDEX IF NOT EXISTS idx_sync_runs_plant_started
                    ON sync_runs (plant_id, started_at DESC);
                CREATE TABLE IF NOT EXISTS generation_records (
                    plant_id TEXT NOT NULL,
                    month TEXT NOT NULL,
                    generation_kwh REAL NOT NULL,
                    self_consumption_kwh REAL,
                    supply_kwh REAL NOT NULL,
                    application_deadline TEXT,
                    updated_at TEXT NOT NULL,
                    PRIMARY KEY (plant_id, month)
                );
                CREATE TABLE IF NOT EXISTS rate_records (
                    plant_id TEXT NOT NULL,
                    month TEXT NOT NULL,
                    self_rate REAL,
                    national_average REAL,
                    nearby_average REAL,
                    nearby_highest REAL,
                    published INTEGER NOT NULL DEFAULT 0,
                    source_control_flag TEXT,
                    updated_at TEXT NOT NULL,
                    PRIMARY KEY (plant_id, month)
                );
                CREATE TABLE IF NOT EXISTS rec_records (
                    plant_id TEXT NOT NULL,
                    application_no TEXT NOT NULL,
                    month TEXT NOT NULL,
                    supply_kwh INTEGER NOT NULL,
                    rec_quantity INTEGER NOT NULL,
                    fee_krw INTEGER NOT NULL,
                    issued_date TEXT NOT NULL,
                    expiry_date TEXT NOT NULL,
                    virtual_account TEXT,
                    bank_name TEXT,
                    status TEXT,
                    updated_at TEXT NOT NULL,
                    PRIMARY KEY (plant_id, application_no)
                );
                CREATE TABLE IF NOT EXISTS documents (
                    id TEXT PRIMARY KEY,
                    plant_id TEXT NOT NULL,
                    category TEXT NOT NULL,
                    attachment_code TEXT NOT NULL,
                    file_class TEXT NOT NULL,
                    file_sec_name TEXT NOT NULL,
                    file_name TEXT NOT NULL,
                    required INTEGER NOT NULL DEFAULT 1,
                    source_sha256 TEXT,
                    output_sha256 TEXT,
                    object_key TEXT NOT NULL,
                    content_type TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE UNIQUE INDEX IF NOT EXISTS idx_documents_unique
                    ON documents (plant_id, attachment_code, file_name);
                "#
                .to_owned(),
            )
            .await
            .context("create schema")?;
        self.ensure_bootstrap_user().await.map(|_| ())
    }

    pub async fn ensure_bootstrap_user(&self) -> Result<User> {
        let now = now_string();
        self.client
            .execute(
                "INSERT INTO users (id, email, display_name, active, created_at) VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT(id) DO UPDATE SET display_name = excluded.display_name, active = excluded.active",
                params!(
                    self.bootstrap_user_id.clone(),
                    Option::<String>::None,
                    self.bootstrap_user_name.clone(),
                    1_i64,
                    now.clone()
                ),
            )
            .await
            .context("upsert bootstrap user")?;
        Ok(User {
            id: self.bootstrap_user_id.clone(),
            email: None,
            display_name: self.bootstrap_user_name.clone(),
            active: true,
        })
    }

    pub async fn upsert_business_with_owner(
        &self,
        business_no: &str,
        display_name: &str,
        common_name: &str,
    ) -> Result<Business> {
        let now = now_string();
        self.client
            .txn([
                (
                    "INSERT INTO businesses (id, display_name, business_no, common_name, active, created_at, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7)
                     ON CONFLICT(id) DO UPDATE SET
                        display_name = excluded.display_name,
                        business_no = excluded.business_no,
                        common_name = excluded.common_name,
                        active = excluded.active,
                        updated_at = excluded.updated_at",
                    params!(
                        business_no.to_owned(),
                        display_name.to_owned(),
                        business_no.to_owned(),
                        common_name.to_owned(),
                        1_i64,
                        now.clone(),
                        now.clone()
                    ),
                ),
                (
                    "INSERT INTO business_memberships (user_id, business_id, role, created_at)
                     VALUES ($1, $2, $3, $4)
                     ON CONFLICT(user_id, business_id) DO UPDATE SET role = excluded.role",
                    params!(
                        self.bootstrap_user_id.clone(),
                        business_no.to_owned(),
                        "owner",
                        now.clone()
                    ),
                ),
            ])
            .await
            .context("upsert business and membership transaction")?;
        Ok(Business {
            id: business_no.to_owned(),
            display_name: display_name.to_owned(),
            business_no: business_no.to_owned(),
            legal_entity_id: None,
            active: true,
        })
    }

    pub async fn save_certificate(
        &self,
        business_no: &str,
        common_name: &str,
        encrypted_blob: &str,
    ) -> Result<StoredCertificate> {
        let id = Uuid::new_v4().simple().to_string();
        let now = now_string();
        self.client
            .txn([
                (
                    "INSERT INTO businesses (id, display_name, business_no, common_name, active, created_at, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7)
                     ON CONFLICT(id) DO UPDATE SET
                        display_name = excluded.display_name,
                        business_no = excluded.business_no,
                        common_name = excluded.common_name,
                        active = excluded.active,
                        updated_at = excluded.updated_at",
                    params!(
                        business_no.to_owned(),
                        common_name.to_owned(),
                        business_no.to_owned(),
                        common_name.to_owned(),
                        1_i64,
                        now.clone(),
                        now.clone()
                    ),
                ),
                (
                    "INSERT INTO business_memberships (user_id, business_id, role, created_at)
                     VALUES ($1, $2, $3, $4)
                     ON CONFLICT(user_id, business_id) DO UPDATE SET role = excluded.role",
                    params!(
                        self.bootstrap_user_id.clone(),
                        business_no.to_owned(),
                        "owner",
                        now.clone()
                    ),
                ),
                (
                    "UPDATE certificates SET active = 0 WHERE business_id = $1",
                    params!(business_no.to_owned()),
                ),
                (
                    "INSERT INTO certificates (id, business_id, common_name, encrypted_blob, active, last_used_at, created_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7)",
                    params!(
                        id.clone(),
                        business_no.to_owned(),
                        common_name.to_owned(),
                        encrypted_blob.to_owned(),
                        1_i64,
                        Option::<String>::None,
                        now
                    ),
                ),
            ])
            .await
            .context("save certificate transaction")?;
        self.load_active_certificate(business_no).await
    }

    pub async fn load_active_certificate(&self, business_no: &str) -> Result<StoredCertificate> {
        let mut rows: Vec<StoredCertificate> = self
            .client
            .query_as(
                "SELECT id, business_id, common_name, encrypted_blob, active, last_used_at, created_at
                 FROM certificates
                 WHERE business_id = $1
                 ORDER BY active DESC, created_at DESC
                 LIMIT 1",
                params!(business_no.to_owned()),
            )
            .await
            .context("query active certificate")?;
        let cert = rows.pop().ok_or_else(|| anyhow!("certificate not found"))?;
        self.client
            .execute(
                "UPDATE certificates SET last_used_at = $1 WHERE id = $2",
                params!(now_string(), cert.id.clone()),
            )
            .await
            .context("update certificate last_used_at")?;
        Ok(cert)
    }

    pub async fn start_sync_run(
        &self,
        business_no: &str,
        plant_id: Option<&str>,
        mode: SyncMode,
    ) -> Result<SyncRun> {
        let id = Uuid::new_v4().simple().to_string();
        let started_at = now_string();
        self.client
            .execute(
                "INSERT INTO sync_runs (id, business_id, plant_id, mode, status, started_at, finished_at, duration_ms, error_message)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                params!(
                    id.clone(),
                    business_no.to_owned(),
                    plant_id.map(ToOwned::to_owned),
                    sync_mode_str(&mode),
                    "running",
                    started_at.clone(),
                    Option::<String>::None,
                    0_i64,
                    Option::<String>::None
                ),
            )
            .await
            .context("insert sync run")?;
        Ok(SyncRun {
            id,
            business_no: business_no.to_owned(),
            plant_id: plant_id.map(ToOwned::to_owned),
            mode,
            status: "running".to_owned(),
            started_at: Some(started_at),
            finished_at: None,
            duration_ms: 0,
        })
    }

    pub async fn finish_sync_run(
        &self,
        run_id: &str,
        status: &str,
        duration_ms: i64,
        error_message: Option<&str>,
        plant_id: Option<&str>,
    ) -> Result<()> {
        self.client
            .execute(
                "UPDATE sync_runs
                 SET status = $1, finished_at = $2, duration_ms = $3, error_message = $4, plant_id = COALESCE($5, plant_id)
                 WHERE id = $6",
                params!(
                    status.to_owned(),
                    now_string(),
                    duration_ms,
                    error_message.map(ToOwned::to_owned),
                    plant_id.map(ToOwned::to_owned),
                    run_id.to_owned()
                ),
            )
            .await
            .context("finish sync run")?;
        Ok(())
    }

    pub async fn upsert_plant(&self, plant: &Plant) -> Result<()> {
        let now = now_string();
        self.client
            .execute(
                "INSERT INTO plants (id, business_id, name, business_no, contract_no, address, installed_kw, rec_start_date, confirm_issued, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                 ON CONFLICT(id) DO UPDATE SET
                    business_id = excluded.business_id,
                    name = excluded.name,
                    business_no = excluded.business_no,
                    contract_no = excluded.contract_no,
                    address = excluded.address,
                    installed_kw = excluded.installed_kw,
                    rec_start_date = excluded.rec_start_date,
                    confirm_issued = excluded.confirm_issued,
                    updated_at = excluded.updated_at",
                params!(
                    plant.code.clone(),
                    plant.business_no.clone(),
                    plant.name.clone(),
                    plant.business_no.clone(),
                    plant.contract_no.clone(),
                    plant.address.clone(),
                    plant.installed_kw,
                    plant.rec_start_date.clone(),
                    plant.confirm_issued.clone(),
                    now.clone(),
                    now
                ),
            )
            .await
            .context("upsert plant")?;
        Ok(())
    }

    pub async fn replace_full_records(
        &self,
        plant_id: &str,
        generation: &[GenerationRecord],
        rates: &[StoredRateRecord],
        rec: &[RecRecord],
    ) -> Result<()> {
        let updated_at = now_string();
        let mut queries = vec![
            (
                "DELETE FROM generation_records WHERE plant_id = $1".to_owned(),
                params!(plant_id.to_owned()),
            ),
            (
                "DELETE FROM rate_records WHERE plant_id = $1".to_owned(),
                params!(plant_id.to_owned()),
            ),
            (
                "DELETE FROM rec_records WHERE plant_id = $1".to_owned(),
                params!(plant_id.to_owned()),
            ),
        ];
        for record in generation {
            queries.push((
                "INSERT INTO generation_records (plant_id, month, generation_kwh, self_consumption_kwh, supply_kwh, application_deadline, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT(plant_id, month) DO UPDATE SET
                    generation_kwh = excluded.generation_kwh,
                    self_consumption_kwh = excluded.self_consumption_kwh,
                    supply_kwh = excluded.supply_kwh,
                    application_deadline = excluded.application_deadline,
                    updated_at = excluded.updated_at".to_owned(),
                params!(
                    plant_id.to_owned(),
                    record.month.clone(),
                    record.generation_kwh,
                    record.self_consumption_kwh,
                    record.supply_kwh,
                    record.application_deadline.clone(),
                    updated_at.clone()
                ),
            ));
        }
        for record in rates {
            queries.push((
                "INSERT INTO rate_records (plant_id, month, self_rate, national_average, nearby_average, nearby_highest, published, source_control_flag, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                 ON CONFLICT(plant_id, month) DO UPDATE SET
                    self_rate = excluded.self_rate,
                    national_average = excluded.national_average,
                    nearby_average = excluded.nearby_average,
                    nearby_highest = excluded.nearby_highest,
                    published = excluded.published,
                    source_control_flag = excluded.source_control_flag,
                    updated_at = excluded.updated_at".to_owned(),
                params!(
                    plant_id.to_owned(),
                    record.record.month.clone(),
                    record.record.self_rate,
                    record.record.national_average,
                    record.record.nearby_average,
                    record.record.nearby_highest,
                    if record.published { 1_i64 } else { 0_i64 },
                    record.source_control_flag.clone(),
                    updated_at.clone()
                ),
            ));
        }
        for record in rec {
            queries.push((
                "INSERT INTO rec_records (plant_id, application_no, month, supply_kwh, rec_quantity, fee_krw, issued_date, expiry_date, virtual_account, bank_name, status, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
                 ON CONFLICT(plant_id, application_no) DO UPDATE SET
                    month = excluded.month,
                    supply_kwh = excluded.supply_kwh,
                    rec_quantity = excluded.rec_quantity,
                    fee_krw = excluded.fee_krw,
                    issued_date = excluded.issued_date,
                    expiry_date = excluded.expiry_date,
                    virtual_account = excluded.virtual_account,
                    bank_name = excluded.bank_name,
                    status = excluded.status,
                    updated_at = excluded.updated_at".to_owned(),
                params!(
                    plant_id.to_owned(),
                    record.application_no.clone(),
                    record.month.clone(),
                    record.supply_kwh,
                    record.rec_quantity,
                    record.fee_krw,
                    record.issued_date.clone(),
                    record.expiry_date.clone(),
                    record.virtual_account.clone(),
                    record.bank_name.clone(),
                    record.status.clone(),
                    updated_at.clone()
                ),
            ));
        }
        self.client
            .txn(queries)
            .await
            .context("replace full records transaction")?;
        Ok(())
    }

    pub async fn upsert_generation_records(
        &self,
        plant_id: &str,
        records: &[GenerationRecord],
    ) -> Result<()> {
        let updated_at = now_string();
        for record in records {
            self.client
                .execute(
                    "INSERT INTO generation_records (plant_id, month, generation_kwh, self_consumption_kwh, supply_kwh, application_deadline, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7)
                     ON CONFLICT(plant_id, month) DO UPDATE SET
                        generation_kwh = excluded.generation_kwh,
                        self_consumption_kwh = excluded.self_consumption_kwh,
                        supply_kwh = excluded.supply_kwh,
                        application_deadline = excluded.application_deadline,
                        updated_at = excluded.updated_at",
                    params!(
                        plant_id.to_owned(),
                        record.month.clone(),
                        record.generation_kwh,
                        record.self_consumption_kwh,
                        record.supply_kwh,
                        record.application_deadline.clone(),
                        updated_at.clone()
                    ),
                )
                .await
                .with_context(|| format!("upsert generation {}", record.month))?;
        }
        Ok(())
    }

    pub async fn upsert_rate_records(
        &self,
        plant_id: &str,
        records: &[StoredRateRecord],
    ) -> Result<()> {
        let updated_at = now_string();
        for record in records {
            self.client
                .execute(
                    "INSERT INTO rate_records (plant_id, month, self_rate, national_average, nearby_average, nearby_highest, published, source_control_flag, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                     ON CONFLICT(plant_id, month) DO UPDATE SET
                        self_rate = excluded.self_rate,
                        national_average = excluded.national_average,
                        nearby_average = excluded.nearby_average,
                        nearby_highest = excluded.nearby_highest,
                        published = excluded.published,
                        source_control_flag = excluded.source_control_flag,
                        updated_at = excluded.updated_at",
                    params!(
                        plant_id.to_owned(),
                        record.record.month.clone(),
                        record.record.self_rate,
                        record.record.national_average,
                        record.record.nearby_average,
                        record.record.nearby_highest,
                        if record.published { 1_i64 } else { 0_i64 },
                        record.source_control_flag.clone(),
                        updated_at.clone()
                    ),
                )
                .await
                .with_context(|| format!("upsert rate {}", record.record.month))?;
        }
        Ok(())
    }

    pub async fn upsert_rec_records(&self, plant_id: &str, records: &[RecRecord]) -> Result<()> {
        let updated_at = now_string();
        for record in records {
            self.client
                .execute(
                    "INSERT INTO rec_records (plant_id, application_no, month, supply_kwh, rec_quantity, fee_krw, issued_date, expiry_date, virtual_account, bank_name, status, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
                     ON CONFLICT(plant_id, application_no) DO UPDATE SET
                        month = excluded.month,
                        supply_kwh = excluded.supply_kwh,
                        rec_quantity = excluded.rec_quantity,
                        fee_krw = excluded.fee_krw,
                        issued_date = excluded.issued_date,
                        expiry_date = excluded.expiry_date,
                        virtual_account = excluded.virtual_account,
                        bank_name = excluded.bank_name,
                        status = excluded.status,
                        updated_at = excluded.updated_at",
                    params!(
                        plant_id.to_owned(),
                        record.application_no.clone(),
                        record.month.clone(),
                        record.supply_kwh,
                        record.rec_quantity,
                        record.fee_krw,
                        record.issued_date.clone(),
                        record.expiry_date.clone(),
                        record.virtual_account.clone(),
                        record.bank_name.clone(),
                        record.status.clone(),
                        updated_at.clone()
                    ),
                )
                .await
                .with_context(|| format!("upsert rec {}", record.application_no))?;
        }
        Ok(())
    }

    pub async fn list_document_objects(&self, plant_id: &str) -> Result<Vec<String>> {
        let rows: Vec<DocumentRow> = self
            .client
            .query_as(
                "SELECT id, category, file_class, file_sec_name, file_name, required, source_sha256, output_sha256, object_key, content_type
                 FROM documents WHERE plant_id = $1",
                params!(plant_id.to_owned()),
            )
            .await
            .context("list document rows")?;
        Ok(rows.into_iter().map(|row| row.object_key).collect())
    }

    pub async fn replace_documents(
        &self,
        plant_id: &str,
        docs: &[PersistedDocument],
    ) -> Result<()> {
        let updated_at = now_string();
        let mut queries = vec![(
            "DELETE FROM documents WHERE plant_id = $1".to_owned(),
            params!(plant_id.to_owned()),
        )];
        for doc in docs {
            queries.push((
                "INSERT INTO documents (id, plant_id, category, attachment_code, file_class, file_sec_name, file_name, required, source_sha256, output_sha256, object_key, content_type, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)".to_owned(),
                params!(
                    doc.id.clone(),
                    plant_id.to_owned(),
                    doc.category.to_storage(),
                    doc.attachment_code.clone(),
                    doc.file_class.clone(),
                    doc.file_sec_name.clone(),
                    doc.file_name.clone(),
                    if doc.required { 1_i64 } else { 0_i64 },
                    doc.source_sha256.clone(),
                    doc.output_sha256.clone(),
                    doc.object_key.clone(),
                    doc.content_type.clone(),
                    updated_at.clone()
                ),
            ));
        }
        self.client
            .txn(queries)
            .await
            .context("replace documents transaction")?;
        Ok(())
    }

    pub async fn list_plants(&self, business_no: &str) -> Result<Vec<Plant>> {
        let rows: Vec<PlantRow> = self
            .client
            .query_as(
                "SELECT id, name, business_no, contract_no, address, installed_kw, rec_start_date, confirm_issued
                 FROM plants WHERE business_id = $1 ORDER BY id ASC",
                params!(business_no.to_owned()),
            )
            .await
            .context("list plants")?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn latest_generation_month(&self, plant_id: &str) -> Result<Option<String>> {
        let rows: Vec<GenerationRow> = self
            .client
            .query_as(
                "SELECT month, generation_kwh, self_consumption_kwh, supply_kwh, application_deadline
                 FROM generation_records WHERE plant_id = $1 ORDER BY month DESC LIMIT 1",
                params!(plant_id.to_owned()),
            )
            .await
            .context("query latest generation month")?;
        Ok(rows.into_iter().next().map(|row| row.month))
    }

    pub async fn get_plant_summary(&self, plant_id: &str) -> Result<PlantSummary> {
        let mut plants: Vec<PlantRow> = self
            .client
            .query_as(
                "SELECT id, name, business_no, contract_no, address, installed_kw, rec_start_date, confirm_issued
                 FROM plants WHERE id = $1 LIMIT 1",
                params!(plant_id.to_owned()),
            )
            .await
            .context("query plant")?;
        let plant_row = plants.pop().ok_or_else(|| anyhow!("plant not found"))?;

        let mut runs: Vec<SyncRunRow> = self
            .client
            .query_as(
                "SELECT id, business_id as business_no, plant_id, mode, status, started_at, finished_at, duration_ms
                 FROM sync_runs WHERE plant_id = $1 AND status = 'completed'
                 ORDER BY started_at DESC LIMIT 1",
                params!(plant_id.to_owned()),
            )
            .await
            .context("query latest sync run")?;
        let last_sync = runs.pop().map(Into::into);

        let generation = self
            .client
            .query_as_one::<CountSumRow, _>(
                "SELECT COUNT(*) as count, COALESCE(SUM(generation_kwh), 0) as total
                 FROM generation_records WHERE plant_id = $1",
                params!(plant_id.to_owned()),
            )
            .await
            .context("aggregate generation")?;
        let rates = self
            .client
            .query_as_one::<CountRow, _>(
                "SELECT COUNT(*) as count FROM rate_records WHERE plant_id = $1 AND published = 1",
                params!(plant_id.to_owned()),
            )
            .await
            .context("aggregate rates")?;
        let rec = self
            .client
            .query_as_one::<CountSumIntRow, _>(
                "SELECT COUNT(*) as count, COALESCE(SUM(rec_quantity), 0) as total
                 FROM rec_records WHERE plant_id = $1",
                params!(plant_id.to_owned()),
            )
            .await
            .context("aggregate rec")?;
        let documents = self
            .client
            .query_as_one::<CountRow, _>(
                "SELECT COUNT(*) as count FROM documents WHERE plant_id = $1",
                params!(plant_id.to_owned()),
            )
            .await
            .context("aggregate documents")?;
        let category_rows: Vec<CategoryCountRow> = self
            .client
            .query_as(
                "SELECT category, COUNT(*) as count FROM documents WHERE plant_id = $1 GROUP BY category ORDER BY category ASC",
                params!(plant_id.to_owned()),
            )
            .await
            .context("aggregate document categories")?;

        Ok(PlantSummary {
            plant: plant_row.into(),
            last_sync,
            generation_count: generation.count,
            generation_total_kwh: generation.total.unwrap_or_default(),
            rates_count: rates.count,
            rec_count: rec.count,
            rec_total: rec.total.unwrap_or_default(),
            document_count: documents.count,
            document_categories: category_rows
                .into_iter()
                .filter_map(|row| {
                    <DocumentCategory as DocumentCategoryExt>::from_storage(&row.category)
                        .map(|cat| (cat, row.count))
                })
                .collect(),
        })
    }

    pub async fn list_generation(&self, plant_id: &str) -> Result<Vec<GenerationRecord>> {
        let rows: Vec<GenerationRow> = self
            .client
            .query_as(
                "SELECT month, generation_kwh, self_consumption_kwh, supply_kwh, application_deadline
                 FROM generation_records WHERE plant_id = $1 ORDER BY month ASC",
                params!(plant_id.to_owned()),
            )
            .await
            .context("list generation")?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn list_rates(&self, plant_id: &str) -> Result<Vec<RateRecord>> {
        let rows: Vec<RateRow> = self
            .client
            .query_as(
                "SELECT month, self_rate, national_average, nearby_average, nearby_highest
                 FROM rate_records WHERE plant_id = $1 AND published = 1 ORDER BY month ASC",
                params!(plant_id.to_owned()),
            )
            .await
            .context("list rates")?;
        Ok(rows
            .into_iter()
            .map(|row| RateRecord {
                month: row.month,
                self_rate: row.self_rate,
                national_average: row.national_average,
                nearby_average: row.nearby_average,
                nearby_highest: row.nearby_highest,
            })
            .collect())
    }

    pub async fn list_rec(&self, plant_id: &str) -> Result<Vec<RecRecord>> {
        let rows: Vec<RecRow> = self
            .client
            .query_as(
                "SELECT month, application_no, supply_kwh, rec_quantity, fee_krw, issued_date, expiry_date, virtual_account, bank_name, status
                 FROM rec_records WHERE plant_id = $1 ORDER BY month ASC, application_no ASC",
                params!(plant_id.to_owned()),
            )
            .await
            .context("list rec")?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn list_documents(&self, plant_id: &str) -> Result<Vec<StoredDocument>> {
        let rows: Vec<DocumentRow> = self
            .client
            .query_as(
                "SELECT id, category, file_class, file_sec_name, file_name, required, source_sha256, output_sha256, object_key, content_type
                 FROM documents WHERE plant_id = $1 ORDER BY category ASC, file_name ASC",
                params!(plant_id.to_owned()),
            )
            .await
            .context("list documents")?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                <DocumentCategory as DocumentCategoryExt>::from_storage(&row.category).map(|category| StoredDocument {
                    artifact: DocumentArtifact {
                        id: row.id,
                        category,
                        file_class: row.file_class,
                        file_sec_name: row.file_sec_name,
                        file_name: row.file_name,
                        required: row.required == 1,
                        source_sha256: row.source_sha256,
                        output_sha256: row.output_sha256,
                        download_url: None,
                    },
                    object_key: row.object_key,
                    content_type: row.content_type,
                })
            })
            .collect())
    }

    pub async fn get_document(&self, id: &str) -> Result<StoredDocument> {
        let mut rows: Vec<DocumentRow> = self
            .client
            .query_as(
                "SELECT id, category, file_class, file_sec_name, file_name, required, source_sha256, output_sha256, object_key, content_type
                 FROM documents WHERE id = $1 LIMIT 1",
                params!(id.to_owned()),
            )
            .await
            .context("get document")?;
        let row = rows.pop().ok_or_else(|| anyhow!("document not found"))?;
        let category = <DocumentCategory as DocumentCategoryExt>::from_storage(&row.category)
            .ok_or_else(|| anyhow!("invalid document category {}", row.category))?;
        Ok(StoredDocument {
            artifact: DocumentArtifact {
                id: row.id,
                category,
                file_class: row.file_class,
                file_sec_name: row.file_sec_name,
                file_name: row.file_name,
                required: row.required == 1,
                source_sha256: row.source_sha256,
                output_sha256: row.output_sha256,
                download_url: None,
            },
            object_key: row.object_key,
            content_type: row.content_type,
        })
    }

    pub async fn list_backup_local(&self) -> Result<Vec<BackupEntry>> {
        let items = self
            .client
            .backup_list_local()
            .await
            .context("list local backups")?;
        Ok(items
            .into_iter()
            .map(|item| BackupEntry {
                name: item.name,
                last_modified: item.last_modified,
                size: item.size,
            })
            .collect())
    }

    pub async fn list_backup_s3(&self) -> Result<Vec<BackupEntry>> {
        let items = self
            .client
            .backup_list_s3()
            .await
            .context("list s3 backups")?;
        Ok(items
            .into_iter()
            .map(|item| BackupEntry {
                name: item.name,
                last_modified: item.last_modified,
                size: item.size,
            })
            .collect())
    }

    pub async fn trigger_backup(&self) -> Result<()> {
        self.client.backup().await.context("trigger hiqlite backup")
    }
}

#[derive(Debug, Clone)]
pub struct StoredRateRecord {
    pub record: RateRecord,
    pub published: bool,
    pub source_control_flag: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PersistedDocument {
    pub id: String,
    pub category: DocumentCategory,
    pub attachment_code: String,
    pub file_class: String,
    pub file_sec_name: String,
    pub file_name: String,
    pub required: bool,
    pub source_sha256: Option<String>,
    pub output_sha256: Option<String>,
    pub object_key: String,
    pub content_type: String,
}

fn sync_mode_str(mode: &SyncMode) -> &'static str {
    match mode {
        SyncMode::Delta => "delta",
        SyncMode::Full => "full",
    }
}

fn now_string() -> String {
    DateTime::<Utc>::from(std::time::SystemTime::now()).to_rfc3339()
}

impl From<PlantRow> for Plant {
    fn from(value: PlantRow) -> Self {
        Self {
            code: value.id,
            name: value.name,
            business_no: value.business_no,
            contract_no: value.contract_no,
            address: value.address,
            installed_kw: value.installed_kw,
            rec_start_date: value.rec_start_date,
            confirm_issued: value.confirm_issued,
        }
    }
}

impl From<SyncRunRow> for SyncRun {
    fn from(value: SyncRunRow) -> Self {
        Self {
            id: value.id,
            business_no: value.business_no,
            plant_id: value.plant_id,
            mode: match value.mode.as_str() {
                "delta" => SyncMode::Delta,
                _ => SyncMode::Full,
            },
            status: value.status,
            started_at: Some(value.started_at),
            finished_at: value.finished_at,
            duration_ms: value.duration_ms,
        }
    }
}

impl From<GenerationRow> for GenerationRecord {
    fn from(value: GenerationRow) -> Self {
        Self {
            month: value.month,
            generation_kwh: value.generation_kwh,
            self_consumption_kwh: value.self_consumption_kwh,
            supply_kwh: value.supply_kwh,
            application_deadline: value.application_deadline,
        }
    }
}

impl From<RecRow> for RecRecord {
    fn from(value: RecRow) -> Self {
        Self {
            month: value.month,
            application_no: value.application_no,
            supply_kwh: value.supply_kwh,
            rec_quantity: value.rec_quantity,
            fee_krw: value.fee_krw,
            issued_date: value.issued_date,
            expiry_date: value.expiry_date,
            virtual_account: value.virtual_account,
            bank_name: value.bank_name,
            status: value.status,
        }
    }
}

trait DocumentCategoryExt {
    fn to_storage(&self) -> &'static str;
    fn from_storage(value: &str) -> Option<DocumentCategory>;
}

impl DocumentCategoryExt for DocumentCategory {
    fn to_storage(&self) -> &'static str {
        match self {
            DocumentCategory::PlantInfo => "plant_info",
            DocumentCategory::Permit => "permit",
            DocumentCategory::SiteLand => "site_land",
            DocumentCategory::Drawings => "drawings",
            DocumentCategory::Photos => "photos",
            DocumentCategory::Equipment => "equipment",
            DocumentCategory::Finance => "finance",
            DocumentCategory::Development => "development",
            DocumentCategory::Other => "other",
        }
    }

    fn from_storage(value: &str) -> Option<DocumentCategory> {
        match value {
            "plant_info" => Some(DocumentCategory::PlantInfo),
            "permit" => Some(DocumentCategory::Permit),
            "site_land" => Some(DocumentCategory::SiteLand),
            "drawings" => Some(DocumentCategory::Drawings),
            "photos" => Some(DocumentCategory::Photos),
            "equipment" => Some(DocumentCategory::Equipment),
            "finance" => Some(DocumentCategory::Finance),
            "development" => Some(DocumentCategory::Development),
            "other" => Some(DocumentCategory::Other),
            _ => None,
        }
    }
}
