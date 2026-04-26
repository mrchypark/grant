use crate::{
    crypto::CertificateBundle,
    npki::{NpkiBundle, NpkiSigner},
};
use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, Utc};
use chrono_tz::Asia::Seoul;
use curl::easy::{Easy, List};
use regex::Regex;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Duration,
};
use tempfile::TempDir;

const DEFAULT_BASE_URL: &str = "https://rps.energy.or.kr";
const DEFAULT_KCERT_BASE_URL: &str = "https://kcert.energy.or.kr";

#[derive(Debug, Clone)]
pub struct BridgeOptions {
    pub business_no: String,
    pub facility_code: Option<String>,
    pub start_month: Option<String>,
    pub end_month: Option<String>,
    pub skip_documents: bool,
    pub bundle: CertificateBundle,
}

pub struct BridgeRun {
    pub temp_dir: TempDir,
    pub snapshot: BridgeSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeFacility {
    pub code: String,
    pub name: String,
    pub business_no: String,
    pub contract_no: String,
    pub address: String,
    pub installed_kw: f64,
    pub rec_start_date: String,
    pub confirm_issued: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeGenerationRecord {
    pub month: String,
    pub generation_kwh: f64,
    pub self_consumption_kwh: Option<f64>,
    pub supply_kwh: f64,
    pub application_deadline: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeRateRecord {
    pub month: String,
    pub self_rate: Option<f64>,
    pub national_average: Option<f64>,
    pub nearby_average: Option<f64>,
    pub nearby_highest: Option<f64>,
    pub published: bool,
    pub source_control_flag: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeRecRecord {
    pub month: String,
    pub application_no: String,
    pub supply_kwh: i64,
    pub rec_quantity: i64,
    pub fee_krw: i64,
    pub issued_date: String,
    pub expiry_date: String,
    pub virtual_account: Option<String>,
    pub bank_name: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeDocument {
    pub attachment_code: String,
    pub file_class: String,
    pub file_sec_name: String,
    pub file_name: String,
    pub required: bool,
    pub category: String,
    pub source_path: String,
    pub source_sha256: Option<String>,
    pub output_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_base64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeSnapshot {
    pub collected_at: DateTime<FixedOffset>,
    pub facility: BridgeFacility,
    pub generation: Vec<BridgeGenerationRecord>,
    pub rates: Vec<BridgeRateRecord>,
    pub rec: Vec<BridgeRecRecord>,
    pub documents: Vec<BridgeDocument>,
}

#[derive(Debug, Clone)]
struct KCertLoginPage {
    login_id: String,
    sys_code: String,
    user_ssn_type: String,
    return_page: String,
    server_km_cert_pem: String,
    id1_field_name: String,
    id2_field_name: String,
}

#[derive(Debug, Clone)]
struct KCertResult {
    code: String,
    message: String,
    id1: String,
    id2: String,
    name: String,
    birth_ymd: String,
    ci: String,
}

#[derive(Debug, Clone)]
struct LoginResult {
    ente_id: String,
}

#[derive(Debug, Clone)]
struct FacilityRecord {
    ente_id: String,
    prvs_id: String,
    prvs_app_no: String,
    code: String,
    name: String,
    business_no: String,
    contract_no: String,
    address: String,
    installed_kw: f64,
    rec_start_date: String,
    confirm_date: String,
}

#[derive(Debug, Clone)]
struct CompletionRecord {
    application_no: String,
    facility_code: String,
    submitted_at: String,
    first_applied: String,
    app_sec: String,
}

#[derive(Debug, Clone)]
struct AttachmentRecord {
    attachment_code: String,
    file_class: String,
    file_sec_name: String,
    file_name: String,
    file_path: String,
    required: bool,
}

#[derive(Debug, Clone)]
struct GenerationFetchRecord {
    row_id: String,
    record: BridgeGenerationRecord,
}

#[derive(Debug, Deserialize)]
struct JsonListResponse {
    #[serde(default)]
    list: Vec<Map<String, Value>>,
}

pub async fn collect_snapshot(opts: BridgeOptions) -> Result<BridgeRun> {
    let temp_dir = TempDir::new().context("create collector temp dir")?;
    let cookie_jar_path = temp_dir.path().join("curl-cookie-jar.txt");
    std::fs::write(&cookie_jar_path, b"").context("initialize curl cookie jar")?;
    let signer = NpkiSigner::new(
        &bundle_from_certificate(&opts.bundle),
        &opts.bundle.password,
    )
    .context("initialize NPKI signer")?;
    let collector = Collector::new(signer, cookie_jar_path);
    let snapshot = collector
        .collect_snapshot(
            temp_dir.path(),
            &normalize_digits(&opts.business_no),
            opts.facility_code.as_deref(),
            opts.start_month.as_deref(),
            opts.end_month.as_deref(),
            opts.skip_documents,
        )
        .await?;
    Ok(BridgeRun { temp_dir, snapshot })
}

struct Collector {
    signer: NpkiSigner,
    cookie_jar_path: PathBuf,
}

impl Collector {
    fn new(signer: NpkiSigner, cookie_jar_path: PathBuf) -> Self {
        Self {
            signer,
            cookie_jar_path,
        }
    }

    async fn collect_snapshot(
        &self,
        root_dir: &Path,
        business_no: &str,
        facility_code: Option<&str>,
        start_month: Option<&str>,
        end_month: Option<&str>,
        skip_documents: bool,
    ) -> Result<BridgeSnapshot> {
        let login = self.login(business_no).await?;
        let mut facilities = self.fetch_facilities(&login.ente_id).await?;
        let mut facility = select_facility(&mut facilities, facility_code)?;
        if facility.business_no.is_empty() {
            facility.business_no = business_no.to_owned();
        }
        self.enrich_facility(&mut facility).await?;
        let completions = self.fetch_completion_records(&login.ente_id).await?;

        let (start_month, end_month) =
            resolve_range(start_month, end_month, &facility, &completions)?;
        let generation_rows = self
            .fetch_generation(&login.ente_id, &start_month, &end_month)
            .await?;
        let generation = generation_rows
            .iter()
            .map(|item| item.record.clone())
            .collect();
        let rates = self.fetch_rates(&generation_rows).await?;
        let rec = self
            .fetch_rec(&login.ente_id, &start_month, &end_month)
            .await?;
        let documents = if skip_documents {
            Vec::new()
        } else {
            self.fetch_documents(root_dir, &facility, &completions)
                .await?
        };

        Ok(BridgeSnapshot {
            collected_at: Utc::now().with_timezone(&Seoul).fixed_offset(),
            facility: BridgeFacility {
                code: facility.code,
                name: facility.name,
                business_no: facility.business_no,
                contract_no: facility.contract_no,
                address: facility.address,
                installed_kw: facility.installed_kw,
                rec_start_date: facility.rec_start_date,
                confirm_issued: facility.confirm_date,
            },
            generation,
            rates,
            rec,
            documents,
        })
    }

    async fn login(&self, business_no: &str) -> Result<LoginResult> {
        let page = self.fetch_kcert_login_page().await?;
        let result = self.submit_kcert(&page, business_no).await?;
        if result.code != "1" {
            bail!("KCert login rejected: {} ({})", result.message, result.code);
        }
        let main_html = self.submit_return_form(&page, &result).await?;
        let ente_id = extract_ente_id(&main_html)?;
        Ok(LoginResult { ente_id })
    }

    async fn fetch_kcert_login_page(&self) -> Result<KCertLoginPage> {
        let body = self
            .post_form_text(
                &format!("{DEFAULT_KCERT_BASE_URL}/keaDev/Login/CertLoginHTML5.do"),
                &[
                    ("sysCode", "NRPS"),
                    ("userSsnType", "1"),
                    ("rtnPage", &format!("{DEFAULT_BASE_URL}/loginProc2_main.do")),
                ],
                Some(headers(&[
                    ("Referer", &format!("{DEFAULT_BASE_URL}/login2.do")),
                    ("Origin", DEFAULT_BASE_URL),
                ])),
            )
            .await?;
        parse_kcert_login_page(&body)
    }

    async fn submit_kcert(&self, page: &KCertLoginPage, business_no: &str) -> Result<KCertResult> {
        #[derive(Deserialize)]
        struct Payload {
            result: String,
        }

        let user_sign_value = self
            .signer
            .sign_pkcs7_pem(page.login_id.as_bytes())
            .context("sign LoginID")?;
        let encrypted_ssn = self
            .signer
            .envelop_pkcs7_pem(page.server_km_cert_pem.as_bytes(), business_no.as_bytes())
            .context("encrypt business number")?;
        let user_sign_cert = self.signer.certificate_pem()?;

        let payload: Payload = self
            .post_form_json(
                &format!("{DEFAULT_KCERT_BASE_URL}/keaDev/LOGIN/CertLoginEwsCheck.do"),
                &[
                    ("UserSignCert", &user_sign_cert),
                    ("UserSignValue", &user_sign_value),
                    ("EncryptedUserSSN", &encrypted_ssn),
                    ("EncryptedUserID", self.signer.vid_random_b64()),
                    ("UserSSNValue", business_no),
                    ("CertId", &format!("{}@signgate.com", page.login_id)),
                    ("LoginID", &page.login_id),
                    ("SysCode", &page.sys_code),
                    ("userSsnType", &page.user_ssn_type),
                ],
                Some(headers(&[
                    (
                        "Referer",
                        &format!("{DEFAULT_KCERT_BASE_URL}/keaDev/Login/CertLoginHTML5.do"),
                    ),
                    ("Origin", DEFAULT_KCERT_BASE_URL),
                ])),
            )
            .await?;

        parse_kcert_result(&payload.result)
    }

    async fn submit_return_form(
        &self,
        page: &KCertLoginPage,
        result: &KCertResult,
    ) -> Result<String> {
        self.post_form_text(
            &page.return_page,
            &[
                ("rtnPage", &page.return_page),
                (&page.id1_field_name, &result.id1),
                (&page.id2_field_name, &result.id2),
                ("id3", &result.id1),
                ("id4", &result.id2),
                ("ofcrcgAuceKey", &result.id1),
                ("sysCode", &page.sys_code),
                ("Name", &result.name),
                ("birthYMD", &result.birth_ymd),
                ("CI", &result.ci),
            ],
            Some(headers(&[
                (
                    "Referer",
                    &format!("{DEFAULT_KCERT_BASE_URL}/keaDev/Login/CertLoginHTML5.do"),
                ),
                ("Origin", DEFAULT_KCERT_BASE_URL),
            ])),
        )
        .await
    }

    async fn fetch_facilities(&self, ente_id: &str) -> Result<Vec<FacilityRecord>> {
        let response: JsonListResponse = self
            .post_form_json(
                &format!("{DEFAULT_BASE_URL}/comm/SP_RPS_G.do"),
                &[
                    ("P_BIZ_SEC", "%"),
                    ("P_ENGY", "%"),
                    ("P_CANC_YN", "%"),
                    ("P_POPLNT_TERM", ""),
                    ("P_PRVS_CODE", ""),
                    ("P_SNUMB", "0"),
                    ("P_PSIZE", "1000"),
                    ("P_ENTE_ID", ente_id),
                    ("P_PKG_NM", "CST_A1_01_01_260_L"),
                    ("P_PAGE_YN", "Y"),
                ],
                None,
            )
            .await?;

        let mut facilities = response
            .list
            .into_iter()
            .map(|row| FacilityRecord {
                ente_id: string_value(&row, &["ENTE_ID"]),
                prvs_id: string_value(&row, &["PRVS_ID"]),
                prvs_app_no: string_value(&row, &["PRVS_APP_NO"]),
                code: string_value(&row, &["PRVS_CODE"]),
                name: string_value(&row, &["POPLNT_TERM"]),
                business_no: String::new(),
                contract_no: String::new(),
                address: String::new(),
                installed_kw: float_value(&row, &["POPLNT_INST_CAPA"]),
                rec_start_date: normalize_date(&string_value(&row, &["CRTIF_STRT_DD"])),
                confirm_date: normalize_date(&string_value(&row, &["PRVS_CNFM_ISUE_DD"])),
            })
            .collect::<Vec<_>>();
        facilities.sort_by(|a, b| a.code.cmp(&b.code));
        Ok(facilities)
    }

    async fn fetch_generation(
        &self,
        ente_id: &str,
        start_month: &str,
        end_month: &str,
    ) -> Result<Vec<GenerationFetchRecord>> {
        let chunks = month_chunks(start_month, end_month, 12)?;
        let mut records = Vec::new();

        for (chunk_start, chunk_end) in chunks {
            let response: JsonListResponse = self
                .post_form_json(
                    &format!("{DEFAULT_BASE_URL}/comm/SP_RPS_G2.do"),
                    &[
                        ("P_SDT", &chunk_start),
                        ("P_EDT", &chunk_end),
                        ("P_MNG_ORG", "%"),
                        ("P_STAT_INFO", "%"),
                        ("P_POPLNT_TERM", ""),
                        ("P_SNUMB", "0"),
                        ("P_PSIZE", "1000"),
                        ("P_ENTE_ID", ente_id),
                        ("P_PKG_NM", "O2_02_02_010_S"),
                        ("P_PAGE_YN", "Y"),
                    ],
                    None,
                )
                .await?;

            for row in response.list {
                records.push(GenerationFetchRecord {
                    row_id: string_value(&row, &["GELEC_ID"]),
                    record: BridgeGenerationRecord {
                        month: normalize_month(&string_value(&row, &["GELEC_YYMM"])),
                        generation_kwh: float_value(&row, &["GELEC_QTY"]),
                        self_consumption_kwh: float_option(&row, &["HOUSE_CNSM_QTY"]),
                        supply_kwh: float_value(&row, &["SPLY_ELCP_QTY"]),
                        application_deadline: string_option(&normalize_date(&string_value(
                            &row,
                            &["APP_END_DD"],
                        ))),
                    },
                });
            }
        }

        records.sort_by(|a, b| a.record.month.cmp(&b.record.month));
        Ok(dedupe_generation(records))
    }

    async fn fetch_rates(
        &self,
        generation: &[GenerationFetchRecord],
    ) -> Result<Vec<BridgeRateRecord>> {
        let mut records = Vec::with_capacity(generation.len());
        for item in generation {
            if item.row_id.is_empty() {
                continue;
            }
            let response: JsonListResponse = self
                .post_form_json(
                    &format!("{DEFAULT_BASE_URL}/comm/SP_RPS_G.do"),
                    &[("P_GELEC_ID", &item.row_id), ("P_PKG_NM", "O2_02_02_026_S")],
                    None,
                )
                .await?;

            let payload = response.list.into_iter().next().unwrap_or_default();
            let control = first_non_empty(&[
                string_value(&payload, &["CONTROL"]),
                string_value(&payload, &["CONTROL_YN"]),
            ]);

            records.push(BridgeRateRecord {
                month: item.record.month.clone(),
                self_rate: float_ptr_from_value(payload.get("ONE_RATE")),
                national_average: float_ptr_from_value(payload.get("TWO_RATE")),
                nearby_highest: float_ptr_from_value(payload.get("THREE_RATE")),
                nearby_average: float_ptr_from_value(payload.get("FOUR_RATE")),
                published: control.to_uppercase() != "N",
                source_control_flag: string_option(&control),
            });
        }
        Ok(records)
    }

    async fn fetch_rec(
        &self,
        ente_id: &str,
        start_month: &str,
        end_month: &str,
    ) -> Result<Vec<BridgeRecRecord>> {
        let chunks = month_chunks(start_month, end_month, 12)?;
        let mut records = Vec::new();

        for (chunk_start, chunk_end) in chunks {
            let start_dense = chunk_start.replace('-', "");
            let end_dense = chunk_end.replace('-', "");
            let response: JsonListResponse = self
                .post_form_json(
                    &format!("{DEFAULT_BASE_URL}/comm/SP_RPS_G.do"),
                    &[
                        ("P_GELEC_YYMM_FROM", &start_dense),
                        ("P_GELEC_YYMM_TO", &end_dense),
                        ("P_ENTE_ID", ente_id),
                        ("P_SNUMB", "0"),
                        ("P_PSIZE", "1000"),
                        ("P_PKG_NM", "O2_05_02_010_S"),
                    ],
                    None,
                )
                .await?;

            for row in response.list {
                let application_no = string_value(&row, &["APP_NO"]);
                if application_no.is_empty() {
                    continue;
                }
                records.push(BridgeRecRecord {
                    month: normalize_month(&string_value(&row, &["GELEC_YYMM"])),
                    application_no,
                    supply_kwh: int_value(&row, &["SPLY_ELCP_QTY"]),
                    rec_quantity: int_value(&row, &["REC_QTY"]),
                    fee_krw: int_value(&row, &["ISUE_CMS"]),
                    issued_date: normalize_date(&string_value(&row, &["ISUE_DD"])),
                    expiry_date: normalize_date(&string_value(&row, &["END_DD"])),
                    virtual_account: string_option(&string_value(&row, &["VACCT_NO"])),
                    bank_name: string_option(&string_value(&row, &["BANK_NAME"])),
                    status: string_option(&first_non_empty(&[
                        string_value(&row, &["STAT_INFO_NM"]),
                        string_value(&row, &["STAT_NAME"]),
                    ])),
                });
            }
        }

        records.sort_by(|a, b| a.month.cmp(&b.month));
        Ok(dedupe_rec(records))
    }

    async fn fetch_completion_records(&self, ente_id: &str) -> Result<Vec<CompletionRecord>> {
        let response: JsonListResponse = self
            .post_form_json(
                &format!("{DEFAULT_BASE_URL}/comm/SP_RPS_G.do"),
                &[
                    ("P_ENTE_ID", ente_id),
                    ("P_STAT_INFO", "%"),
                    ("P_POPLNT_TERM", ""),
                    ("P_SNUMB", "0"),
                    ("P_PSIZE", "1000"),
                    ("P_PKG_NM", "O2_04_02_010_S"),
                ],
                None,
            )
            .await?;

        Ok(response
            .list
            .into_iter()
            .map(|row| CompletionRecord {
                application_no: string_value(&row, &["PRVS_APP_NO"]),
                facility_code: string_value(&row, &["PRVS_CODE"]),
                submitted_at: normalize_date(&string_value(&row, &["REPT_DD"])),
                first_applied: normalize_date(&string_value(&row, &["FRST_APP_DD"])),
                app_sec: map_app_sec(
                    &string_value(&row, &["APP_SEC"]),
                    &string_value(&row, &["APP_SEC_NM"]),
                ),
            })
            .collect())
    }

    async fn fetch_documents(
        &self,
        root_dir: &Path,
        facility: &FacilityRecord,
        completions: &[CompletionRecord],
    ) -> Result<Vec<BridgeDocument>> {
        let download_root = root_dir
            .join("downloads")
            .join("live")
            .join(safe_download_segment("facilityCode", &facility.code)?);
        if download_root.exists() {
            std::fs::remove_dir_all(&download_root)
                .with_context(|| format!("reset {}", download_root.display()))?;
        }

        let mut documents = Vec::new();
        for completion in completions {
            if completion.application_no.is_empty() || completion.facility_code != facility.code {
                continue;
            }

            let attachments = self.fetch_attachments(completion).await?;
            for attachment in attachments {
                if attachment.file_path.is_empty() || attachment.file_name.is_empty() {
                    continue;
                }
                let content = self
                    .download_file(&attachment.file_path, &attachment.file_name)
                    .await?;
                let source_path = write_downloaded_file(
                    &download_root,
                    &completion.application_no,
                    &attachment.file_name,
                    &content,
                )?;
                documents.push(BridgeDocument {
                    attachment_code: attachment.attachment_code,
                    file_class: attachment.file_class,
                    file_sec_name: attachment.file_sec_name,
                    file_name: attachment.file_name,
                    required: attachment.required,
                    category: String::new(),
                    source_path,
                    source_sha256: None,
                    output_sha256: None,
                    content_base64: None,
                    content_type: None,
                });
            }
        }

        documents.sort_by(|a, b| {
            a.attachment_code
                .cmp(&b.attachment_code)
                .then_with(|| a.file_name.cmp(&b.file_name))
        });
        Ok(documents)
    }

    async fn enrich_facility(&self, facility: &mut FacilityRecord) -> Result<()> {
        let master: Map<String, Value> = self
            .post_form_json(
                &format!("{DEFAULT_BASE_URL}/comm/SP_RPS_G.do"),
                &[
                    ("P_PRVS_ID", &facility.prvs_id),
                    ("P_PKG_NM", "CST_A1_00_GET_PRVS_MST"),
                ],
                None,
            )
            .await?;

        facility.business_no = normalize_digits(&first_non_empty(&[
            string_value(&master, &["APP_BRNUM"]),
            facility.business_no.clone(),
        ]));
        facility.name = first_non_empty(&[
            string_value(&master, &["POPLNT_TERM"]),
            facility.name.clone(),
        ]);
        facility.installed_kw = first_non_zero(&[
            float_value(&master, &["POPLNT_INST_CAPA"]),
            facility.installed_kw,
        ]);
        facility.rec_start_date = first_non_empty(&[
            normalize_date(&string_value(&master, &["CRTIF_STRT_DD"])),
            facility.rec_start_date.clone(),
        ]);
        facility.confirm_date = first_non_empty(&[
            normalize_date(&string_value(&master, &["PRVS_CNFM_ISUE_DD"])),
            facility.confirm_date.clone(),
        ]);
        facility.address = join_non_empty(&[
            string_value(&master, &["POPLNT_ZIP_ADDR"]),
            string_value(&master, &["POPLNT_DTLS_ADDR"]),
        ]);

        let contracts: JsonListResponse = self
            .post_form_json(
                &format!("{DEFAULT_BASE_URL}/comm/SP_RPS_L.do"),
                &[
                    ("P_PRVS_APP_NO", &facility.prvs_app_no),
                    ("P_PKG_NM", "CST_A1_00_GELEC_CNTR_L"),
                ],
                None,
            )
            .await?;
        if let Some(contract) = contracts.list.first() {
            facility.contract_no = first_non_empty(&[
                string_value(contract, &["POPLNT_CNTR_NO"]),
                facility.contract_no.clone(),
            ]);
        }

        if facility.address.is_empty() {
            let locations: JsonListResponse = self
                .post_form_json(
                    &format!("{DEFAULT_BASE_URL}/comm/SP_RPS_L.do"),
                    &[
                        ("P_PRVS_APP_NO", &facility.prvs_app_no),
                        ("P_ENTE_ID", &facility.ente_id),
                        ("P_PKG_NM", "CST_A1_01_01_500_L"),
                    ],
                    None,
                )
                .await?;
            if let Some(location) = locations.list.first() {
                facility.address = first_non_empty(&[
                    join_non_empty(&[
                        string_value(location, &["LOT_ADDR"]),
                        string_value(location, &["DTL_ADDR"]),
                    ]),
                    join_non_empty(&[
                        string_value(location, &["ROAD_TERM_ADDR"]),
                        string_value(location, &["DTL_ADDR"]),
                    ]),
                    facility.address.clone(),
                ]);
            }
        }

        Ok(())
    }

    async fn fetch_attachments(
        &self,
        completion: &CompletionRecord,
    ) -> Result<Vec<AttachmentRecord>> {
        let response: JsonListResponse = self
            .post_form_json(
                &format!("{DEFAULT_BASE_URL}/comm/SP_RPS_NP_L.do"),
                &[
                    ("P_PKG_NM", "CST_A1_00_PRVS_APP_FILE_L"),
                    ("P_SEC", "L"),
                    ("P_APP_SEC", &completion.app_sec),
                    ("P_PRVS_APP_NO", &completion.application_no),
                ],
                None,
            )
            .await?;

        Ok(response
            .list
            .into_iter()
            .filter_map(|row| {
                let file_name = first_non_empty(&[
                    string_value(&row, &["ACTL_FILE_NM"]),
                    string_value(&row, &["FILE_TERM"]),
                ]);
                let file_path = string_value(&row, &["FILE_PATH"]);
                if file_name.is_empty() || file_path.is_empty() {
                    return None;
                }
                Some(AttachmentRecord {
                    attachment_code: string_value(&row, &["ATCH_FILE_CODE"]),
                    file_class: string_value(&row, &["FILE_CLASS"]),
                    file_sec_name: string_value(&row, &["FILE_SEC_NM"]),
                    file_name,
                    file_path,
                    required: string_value(&row, &["ESEN_YN_NM"]).contains("필수"),
                })
            })
            .collect())
    }

    async fn download_file(&self, file_path: &str, file_name: &str) -> Result<Vec<u8>> {
        self.post_form_bytes(
            &format!("{DEFAULT_BASE_URL}/comm/SP_RPS_FILE_DOWN.do"),
            &[
                ("P_PKG_NM", "O1_02_01_030_DOWN"),
                ("hidFILE_PATH", file_path),
                ("hidFILE_TERM", file_name),
            ],
            Some(headers(&[("Connection", "close")])),
        )
        .await
        .with_context(|| format!("download file {file_name}"))
    }

    async fn post_form_json<T>(
        &self,
        endpoint: &str,
        form: &[(&str, &str)],
        headers: Option<Vec<String>>,
    ) -> Result<T>
    where
        T: DeserializeOwned,
    {
        for attempt in 1..=3 {
            let (status, body) = match self.post_form(endpoint, form, headers.clone()) {
                Ok(response) => response,
                Err(_err) if attempt < 3 => {
                    tokio::time::sleep(retry_delay(attempt)).await;
                    continue;
                }
                Err(err) => return Err(err),
            };
            let body = String::from_utf8(body)
                .with_context(|| format!("decode UTF-8 body from {endpoint}"))?;
            if !(200..300).contains(&status) {
                bail!("POST {endpoint}: status {}: {}", status, body.trim());
            }
            return serde_json::from_str(&body).with_context(|| format!("decode {endpoint}"));
        }
        unreachable!("retry loop must return")
    }

    async fn post_form_text(
        &self,
        endpoint: &str,
        form: &[(&str, &str)],
        headers: Option<Vec<String>>,
    ) -> Result<String> {
        for attempt in 1..=3 {
            let (status, body) = match self.post_form(endpoint, form, headers.clone()) {
                Ok(response) => response,
                Err(_err) if attempt < 3 => {
                    tokio::time::sleep(retry_delay(attempt)).await;
                    continue;
                }
                Err(err) => return Err(err),
            };
            let body = String::from_utf8(body)
                .with_context(|| format!("decode UTF-8 body from {endpoint}"))?;
            if !(200..300).contains(&status) {
                bail!("POST {endpoint}: status {}: {}", status, body.trim());
            }
            return Ok(body);
        }
        unreachable!("retry loop must return")
    }

    async fn post_form_bytes(
        &self,
        endpoint: &str,
        form: &[(&str, &str)],
        headers: Option<Vec<String>>,
    ) -> Result<Vec<u8>> {
        for attempt in 1..=3 {
            let (status, body) = match self.post_form(endpoint, form, headers.clone()) {
                Ok(response) => response,
                Err(_err) if attempt < 3 => {
                    tokio::time::sleep(retry_delay(attempt)).await;
                    continue;
                }
                Err(err) => return Err(err),
            };
            if !(200..300).contains(&status) {
                let sample = String::from_utf8_lossy(&body);
                bail!("POST {endpoint}: status {}: {}", status, sample.trim());
            }
            return Ok(body);
        }
        unreachable!("retry loop must return")
    }

    fn post_form(
        &self,
        endpoint: &str,
        form: &[(&str, &str)],
        headers: Option<Vec<String>>,
    ) -> Result<(u32, Vec<u8>)> {
        let encoded = encode_form(form);
        let cookie_path = self.cookie_jar_path.to_string_lossy().into_owned();
        let header_lines = headers.unwrap_or_default();

        let mut easy = Easy::new();
        easy.url(endpoint)
            .with_context(|| format!("set URL {endpoint}"))?;
        easy.post(true).context("enable POST")?;
        easy.follow_location(true).context("enable redirects")?;
        easy.useragent("curl/8.7.1")
            .context("set curl user agent")?;
        easy.http_version(curl::easy::HttpVersion::V11)
            .context("force HTTP/1.1")?;
        easy.connect_timeout(Duration::from_secs(30))
            .context("set connect timeout")?;
        easy.timeout(Duration::from_secs(300))
            .context("set transfer timeout")?;
        easy.tcp_keepalive(true).context("enable TCP keepalive")?;
        easy.cookie_file(&cookie_path)
            .with_context(|| format!("load cookie jar {cookie_path}"))?;
        easy.cookie_jar(&cookie_path)
            .with_context(|| format!("persist cookie jar {cookie_path}"))?;
        easy.post_fields_copy(encoded.as_bytes())
            .context("set POST body")?;

        let mut header_list = List::new();
        header_list
            .append("Content-Type: application/x-www-form-urlencoded")
            .context("append content-type header")?;
        header_list
            .append("Accept: */*")
            .context("append accept header")?;
        for header in header_lines {
            header_list
                .append(&header)
                .with_context(|| format!("append header {header}"))?;
        }
        easy.http_headers(header_list).context("set curl headers")?;

        let mut body = Vec::new();
        {
            let mut transfer = easy.transfer();
            transfer
                .write_function(|data| {
                    body.extend_from_slice(data);
                    Ok(data.len())
                })
                .context("install curl write callback")?;
            transfer
                .perform()
                .with_context(|| format!("POST {endpoint}"))?;
        }

        let status = easy.response_code().context("read curl response code")?;
        Ok((status, body))
    }
}

fn retry_delay(attempt: usize) -> Duration {
    Duration::from_millis(250 * attempt as u64)
}

fn encode_form(form: &[(&str, &str)]) -> String {
    form.iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                urlencoding::encode(key),
                urlencoding::encode(value)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn bundle_from_certificate(bundle: &CertificateBundle) -> NpkiBundle {
    NpkiBundle {
        sign_cert_der: bundle.sign_cert_der.clone(),
        sign_pri_key: bundle.sign_pri_key.clone(),
        km_cert_der: bundle.km_cert_der.clone(),
        km_pri_key: bundle.km_pri_key.clone(),
    }
}

fn parse_kcert_login_page(body: &str) -> Result<KCertLoginPage> {
    let inputs = parse_inputs(body);
    let page = KCertLoginPage {
        login_id: input_value(&inputs, "LoginID"),
        sys_code: input_value(&inputs, "SysCode"),
        user_ssn_type: input_value(&inputs, "userSsnType"),
        return_page: input_value(&inputs, "rtnPage"),
        server_km_cert_pem: input_value(&inputs, "ServerKmCert"),
        id1_field_name: input_name(&inputs, "id1"),
        id2_field_name: input_name(&inputs, "id2"),
    };

    if page.login_id.is_empty() {
        bail!("parse KCert login page: missing LoginID");
    }
    if page.sys_code.is_empty() {
        bail!("parse KCert login page: missing SysCode");
    }
    if page.return_page.is_empty() {
        bail!("parse KCert login page: missing rtnPage");
    }
    if page.id1_field_name.is_empty() {
        bail!("parse KCert login page: missing id1 field name");
    }
    if page.id2_field_name.is_empty() {
        bail!("parse KCert login page: missing id2 field name");
    }
    if page.server_km_cert_pem.is_empty() {
        bail!("parse KCert login page: missing ServerKmCert");
    }

    Ok(page)
}

fn parse_kcert_result(result: &str) -> Result<KCertResult> {
    let parts = result.split("::").collect::<Vec<_>>();
    if parts.len() < 7 {
        bail!(
            "parse KCert result: expected at least 7 fields, got {}",
            parts.len()
        );
    }
    Ok(KCertResult {
        code: parts[0].to_owned(),
        message: parts[1].to_owned(),
        id1: parts[2].to_owned(),
        id2: parts[3].to_owned(),
        name: parts[4].to_owned(),
        birth_ymd: parts[5].to_owned(),
        ci: parts[6].to_owned(),
    })
}

fn extract_ente_id(body: &str) -> Result<String> {
    let Some(captures) = ente_id_pattern().captures(body) else {
        bail!("extract P_ENTE_ID: not found");
    };
    Ok(captures
        .get(1)
        .map(|value| value.as_str().to_owned())
        .unwrap_or_default())
}

fn parse_inputs(body: &str) -> BTreeMap<String, InputField> {
    input_pattern()
        .captures_iter(body)
        .filter_map(|capture| {
            let id = capture.get(1)?.as_str().to_owned();
            let name = capture.get(2)?.as_str().to_owned();
            let value = capture.get(3)?.as_str().to_owned();
            Some((id, InputField { name, value }))
        })
        .collect()
}

fn input_value(fields: &BTreeMap<String, InputField>, id: &str) -> String {
    fields
        .get(id)
        .map(|field| field.value.clone())
        .unwrap_or_default()
}

fn input_name(fields: &BTreeMap<String, InputField>, id: &str) -> String {
    fields
        .get(id)
        .map(|field| field.name.clone())
        .unwrap_or_default()
}

fn select_facility(
    facilities: &mut [FacilityRecord],
    requested_code: Option<&str>,
) -> Result<FacilityRecord> {
    if facilities.is_empty() {
        bail!("no facilities returned");
    }
    if let Some(code) = requested_code.filter(|value| !value.is_empty()) {
        return facilities
            .iter()
            .find(|facility| facility.code == code)
            .cloned()
            .ok_or_else(|| anyhow!("facility {code:?} not found"));
    }
    if facilities.len() > 1 {
        bail!("multiple facilities returned; specify facilityCode");
    }
    Ok(facilities[0].clone())
}

fn resolve_range(
    requested_start: Option<&str>,
    requested_end: Option<&str>,
    facility: &FacilityRecord,
    completions: &[CompletionRecord],
) -> Result<(String, String)> {
    let end_month = requested_end
        .map(str::to_owned)
        .unwrap_or_else(current_month_seoul);
    let start_month = if let Some(start) = requested_start {
        start.to_owned()
    } else {
        earliest_plant_history_month(facility, completions)
            .ok_or_else(|| anyhow!("cannot infer start month: plant history is empty"))?
    };

    parse_month(&start_month).with_context(|| format!("invalid start month {start_month:?}"))?;
    parse_month(&end_month).with_context(|| format!("invalid end month {end_month:?}"))?;
    Ok((start_month, end_month))
}

fn earliest_plant_history_month(
    facility: &FacilityRecord,
    completions: &[CompletionRecord],
) -> Option<String> {
    let mut candidates = vec![
        month_from_date(&facility.rec_start_date),
        month_from_date(&facility.confirm_date),
    ];

    for completion in completions {
        if !completion.facility_code.is_empty() && completion.facility_code != facility.code {
            continue;
        }
        candidates.push(month_from_date(&completion.first_applied));
        candidates.push(month_from_date(&completion.submitted_at));
    }

    candidates.into_iter().flatten().min()
}

fn month_from_date(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.len() >= 7 {
        let month = &trimmed[..7];
        if parse_month(month).is_ok() {
            return Some(month.to_owned());
        }
    }
    let normalized = normalize_month(trimmed);
    if parse_month(&normalized).is_ok() {
        Some(normalized)
    } else {
        None
    }
}

fn month_chunks(
    start_month: &str,
    end_month: &str,
    max_months: usize,
) -> Result<Vec<(String, String)>> {
    let start = parse_month(start_month)?;
    let end = parse_month(end_month)?;
    if start > end {
        bail!("start month {start_month} is after end month {end_month}");
    }

    let mut chunks = Vec::new();
    let mut cursor = start;
    while cursor <= end {
        let mut chunk_end = add_months(cursor, max_months as i32 - 1)?;
        if chunk_end > end {
            chunk_end = end;
        }
        chunks.push((format_month(cursor), format_month(chunk_end)));
        cursor = add_months(chunk_end, 1)?;
    }
    Ok(chunks)
}

fn parse_month(value: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(&format!("{value}-01"), "%Y-%m-%d").context("parse YYYY-MM month")
}

fn add_months(date: NaiveDate, months: i32) -> Result<NaiveDate> {
    let year = date.year();
    let month0 = date.month0() as i32;
    let total = year * 12 + month0 + months;
    let new_year = total.div_euclid(12);
    let new_month0 = total.rem_euclid(12) as u32;
    NaiveDate::from_ymd_opt(new_year, new_month0 + 1, 1)
        .ok_or_else(|| anyhow!("invalid month arithmetic result"))
}

fn format_month(value: NaiveDate) -> String {
    value.format("%Y-%m").to_string()
}

fn dedupe_generation(records: Vec<GenerationFetchRecord>) -> Vec<GenerationFetchRecord> {
    let mut seen = BTreeMap::new();
    for record in records {
        seen.insert(record.record.month.clone(), record);
    }
    seen.into_values().collect()
}

fn dedupe_rec(records: Vec<BridgeRecRecord>) -> Vec<BridgeRecRecord> {
    let mut seen = BTreeMap::new();
    for record in records {
        seen.insert(record.application_no.clone(), record);
    }
    seen.into_values().collect()
}

fn normalize_digits(value: &str) -> String {
    value.chars().filter(|ch| ch.is_ascii_digit()).collect()
}

fn normalize_date(value: &str) -> String {
    let digits = normalize_digits(value);
    if digits.len() != 8 {
        value.trim().to_owned()
    } else {
        format!("{}-{}-{}", &digits[..4], &digits[4..6], &digits[6..8])
    }
}

fn normalize_month(value: &str) -> String {
    let digits = normalize_digits(value);
    if digits.len() != 6 {
        value.trim().to_owned()
    } else {
        format!("{}-{}", &digits[..4], &digits[4..6])
    }
}

fn string_value(row: &Map<String, Value>, keys: &[&str]) -> String {
    keys.iter()
        .filter_map(|key| row.get(*key))
        .find_map(|value| match value {
            Value::String(text) => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_owned())
                }
            }
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        })
        .unwrap_or_default()
}

fn float_value(row: &Map<String, Value>, keys: &[&str]) -> f64 {
    keys.iter()
        .filter_map(|key| row.get(*key))
        .find_map(|value| match value {
            Value::Number(number) => number.as_f64(),
            Value::String(text) => text.trim().replace(',', "").parse::<f64>().ok(),
            _ => None,
        })
        .unwrap_or(0.0)
}

fn float_option(row: &Map<String, Value>, keys: &[&str]) -> Option<f64> {
    let value = float_value(row, keys);
    if value == 0.0 { None } else { Some(value) }
}

fn int_value(row: &Map<String, Value>, keys: &[&str]) -> i64 {
    float_value(row, keys) as i64
}

fn float_ptr_from_value(value: Option<&Value>) -> Option<f64> {
    match value {
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::String(text)) => text.trim().replace(',', "").parse::<f64>().ok(),
        _ => None,
    }
}

fn map_app_sec(app_sec: &str, app_name: &str) -> String {
    if !app_sec.trim().is_empty() {
        return app_sec.trim().to_owned();
    }
    match app_name.trim() {
        "신규" => "A".to_owned(),
        "변경" => "B".to_owned(),
        "검토서" => "C".to_owned(),
        "말소" => "D".to_owned(),
        _ => String::new(),
    }
}

fn first_non_empty(values: &[String]) -> String {
    values
        .iter()
        .find(|value| !value.trim().is_empty())
        .cloned()
        .unwrap_or_default()
}

fn first_non_zero(values: &[f64]) -> f64 {
    values
        .iter()
        .copied()
        .find(|value| *value != 0.0)
        .unwrap_or(0.0)
}

fn join_non_empty(values: &[String]) -> String {
    values
        .iter()
        .filter_map(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn string_option(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn headers(pairs: &[(&str, &str)]) -> Vec<String> {
    pairs
        .iter()
        .map(|(name, value)| format!("{name}: {value}"))
        .collect()
}

fn write_downloaded_file(
    root: &Path,
    application_no: &str,
    file_name: &str,
    content: &[u8],
) -> Result<String> {
    std::fs::create_dir_all(root).with_context(|| format!("create {}", root.display()))?;
    let root =
        std::fs::canonicalize(root).with_context(|| format!("resolve {}", root.display()))?;
    let dir = root.join(safe_download_segment("applicationNo", application_no)?);
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let dir = std::fs::canonicalize(&dir).with_context(|| format!("resolve {}", dir.display()))?;
    if !dir.starts_with(&root) {
        bail!("download directory escaped collector workspace");
    }

    let target = unique_path(dir.join(safe_download_segment("fileName", file_name)?));
    std::fs::write(&target, content)
        .with_context(|| format!("write downloaded file {}", target.display()))?;
    Ok(target.to_string_lossy().into_owned())
}

fn safe_download_segment(field_name: &str, value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("{field_name} must not be empty");
    }
    if trimmed != "." && trimmed != ".." && trimmed.bytes().all(is_safe_path_segment_byte) {
        return Ok(trimmed.to_owned());
    }

    let mut hasher = Sha256::new();
    hasher.update(trimmed.as_bytes());
    Ok(format!(
        "{field_name}-{}",
        hex::encode(&hasher.finalize()[..8])
    ))
}

fn is_safe_path_segment_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_')
}

fn unique_path(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }

    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("file");
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    for idx in 2.. {
        let candidate = if extension.is_empty() {
            parent.join(format!("{stem}-{idx}"))
        } else {
            parent.join(format!("{stem}-{idx}.{extension}"))
        };
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("unique path search must terminate")
}

fn current_month_seoul() -> String {
    Utc::now().with_timezone(&Seoul).format("%Y-%m").to_string()
}

#[derive(Debug, Clone)]
struct InputField {
    name: String,
    value: String,
}

fn input_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r#"(?is)<input[^>]*id="([^"]+)"[^>]*name="([^"]*)"[^>]*value="(.*?)"[^>]*>"#)
            .expect("valid input regex")
    })
}

fn ente_id_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r#"obj\["P_ENTE_ID"\]\s*=\s*'([^']+)'"#).expect("valid ente id regex")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_kcert_login_page_extracts_required_fields() {
        let html = r#"
            <form id="otherFrm">
                <input type="hidden" id="rtnPage" name="rtnPage" value="https://rps.energy.or.kr/loginProc2_main.do" />
                <input type="hidden" id="id1" name="8UDOKH4EGtc=" value="" />
                <input type="hidden" id="id2" name="uE6yNPh47vU=" value="" />
            </form>
            <form id="submit_form">
                <input type="hidden" id="LoginID" name="LoginID" value="kea0729" />
                <input type="hidden" id="SysCode" name="SysCode" value="NRPS" />
                <input type="hidden" id="userSsnType" name="userSsnType" value="1" />
            </form>
            <input type="hidden" id="ServerKmCert" name="ServerKmCert" value="-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----" />
        "#;

        let parsed = parse_kcert_login_page(html).expect("parse login page");
        assert_eq!(parsed.login_id, "kea0729");
        assert_eq!(parsed.sys_code, "NRPS");
        assert_eq!(
            parsed.return_page,
            "https://rps.energy.or.kr/loginProc2_main.do"
        );
        assert_eq!(parsed.id1_field_name, "8UDOKH4EGtc=");
        assert_eq!(parsed.id2_field_name, "uE6yNPh47vU=");
        assert!(parsed.server_km_cert_pem.contains("BEGIN CERTIFICATE"));
    }

    #[test]
    fn parse_kcert_result_splits_delimited_payload() {
        let parsed =
            parse_kcert_result("1::성공::token1::token2::커널로그에너지::20200101::ci-value")
                .expect("parse result");
        assert_eq!(parsed.code, "1");
        assert_eq!(parsed.id1, "token1");
        assert_eq!(parsed.id2, "token2");
    }

    #[test]
    fn extract_ente_id_reads_main_page_script() {
        let ente_id = extract_ente_id(r#"obj["P_ENTE_ID"] = '175858';"#).expect("extract ente id");
        assert_eq!(ente_id, "175858");
    }

    #[test]
    fn write_downloaded_file_keeps_unsafe_segments_under_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("downloads");
        let source_path = write_downloaded_file(&root, "../outside", "../../permit.pdf", b"PDF")
            .expect("write downloaded file");
        let source_path = std::fs::canonicalize(source_path).expect("canonical source path");
        let root = std::fs::canonicalize(root).expect("canonical root");

        assert!(source_path.starts_with(&root));
        assert_eq!(
            std::fs::read(source_path).expect("read downloaded file"),
            b"PDF"
        );
        assert!(!dir.path().join("outside").exists());
    }

    #[test]
    fn safe_download_segment_preserves_simple_ascii_segments() {
        assert_eq!(
            safe_download_segment("facilityCode", "FAC-001_2.3").expect("safe segment"),
            "FAC-001_2.3"
        );
    }

    #[test]
    fn resolve_range_uses_earliest_plant_history_month() {
        let facility = FacilityRecord {
            ente_id: String::new(),
            prvs_id: String::new(),
            prvs_app_no: String::new(),
            code: "PV15-R-090795".to_owned(),
            name: String::new(),
            business_no: String::new(),
            contract_no: String::new(),
            address: String::new(),
            installed_kw: 0.0,
            rec_start_date: "2024-11-12".to_owned(),
            confirm_date: "2024-10-01".to_owned(),
        };
        let completions = vec![
            CompletionRecord {
                application_no: String::new(),
                facility_code: "OTHER".to_owned(),
                submitted_at: String::new(),
                first_applied: "2024-01-01".to_owned(),
                app_sec: String::new(),
            },
            CompletionRecord {
                application_no: String::new(),
                facility_code: "PV15-R-090795".to_owned(),
                submitted_at: "2024-09-11".to_owned(),
                first_applied: String::new(),
                app_sec: String::new(),
            },
            CompletionRecord {
                application_no: String::new(),
                facility_code: "PV15-R-090795".to_owned(),
                submitted_at: String::new(),
                first_applied: "2024-08-20".to_owned(),
                app_sec: String::new(),
            },
        ];

        let (start, end) =
            resolve_range(None, Some("2026-03"), &facility, &completions).expect("resolve range");
        assert_eq!(start, "2024-08");
        assert_eq!(end, "2026-03");
    }
}
