use domain::{
    Business, BusinessMembership, BusinessRole, DocumentArtifact, DocumentCategory, GenerationRecord,
    LegalEntity, Plant, RateRecord, RecRecord, SyncMode, SyncRun, User,
};

#[test]
fn serializes_core_models() {
    let user = User {
        id: "conalog-account".into(),
        email: Some("ops@example.com".into()),
        display_name: "Conalog Account".into(),
        active: true,
    };
    let business = Business {
        id: "2768603291".into(),
        display_name: "커널로그에너지".into(),
        business_no: "2768603291".into(),
        legal_entity_id: Some("legal_1".into()),
        active: true,
    };
    let membership = BusinessMembership {
        user_id: user.id.clone(),
        business_id: business.id.clone(),
        role: BusinessRole::Owner,
    };
    let legal = LegalEntity {
        id: "legal_1".into(),
        name: "주식회사 커널로그에너지".into(),
        representative_name: Some("오복성".into()),
    };
    let plant = Plant {
        code: "PV15-R-090795".into(),
        name: "커널로그1호 태양광발전소".into(),
        business_no: "2768603291".into(),
        contract_no: "5001058115".into(),
        address: "부산광역시 기장군 장안읍 명례리 907-4".into(),
        installed_kw: 221.615,
        rec_start_date: "2024-09-05".into(),
        confirm_issued: "2024-11-12".into(),
    };
    let sync = SyncRun {
        id: "run_1".into(),
        business_no: "2768603291".into(),
        plant_id: Some(plant.code.clone()),
        mode: SyncMode::Delta,
        status: "completed".into(),
        started_at: Some("2026-03-31T00:00:00Z".into()),
        finished_at: Some("2026-03-31T00:00:01Z".into()),
        duration_ms: 1000,
    };
    let generation = GenerationRecord {
        month: "2026-02".into(),
        generation_kwh: 21036.0,
        self_consumption_kwh: None,
        supply_kwh: 21036.0,
        application_deadline: Some("2026-05-29".into()),
    };
    let rate = RateRecord {
        month: "2026-01".into(),
        self_rate: Some(14.1),
        national_average: Some(12.1),
        nearby_average: Some(13.7),
        nearby_highest: Some(40.4),
    };
    let rec = RecRecord {
        month: "2026-02".into(),
        application_no: "12154225".into(),
        supply_kwh: 21036,
        rec_quantity: 31,
        fee_krw: 1705,
        issued_date: "2026-03-25".into(),
        expiry_date: "2029-03-25".into(),
        virtual_account: Some("48102174397708".into()),
        bank_name: Some("기업은행".into()),
        status: Some("발급완료".into()),
    };
    let document = DocumentArtifact {
        id: "doc_1".into(),
        category: DocumentCategory::Drawings,
        file_class: "설비구성".into(),
        file_sec_name: "설치도면".into(),
        file_name: "설치도면.pdf".into(),
        required: true,
        source_sha256: Some("sha".into()),
        output_sha256: Some("sha".into()),
        download_url: Some("/api/v1/documents/doc_1/download".into()),
    };

    let value = serde_json::json!({
        "user": user,
        "business": business,
        "membership": membership,
        "legalEntity": legal,
        "plant": plant,
        "sync": sync,
        "generation": generation,
        "rate": rate,
        "rec": rec,
        "document": document,
    });

    assert_eq!(value["business"]["businessNo"], "2768603291");
    assert_eq!(value["membership"]["role"], "owner");
    assert_eq!(value["sync"]["mode"], "delta");
    assert_eq!(value["document"]["category"], "drawings");
}
