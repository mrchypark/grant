package adminui

import "strings"

const (
	opProviderCreate           = "provider.create"
	opProviderUpdate           = "provider.update"
	opActionCreate             = "action.create"
	opActionUpdate             = "action.update"
	opActionDelete             = "action.delete"
	opHTTPStage                = "http.stage"
	opHTTPActivate             = "http.activate"
	opUserActive               = "user.active"
	opOrganizationCreate       = "organization.create"
	opOrganizationUpdate       = "organization.update"
	opOrganizationDeactivate   = "organization.deactivate"
	opOrganizationReactivate   = "organization.reactivate"
	opOrganizationOwner        = "organization.owner"
	opMemberEnroll             = "member.enroll"
	opMemberRemove             = "member.remove"
	opGroupCreate              = "group.create"
	opGroupUpdate              = "group.update"
	opGroupDelete              = "group.delete"
	opGroupMemberAdd           = "group.member.add"
	opGroupMemberRemove        = "group.member.remove"
	opAgreementCreate          = "agreement.create"
	opAgreementUpdate          = "agreement.update"
	opRoleCreate               = "role.create"
	opRoleUpdate               = "role.update"
	opRoleDelete               = "role.delete"
	opRoleActions              = "role.actions"
	opBindingCreate            = "binding.create"
	opBindingDelete            = "binding.delete"
	opApplicationCreate        = "application.create"
	opApplicationUpdate        = "application.update"
	opApplicationDelete        = "application.delete"
	opApplicationActive        = "application.active"
	opApplicationActions       = "application.actions"
	opApplicationTrust         = "application.trust"
	opApplicationIdentity      = "application.identity"
	opApplicationConsent       = "application.consent"
	opApplicationConsentRevoke = "application.consent.revoke"
	opGrantRevoke              = "application.grant.revoke"
)

func options(values ...string) []Option {
	labels := map[string]string{
		"true": "예", "false": "아니요",
		"low": "낮음", "medium": "보통", "high": "높음", "critical": "치명적",
		"deactivate": "비활성화", "reactivate": "재활성화",
		"delegation": "위임", "epc": "EPC", "om": "O&M", "distributor": "유통사", "dealer": "판매사",
		"draft": "초안", "active": "활성", "expired": "만료", "terminated": "종료",
		"user": "사용자", "group": "그룹", "organization": "조직",
		"disabled": "비활성", "public": "공개", "confidential": "기밀",
		"web": "웹", "mobile": "모바일", "server": "서버", "internal": "내부",
	}
	out := make([]Option, 0, len(values))
	for _, v := range values {
		label, ok := labels[v]
		if !ok {
			label = strings.ReplaceAll(v, "_", " ")
		}
		out = append(out, Option{Value: v, Label: label})
	}
	return out
}

func field(name, label, typ, help string, required bool, opts ...Option) Field {
	return Field{Name: name, Label: label, Type: typ, Help: help, Required: required, Options: opts}
}

func form(id, title, desc, action string, fields ...Field) Form {
	return Form{ID: id, Title: title, Description: desc, Action: "/operations", SubmitLabel: title, Fields: fields, Hidden: map[string]string{"operation": action}}
}

func confirm(f Form, message string) Form { f.Confirmation = message; f.Danger = true; return f }

func formsFor(key string) []Form {
	text := func(n, l string, req bool) Field { return field(n, l, "text", "", req) }
	area := func(n, l string) Field { return field(n, l, "textarea", "", false) }
	selectField := func(n, l string, req bool, vals ...string) Field {
		return field(n, l, "select", "", req, options(vals...)...)
	}
	switch key {
	case "providers":
		return []Form{
			form("provider-create", "프로바이더 등록", "프로바이더 ID는 변경할 수 없습니다. 계속 사용할 안정적인 ID를 선택하세요.", opProviderCreate, text("id", "프로바이더 ID", true), text("name", "이름", true), area("description", "설명"), selectField("active", "활성 상태", true, "true", "false")),
			form("provider-update", "프로바이더 수정", "입력한 프로바이더 항목만 변경합니다.", opProviderUpdate, text("id", "프로바이더 ID", true), text("name", "이름", false), area("description", "설명"), selectField("active", "활성 상태", false, "true", "false")),
		}
	case "actions":
		return []Form{
			form("action-create", "액션 등록", "위험도와 권한 부여 설정은 이후 권한 확인과 사용자 동의에 영향을 줍니다.", opActionCreate, text("id", "액션 ID", true), text("code", "코드", true), text("name", "이름", true), area("description", "설명"), selectField("riskLevel", "위험도", true, "low", "medium", "high", "critical"), selectField("visibilityGranting", "리소스 표시 권한 부여", true, "true", "false"), selectField("grantableToApplications", "앱에 권한 부여 허용", true, "true", "false"), text("consentLabel", "동의 화면 레이블", false), area("consentDescription", "동의 안내")),
			form("action-update", "액션 수정", "선택 항목을 비워 두면 현재 값을 유지합니다.", opActionUpdate, text("id", "액션 ID", true), text("name", "이름", false), area("description", "설명"), selectField("riskLevel", "위험도", false, "low", "medium", "high", "critical"), selectField("visibilityGranting", "리소스 표시 권한 부여", false, "true", "false"), selectField("grantableToApplications", "앱에 권한 부여 허용", false, "true", "false"), text("consentLabel", "동의 화면 레이블", false), area("consentDescription", "동의 안내")),
			form("action-delete", "액션 삭제", "활성 카탈로그에서 사용 중인 액션은 삭제가 거부될 수 있습니다.", opActionDelete, text("id", "액션 ID", true)),
		}
	case "http":
		return []Form{
			form("http-stage", "세대 등록", "StageHttpCapabilityGeneration JSON 전체를 입력합니다. 등록만으로 활성화되지는 않습니다.", opHTTPStage, field("generation", "세대 JSON", "textarea", "schemaVersion, generation, expectedLatestGeneration, capabilities 항목을 포함하세요.", true)),
			form("http-activate", "등록한 세대 활성화", "현재 세대와 다이제스트를 비교해 변경 여부를 확인한 뒤 활성화합니다.", opHTTPActivate, text("expectedCatalogDigest", "현재 활성 카탈로그 다이제스트", false), text("expectedGeneration", "현재 세대 (없으면 비워 둠)", false), text("targetGeneration", "등록한 세대", true), text("targetCatalogDigest", "등록한 카탈로그 다이제스트", true)),
		}
	case "users":
		return []Form{form("user-active", "Grant 사용자 접근 권한 변경", "Grant 사용자 레코드의 접근 권한만 변경합니다. 로그인 계정과 인증 정보는 GoAuthy에서 관리합니다.", opUserActive, text("id", "Grant 사용자 ID", true), selectField("active", "Grant 접근 허용", true, "true", "false"))}
	case "organizations":
		return []Form{
			form("organization-create", "조직 생성", "소유자는 이미 활성 상태인 Grant 사용자여야 합니다.", opOrganizationCreate, text("id", "조직 ID", true), text("name", "이름", true), text("ownerUserId", "소유자 Grant 사용자 ID", true)),
			form("organization-update", "조직 수정", "", opOrganizationUpdate, text("id", "조직 ID", true), text("name", "이름", true)),
			form("organization-lifecycle", "조직 상태 변경", "조직 비활성화와 재활성화는 선택한 조직에 적용됩니다.", opOrganizationDeactivate, text("id", "조직 ID", true), selectField("status", "변경할 상태", true, "deactivate", "reactivate")),
			form("organization-owner", "조직 소유자 변경", "조직을 선택하면 현재 소유자와 버전을 불러옵니다.", opOrganizationOwner, text("id", "조직 ID", true), text("userId", "새 소유자 Grant 사용자 ID", true)),
		}
	case "members":
		return []Form{
			form("member-enroll", "구성원 등록", "선택한 조직에 기존 Grant 사용자를 등록합니다.", opMemberEnroll, text("userId", "Grant 사용자 ID", true)),
			form("member-remove", "구성원 제거", "선택한 조직에서 사용자의 구성원 권한을 제거합니다.", opMemberRemove, text("userId", "Grant 사용자 ID", true)),
		}
	case "groups":
		return []Form{
			form("group-create", "그룹 생성", "", opGroupCreate, text("id", "그룹 ID", true), text("name", "이름", true), area("description", "설명")),
			form("group-update", "그룹 수정", "선택한 그룹의 이름과 설명을 불러옵니다. 설명을 비우면 저장된 설명을 지웁니다.", opGroupUpdate, text("id", "그룹 ID", true), text("name", "이름", true), area("description", "설명")),
			form("group-delete", "그룹 삭제", "이 조직 그룹을 삭제합니다.", opGroupDelete, text("id", "그룹 ID", true)),
			form("group-member-add", "그룹 구성원 추가", "사용자는 먼저 조직의 활성 구성원이어야 합니다.", opGroupMemberAdd, text("id", "그룹 ID", true), text("userId", "Grant 사용자 ID", true)),
			form("group-member-remove", "그룹 구성원 제거", "", opGroupMemberRemove, text("id", "그룹 ID", true), text("userId", "Grant 사용자 ID", true)),
		}
	case "agreements":
		return []Form{
			form("agreement-create", "계약 생성", "계약 기록은 보존됩니다. 적용할 상태를 명시적으로 선택하세요.", opAgreementCreate, text("id", "계약 ID", true), text("title", "계약명", true), selectField("agreementType", "계약 유형", true, "delegation", "epc", "om", "distributor", "dealer"), selectField("status", "상태", true, "draft", "active", "expired", "terminated"), text("grantorOrganizationId", "권한 제공 조직 ID", true), text("granteeOrganizationId", "권한 수신 조직 ID", true), text("referenceNo", "참조 번호", false), text("startsAt", "시작 시각 (RFC3339)", false), text("endsAt", "종료 시각 (RFC3339)", false)),
			form("agreement-update", "계약 수정", "상태를 변경해도 계약 기록과 이력은 보존됩니다.", opAgreementUpdate, text("id", "계약 ID", true), text("title", "계약명", false), selectField("agreementType", "계약 유형", false, "delegation", "epc", "om", "distributor", "dealer"), selectField("status", "상태", false, "draft", "active", "expired", "terminated")),
		}
	case "roles":
		return []Form{form("role-create", "역할 생성", "", opRoleCreate, text("id", "역할 ID", true), text("code", "코드", true), text("name", "이름", true), area("description", "설명")), form("role-update", "역할 수정", "입력한 항목만 변경합니다.", opRoleUpdate, text("id", "역할 ID", true), text("name", "이름", false), area("description", "설명")), confirm(form("role-delete", "역할 삭제", "역할 바인딩에서 사용 중이면 삭제가 거부될 수 있습니다.", opRoleDelete, text("id", "역할 ID", true)), "이 역할을 삭제할까요?"), confirm(form("role-actions", "역할 액션 목록 교체", "액션 ID를 쉼표로 구분해 입력합니다. 비워 두면 역할의 기존 액션 연결을 모두 제거합니다.", opRoleActions, text("id", "역할 ID", true), area("actionIds", "액션 ID 목록")), "액션 목록이 비어 있으면 이 역할의 모든 액션 연결이 제거됩니다. 계속할까요?")}
	case "bindings":
		return []Form{form("binding-create", "역할 바인딩 생성", "주체와 리소스 범위에 역할 권한을 연결합니다. API에서 주체, 리소스, 역할, 계약, 조직 범위를 확인합니다.", opBindingCreate, text("id", "역할 바인딩 ID", true), selectField("subjectType", "권한을 받을 주체 유형", true, "user", "group", "organization"), text("subjectId", "주체 ID", true), text("providerId", "프로바이더 ID", true), text("resourceType", "리소스 유형", true), text("resourceId", "리소스 ID", true), text("roleId", "역할 ID", true), text("agreementId", "계약 ID", false), selectField("state", "상태", true, "active", "disabled")), form("binding-delete", "역할 바인딩 삭제", "", opBindingDelete, text("id", "역할 바인딩 ID", true))}
	case "applications":
		return []Form{
			form("application-create", "앱 등록", "선택한 조직 범위에 앱을 등록합니다.", opApplicationCreate, text("id", "앱 ID", true), text("slug", "슬러그", true), text("name", "이름", true), area("description", "설명"), selectField("applicationType", "자격 증명 유형", true, "public", "confidential"), selectField("appType", "앱 유형", true, "web", "mobile", "server", "internal"), text("redirectUri", "리디렉션 URI", false), selectField("active", "활성 상태", true, "true", "false")),
			form("application-update", "앱 수정", "선택 항목을 비워 두면 현재 값을 유지합니다.", opApplicationUpdate, text("id", "앱 ID", true), text("name", "이름", false), area("description", "설명")),
			form("application-active", "앱 상태 변경", "", opApplicationActive, text("id", "앱 ID", true), selectField("active", "활성 상태", true, "true", "false")),
			form("application-delete", "앱 삭제", "관련 권한 부여나 역할 바인딩이 남아 있으면 삭제가 거부될 수 있습니다.", opApplicationDelete, text("id", "앱 ID", true)),
			confirm(form("application-actions", "앱 액션 목록 교체", "액션 ID를 쉼표로 구분해 입력합니다. 비워 두면 기존 액션 연결을 모두 제거합니다.", opApplicationActions, text("id", "앱 ID", true), area("actionIds", "액션 ID 목록")), "액션 목록이 비어 있으면 이 앱의 모든 액션 연결이 제거됩니다. 계속할까요?"),
			form("application-trust", "앱 신뢰 상태 변경", "조직 범위에 적용되는 플랫폼 작업입니다.", opApplicationTrust, text("id", "앱 ID", true), selectField("trusted", "신뢰 상태", true, "true", "false")),
			form("application-identity", "외부 계정 연결", "앱을 외부 클라이언트 계정에 연결합니다.", opApplicationIdentity, text("id", "앱 ID", true), text("clientId", "외부 클라이언트 ID", true)),
			confirm(form("application-consent", "사용자 동의 액션 목록 교체", "선택한 앱에서 현재 사용자의 동의와 버전을 불러옵니다. 액션 ID를 쉼표로 구분해 입력하고, 비워 두면 연결을 모두 제거합니다.", opApplicationConsent, text("id", "앱 ID", true), area("actionIds", "액션 ID 목록")), "액션 목록이 비어 있으면 현재 동의의 액션 연결이 모두 제거됩니다. 계속할까요?"),
			form("application-consent-revoke", "사용자 동의 철회", "현재 로그인한 운영자의 앱 동의를 철회합니다.", opApplicationConsentRevoke, text("id", "앱 ID", true)),
			form("grant-revoke", "앱 권한 부여 철회", "기존 권한 부여 기록을 철회합니다.", opGrantRevoke, text("id", "권한 부여 ID", true)),
		}
	default:
		return nil
	}
}
