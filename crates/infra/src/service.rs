use crate::{
    artifacts::ArtifactStore,
    bridge::{self, BridgeDocument, BridgeOptions, BridgeRateRecord},
    config::AppConfig,
    crypto::{CertificateBundle, decode_base64, decrypt_bundle, encrypt_bundle, extract_common_name, normalize_business_no},
    database::{Database, PersistedDocument, PlantSummary, StoredCertificate, StoredDocument, StoredRateRecord},
};
use anyhow::{Context, Result};
use chrono::{Datelike, Utc};
use chrono_tz::Asia::Seoul;
use domain::{DocumentArtifact, DocumentCategory, GenerationRecord, Plant, RateRecord, RecRecord, SyncMode, SyncRun};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};
use tracing::{error, info, warn};

#[derive(Clone)]
pub struct AppService {
    cfg: AppConfig,
    db: Database,
    artifacts: ArtifactStore,
}

#[derive(Debug, Clone)]
pub struct PutCertificateInput {
    pub business_no: String,
    pub password: String,
    pub sign_cert_der_b64: String,
    pub sign_pri_key_b64: String,
    pub km_cert_der_b64: String,
    pub km_pri_key_b64: String,
}

#[derive(Debug, Clone)]
pub struct SyncInput {
    pub business_no: String,
    pub facility_code: Option<String>,
    pub start_month: Option<String>,
    pub end_month: Option<String>,
    pub mode: SyncMode,
}

#[derive(Debug, Clone)]
pub struct PlantView {
    pub plant: Plant,
    pub last_synced_at: Option<String>,
    pub last_sync: Option<SyncRun>,
    pub generation_count: i64,
    pub generation_total_kwh: f64,
    pub rates_count: i64,
    pub rec_count: i64,
    pub rec_total: i64,
    pub document_count: i64,
    pub document_categories: BTreeMap<String, i64>,
}

#[derive(Debug, Clone)]
pub struct BackupInfo {
    pub name: String,
    pub last_modified: i64,
    pub size: Option<u64>,
}

impl AppService {
    pub fn new(cfg: AppConfig, db: Database, artifacts: ArtifactStore) -> Self {
        Self { cfg, db, artifacts }
    }

    pub async fn initialize(&self) -> Result<()> {
        self.db.ensure_schema().await
    }

    pub async fn put_certificate(&self, input: PutCertificateInput) -> Result<StoredCertificate> {
        let business_no = normalize_business_no(&input.business_no)?;
        let sign_cert_der = decode_base64(&input.sign_cert_der_b64)?;
        let sign_pri_key = decode_base64(&input.sign_pri_key_b64)?;
        let km_cert_der = decode_base64(&input.km_cert_der_b64)?;
        let km_pri_key = decode_base64(&input.km_pri_key_b64)?;
        let common_name = extract_common_name(&sign_cert_der)?;

        let bundle = CertificateBundle {
            business_no: business_no.clone(),
            password: input.password,
            common_name: common_name.clone(),
            sign_cert_der,
            sign_pri_key,
            km_cert_der,
            km_pri_key,
        };
        let encrypted = encrypt_bundle(&self.cfg.security.master_key, &bundle)?;
        self.db
            .save_certificate(&business_no, &common_name, &encrypted)
            .await
    }

    pub async fn sync(&self, input: SyncInput) -> Result<SyncRun> {
        let business_no = normalize_business_no(&input.business_no)?;
        let cert = self.db.load_active_certificate(&business_no).await?;
        let bundle = decrypt_bundle(&self.cfg.security.master_key, &cert.encrypted_blob)?;

        let (effective_mode, start_month, end_month, skip_documents) =
            self.resolve_sync_plan(&input).await?;
        info!(
            business_no,
            facility_code = input.facility_code.as_deref().unwrap_or(""),
            mode = ?effective_mode,
            start_month = start_month.as_deref().unwrap_or(""),
            end_month = end_month.as_deref().unwrap_or(""),
            skip_documents,
            "starting sync"
        );
        let run = self
            .db
            .start_sync_run(
                &business_no,
                input.facility_code.as_deref(),
                effective_mode.clone(),
            )
            .await?;

        let started = std::time::Instant::now();
        let collect = self
            .collect_snapshot(
                business_no.clone(),
                input.facility_code.clone(),
                start_month.clone(),
                end_month.clone(),
                skip_documents,
                bundle,
            )
            .await;

        match collect {
            Ok(run_output) => {
                let snapshot = run_output.snapshot;
                info!(
                    business_no,
                    plant_code = snapshot.facility.code,
                    generation_count = snapshot.generation.len(),
                    rate_count = snapshot.rates.len(),
                    rec_count = snapshot.rec.len(),
                    document_count = snapshot.documents.len(),
                    "collector finished"
                );
                let plant = Plant {
                    code: snapshot.facility.code.clone(),
                    name: snapshot.facility.name.clone(),
                    business_no: snapshot.facility.business_no.clone(),
                    contract_no: snapshot.facility.contract_no.clone(),
                    address: snapshot.facility.address.clone(),
                    installed_kw: snapshot.facility.installed_kw,
                    rec_start_date: snapshot.facility.rec_start_date.clone(),
                    confirm_issued: snapshot.facility.confirm_issued.clone(),
                };
                self.db.upsert_plant(&plant).await?;

                let generation: Vec<GenerationRecord> = snapshot
                    .generation
                    .iter()
                    .map(|row| GenerationRecord {
                        month: row.month.clone(),
                        generation_kwh: row.generation_kwh,
                        self_consumption_kwh: row.self_consumption_kwh,
                        supply_kwh: row.supply_kwh,
                        application_deadline: row.application_deadline.clone(),
                    })
                    .collect();
                let rates: Vec<StoredRateRecord> = snapshot
                    .rates
                    .iter()
                    .map(map_rate_record)
                    .collect();
                let rec: Vec<RecRecord> = snapshot.rec.iter().map(map_rec_record).collect();

                if effective_mode == SyncMode::Full {
                    info!(plant_code = plant.code, "replacing canonical full records");
                    self.db
                        .replace_full_records(&plant.code, &generation, &rates, &rec)
                        .await?;
                    info!(plant_code = plant.code, "listing stale document objects");
                    let existing_objects = self.db.list_document_objects(&plant.code).await?;
                    for object_key in existing_objects {
                        if let Err(err) = self.artifacts.delete_object(&object_key).await {
                            warn!("delete stale object {} failed: {}", object_key, err);
                        }
                    }
                    info!(
                        plant_code = plant.code,
                        document_count = snapshot.documents.len(),
                        "persisting documents to object storage"
                    );
                    let persisted_docs =
                        self.persist_documents(&business_no, &plant.code, &snapshot.documents).await?;
                    info!(
                        plant_code = plant.code,
                        document_count = persisted_docs.len(),
                        "writing document rows"
                    );
                    self.db.replace_documents(&plant.code, &persisted_docs).await?;
                } else {
                    info!(plant_code = plant.code, "upserting delta records");
                    self.db.upsert_generation_records(&plant.code, &generation).await?;
                    self.db.upsert_rate_records(&plant.code, &rates).await?;
                    self.db.upsert_rec_records(&plant.code, &rec).await?;
                }

                let duration_ms = started.elapsed().as_millis() as i64;
                self.db
                    .finish_sync_run(&run.id, "completed", duration_ms, None)
                    .await?;
                info!(
                    "sync completed business={} plant={} mode={:?} duration_ms={}",
                    business_no, plant.code, effective_mode, duration_ms
                );

                Ok(SyncRun {
                    id: run.id,
                    business_no,
                    plant_id: Some(plant.code),
                    mode: effective_mode,
                    status: "completed".to_owned(),
                    started_at: run.started_at,
                    finished_at: Some(now_string()),
                    duration_ms,
                })
            }
            Err(err) => {
                let duration_ms = started.elapsed().as_millis() as i64;
                let message = format!("{err:#}");
                error!(
                    business_no,
                    facility_code = input.facility_code.as_deref().unwrap_or(""),
                    mode = ?effective_mode,
                    duration_ms,
                    error = %message,
                    "sync failed"
                );
                let _ = self
                    .db
                    .finish_sync_run(&run.id, "failed", duration_ms, Some(&message))
                    .await;
                Err(err)
            }
        }
    }

    pub async fn list_plants(&self, business_no: &str) -> Result<Vec<Plant>> {
        self.db.list_plants(&normalize_business_no(business_no)?).await
    }

    pub async fn get_plant(&self, facility_code: &str) -> Result<PlantView> {
        let summary = self.db.get_plant_summary(facility_code).await?;
        Ok(map_plant_summary(summary))
    }

    pub async fn list_generation(&self, facility_code: &str) -> Result<Vec<GenerationRecord>> {
        self.db.list_generation(facility_code).await
    }

    pub async fn list_rates(&self, facility_code: &str) -> Result<Vec<RateRecord>> {
        self.db.list_rates(facility_code).await
    }

    pub async fn list_rec(&self, facility_code: &str) -> Result<Vec<RecRecord>> {
        self.db.list_rec(facility_code).await
    }

    pub async fn list_documents(&self, facility_code: &str) -> Result<Vec<DocumentArtifact>> {
        let docs = self.db.list_documents(facility_code).await?;
        Ok(docs
            .into_iter()
            .map(|mut doc| {
                doc.artifact.download_url = Some(format!("/api/v1/documents/{}/download", doc.artifact.id));
                doc.artifact
            })
            .collect())
    }

    pub async fn download_document(&self, document_id: &str) -> Result<(StoredDocument, bytes::Bytes)> {
        let doc = self.db.get_document(document_id).await?;
        let bytes = self.artifacts.get_bytes(&doc.object_key).await?;
        Ok((doc, bytes))
    }

    pub async fn trigger_backup(&self) -> Result<()> {
        self.db.trigger_backup().await
    }

    pub async fn list_backups_local(&self) -> Result<Vec<BackupInfo>> {
        let items = self.db.list_backup_local().await?;
        Ok(items
            .into_iter()
            .map(|item| BackupInfo {
                name: item.name,
                last_modified: item.last_modified,
                size: item.size,
            })
            .collect())
    }

    pub async fn list_backups_s3(&self) -> Result<Vec<BackupInfo>> {
        let items = self.db.list_backup_s3().await?;
        Ok(items
            .into_iter()
            .map(|item| BackupInfo {
                name: item.name,
                last_modified: item.last_modified,
                size: item.size,
            })
            .collect())
    }

    async fn collect_snapshot(
        &self,
        business_no: String,
        facility_code: Option<String>,
        start_month: Option<String>,
        end_month: Option<String>,
        skip_documents: bool,
        bundle: CertificateBundle,
    ) -> Result<bridge::BridgeRun> {
        bridge::collect_snapshot(BridgeOptions {
            business_no,
            facility_code,
            start_month,
            end_month,
            skip_documents,
            bundle,
        })
        .await
    }

    async fn resolve_sync_plan(
        &self,
        input: &SyncInput,
    ) -> Result<(SyncMode, Option<String>, Option<String>, bool)> {
        match input.mode {
            SyncMode::Full => Ok((
                SyncMode::Full,
                input.start_month.clone(),
                input.end_month.clone(),
                false,
            )),
            SyncMode::Delta => {
                let Some(facility_code) = input.facility_code.as_deref() else {
                    return Ok((SyncMode::Full, None, None, false));
                };
                let Some(latest_month) = self.db.latest_generation_month(facility_code).await? else {
                    return Ok((SyncMode::Full, None, None, false));
                };
                let end_month = current_month_seoul();
                Ok((SyncMode::Delta, Some(latest_month), Some(end_month), true))
            }
        }
    }

    async fn persist_documents(
        &self,
        business_no: &str,
        plant_code: &str,
        docs: &[BridgeDocument],
    ) -> Result<Vec<PersistedDocument>> {
        let mut persisted = Vec::with_capacity(docs.len());
        for doc in docs {
            let id = stable_document_id(business_no, plant_code, doc);
            let category = resolved_document_category(doc);
            let object_key = format!("{business_no}/{plant_code}/{id}.pdf");
            let source_path = Path::new(&doc.source_path);
            info!(
                plant_code,
                file_name = doc.file_name.as_str(),
                object_key = object_key.as_str(),
                "uploading document"
            );
            self.artifacts
                .put_file(object_key.clone(), source_path, "application/pdf")
                .await
                .with_context(|| format!("upload document {}", doc.file_name))?;
            persisted.push(PersistedDocument {
                id,
                category,
                attachment_code: doc.attachment_code.clone(),
                file_class: doc.file_class.clone(),
                file_sec_name: doc.file_sec_name.clone(),
                file_name: doc.file_name.clone(),
                required: doc.required,
                source_sha256: doc.source_sha256.clone(),
                output_sha256: doc.output_sha256.clone().or_else(|| doc.source_sha256.clone()),
                object_key,
                content_type: "application/pdf".to_owned(),
            });
        }
        Ok(persisted)
    }
}

fn stable_document_id(
    business_no: &str,
    plant_code: &str,
    doc: &BridgeDocument,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(business_no.as_bytes());
    hasher.update(b":");
    hasher.update(plant_code.as_bytes());
    hasher.update(b":");
    hasher.update(doc.attachment_code.as_bytes());
    hasher.update(b":");
    hasher.update(doc.file_name.as_bytes());
    let digest = hasher.finalize();
    hex::encode(&digest[..16])
}

fn map_rate_record(row: &BridgeRateRecord) -> StoredRateRecord {
    StoredRateRecord {
        record: RateRecord {
            month: row.month.clone(),
            self_rate: row.self_rate,
            national_average: row.national_average,
            nearby_average: row.nearby_average,
            nearby_highest: row.nearby_highest,
        },
        published: row.published,
        source_control_flag: row.source_control_flag.clone(),
    }
}

fn map_rec_record(row: &bridge::BridgeRecRecord) -> RecRecord {
    RecRecord {
        month: row.month.clone(),
        application_no: row.application_no.clone(),
        supply_kwh: row.supply_kwh,
        rec_quantity: row.rec_quantity,
        fee_krw: row.fee_krw,
        issued_date: row.issued_date.clone(),
        expiry_date: row.expiry_date.clone(),
        virtual_account: row.virtual_account.clone(),
        bank_name: row.bank_name.clone(),
        status: row.status.clone(),
    }
}

fn map_document_category(value: &str) -> DocumentCategory {
    match value {
        "plant_info" => DocumentCategory::PlantInfo,
        "permit" => DocumentCategory::Permit,
        "site_land" => DocumentCategory::SiteLand,
        "drawings" => DocumentCategory::Drawings,
        "photos" => DocumentCategory::Photos,
        "equipment" => DocumentCategory::Equipment,
        "finance" => DocumentCategory::Finance,
        "development" => DocumentCategory::Development,
        _ => DocumentCategory::Other,
    }
}

fn resolved_document_category(doc: &BridgeDocument) -> DocumentCategory {
    let explicit = map_document_category(doc.category.trim());
    if explicit != DocumentCategory::Other {
        return explicit;
    }
    classify_document(
        &format!("{} {}", doc.file_sec_name.trim(), doc.file_name.trim()),
        &doc.file_class,
    )
}

fn classify_document(name_hint: &str, file_class: &str) -> DocumentCategory {
    let name = name_hint.to_lowercase();
    let class = file_class.to_lowercase();

    if contains_any(&name, &["내부사진", "현장사진", "공사사진", "사진"]) {
        DocumentCategory::Photos
    } else if contains_any(&name, &["개발행위", "준공검사"]) {
        DocumentCategory::Development
    } else if contains_any(&name, &["설치도면", "단선결선도", "현황도", "결선도", "배치도"]) {
        DocumentCategory::Drawings
    } else if contains_any(&name, &["토지대장", "지적공부", "건축물대장", "등록증명서"]) {
        DocumentCategory::SiteLand
    } else if contains_any(&name, &["허가증", "허가서", "신고필증", "사용전검사", "상업운전개시"]) {
        DocumentCategory::Permit
    } else if contains_any(&name, &["ks 인증서", "구조안전", "접속함", "모듈", "인버터"]) {
        DocumentCategory::Equipment
    } else if contains_any(&name, &["견적서", "세금계산서", "비용 산출내역서"]) {
        DocumentCategory::Finance
    } else if contains_any(&name, &["사업자등록증"]) {
        DocumentCategory::PlantInfo
    } else if class.contains("소요자금") {
        DocumentCategory::Finance
    } else if class.contains("개발행위") {
        DocumentCategory::Development
    } else if class.contains("설비구성") {
        DocumentCategory::Equipment
    } else if class.contains("발전소정보") {
        DocumentCategory::PlantInfo
    } else if class.contains("발전방식") {
        DocumentCategory::SiteLand
    } else {
        DocumentCategory::Other
    }
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(&needle.to_lowercase()))
}

fn map_plant_summary(summary: PlantSummary) -> PlantView {
    let last_synced_at = summary
        .last_sync
        .as_ref()
        .and_then(|run| run.finished_at.clone().or_else(|| run.started_at.clone()));
    let last_sync = summary.last_sync;
    let document_categories = summary
        .document_categories
        .into_iter()
        .map(|(category, count)| (document_category_str(&category).to_owned(), count))
        .collect();
    PlantView {
        plant: summary.plant,
        last_synced_at,
        last_sync,
        generation_count: summary.generation_count,
        generation_total_kwh: summary.generation_total_kwh,
        rates_count: summary.rates_count,
        rec_count: summary.rec_count,
        rec_total: summary.rec_total,
        document_count: summary.document_count,
        document_categories,
    }
}

fn document_category_str(category: &DocumentCategory) -> &'static str {
    match category {
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

fn current_month_seoul() -> String {
    let now = Utc::now().with_timezone(&Seoul);
    format!("{:04}-{:02}", now.year(), now.month())
}

fn now_string() -> String {
    Utc::now().to_rfc3339()
}
