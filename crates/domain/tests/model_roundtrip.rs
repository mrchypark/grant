use domain::{
    Action, ActionRiskLevel, Agreement, AgreementStatus, AgreementType, DocumentArtifact,
    DocumentCategory, GenerationRecord, Group, MembershipRole, MembershipStatus, Organization,
    OrganizationMembership, Plant, RateRecord, RecRecord, Role, RoleBinding,
    RoleBindingResourceType, RoleBindingState, RoleBindingSubjectType, RoleRule, RoleRuleEffect,
    SyncMode, SyncRun, User,
};

#[test]
fn serializes_core_models() {
    let user = User {
        id: "conalog-account".into(),
        email: Some("ops@example.com".into()),
        display_name: "Conalog Account".into(),
        active: true,
    };
    let organization = Organization {
        id: "2768603291".into(),
        business_no: Some("2768603291".into()),
        name: "커널로그에너지".into(),
        inactivated_at: None,
    };
    let membership = OrganizationMembership {
        id: "org_mem_1".into(),
        organization_id: organization.id.clone(),
        user_id: user.id.clone(),
        membership_role: MembershipRole::Owner,
        status: MembershipStatus::Active,
    };
    let group = Group {
        id: "group_1".into(),
        organization_id: organization.id.clone(),
        name: "현장팀".into(),
        description: Some("현장 운영 그룹".into()),
    };
    let agreement = Agreement {
        id: "agr_1".into(),
        agreement_type: AgreementType::Om,
        grantor_organization_id: organization.id.clone(),
        grantee_organization_id: "org_2".into(),
        status: AgreementStatus::Active,
        title: "운영 위임 계약".into(),
        reference_no: Some("AG-2026-001".into()),
        starts_at: Some("2026-01-01T00:00:00Z".into()),
        ends_at: Some("2027-01-01T00:00:00Z".into()),
    };
    let action = Action {
        id: "action_1".into(),
        code: "plant.profile.read".into(),
        name: "Plant Profile Read".into(),
        description: Some("Read plant profile".into()),
        risk_level: ActionRiskLevel::Low,
        visibility_granting: true,
    };
    let role = Role {
        id: "role_1".into(),
        organization_id: Some(organization.id.clone()),
        code: "viewer".into(),
        name: "Viewer".into(),
        description: Some("Read-only access".into()),
    };
    let role_rule = RoleRule {
        id: "rule_1".into(),
        role_id: role.id.clone(),
        effect: RoleRuleEffect::Allow,
        action_pattern: "plant.*.read".into(),
    };
    let binding = RoleBinding {
        id: "binding_1".into(),
        subject_type: RoleBindingSubjectType::Group,
        subject_id: group.id.clone(),
        subject_organization_id: organization.id.clone(),
        resource_type: RoleBindingResourceType::Plant,
        resource_id: "plant_1".into(),
        role_id: role.id.clone(),
        agreement_id: Some(agreement.id.clone()),
        state: RoleBindingState::Active,
        created_by_user_id: user.id.clone(),
        created_at: "2026-03-31T00:00:00Z".into(),
        updated_at: "2026-03-31T00:00:00Z".into(),
    };
    let plant = Plant {
        id: "plant_1".into(),
        facility_code: "PV15-R-090795".into(),
        name: "커널로그1호 태양광발전소".into(),
        owner_organization_id: organization.id.clone(),
        created_by_organization_id: Some("org_2".into()),
        contract_no: "5001058115".into(),
        address: "부산광역시 기장군 장안읍 명례리 907-4".into(),
        installed_kw: 221.615,
        rec_start_date: "2024-09-05".into(),
        confirm_issued: "2024-11-12".into(),
    };
    let sync = SyncRun {
        id: "run_1".into(),
        organization_id: organization.id.clone(),
        plant_id: Some(plant.id.clone()),
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
        "organization": organization,
        "membership": membership,
        "group": group,
        "agreement": agreement,
        "action": action,
        "role": role,
        "roleRule": role_rule,
        "binding": binding,
        "plant": plant,
        "sync": sync,
        "generation": generation,
        "rate": rate,
        "rec": rec,
        "document": document,
    });

    assert_eq!(value["organization"]["businessNo"], "2768603291");
    assert_eq!(value["membership"]["membershipRole"], "owner");
    assert_eq!(value["group"]["name"], "현장팀");
    assert_eq!(value["agreement"]["agreementType"], "om");
    assert_eq!(value["action"]["code"], "plant.profile.read");
    assert_eq!(value["role"]["code"], "viewer");
    assert_eq!(value["roleRule"]["actionPattern"], "plant.*.read");
    assert_eq!(value["binding"]["resourceType"], "plant");
    assert_eq!(value["binding"]["resourceId"], "plant_1");
    assert_eq!(value["sync"]["mode"], "delta");
    assert_eq!(value["sync"]["organizationId"], "2768603291");
    assert_eq!(value["document"]["category"], "drawings");
    assert_eq!(value["plant"]["id"], "plant_1");
    assert_eq!(value["plant"]["facilityCode"], "PV15-R-090795");
    assert_eq!(value["plant"]["ownerOrganizationId"], "2768603291");
    assert_eq!(value["plant"]["createdByOrganizationId"], "org_2");
}
