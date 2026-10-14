package adminui

import (
	"encoding/json"
	"fmt"
	"net/url"
	"strconv"
	"strings"
)

type resource struct {
	path        string
	title       string
	description string
	columns     []string
	jsonFields  []string
}

var resources = map[string]resource{
	"":              {"/api/v1/providers", "개요", "Grant 관리 항목을 확인하고 필요한 작업을 시작합니다.", []string{"ID", "이름", "활성"}, []string{"id", "name", "active"}},
	"providers":     {"/api/v1/providers", "프로바이더", "Grant에 등록된 서비스 정의를 관리합니다.", []string{"ID", "이름", "설명", "활성"}, []string{"id", "name", "description", "active"}},
	"actions":       {"/api/v1/admin/providers/{providerId}/actions", "액션", "프로바이더의 액션 코드와 위험도, 앱 권한 부여 설정을 관리합니다.", []string{"ID", "코드", "이름", "위험 수준", "가시성 부여", "앱에 부여 가능"}, []string{"id", "code", "name", "riskLevel", "visibilityGranting", "grantableToApplications"}},
	"http":          {"/api/v1/admin/providers/{providerId}/http-capability-active", "HTTP 기능", "현재 활성화된 HTTP 기능 카탈로그를 확인합니다. 카탈로그 버전은 별도로 등록하고 활성화합니다.", []string{"카탈로그 세대", "카탈로그 다이제스트", "기능 수"}, []string{"generation", "catalogDigest", "capabilityCount"}},
	"users":         {"/api/v1/admin/users", "Grant 사용자", "Grant 사용자 목록과 활성 상태를 관리합니다. 로그인 계정과 인증 정보는 GoAuthy에서 관리합니다.", []string{"ID", "이메일", "표시 이름", "활성"}, []string{"id", "email", "displayName", "active"}},
	"organizations": {"/api/v1/orgs", "조직", "인증된 운영자가 조회할 수 있는 조직을 관리합니다.", []string{"ID", "이름", "당사자 ID", "비활성화 시각"}, []string{"id", "name", "partyId", "inactivatedAt"}},
	"members":       {"/api/v1/orgs/{organizationId}/members", "구성원", "조직 구성원과 멤버십 역할을 관리합니다.", []string{"사용자 ID", "멤버십 역할", "상태"}, []string{"userId", "membershipRole", "status"}},
	"groups":        {"/api/v1/orgs/{organizationId}/groups", "그룹", "조직 그룹과 그룹 구성원을 관리합니다.", []string{"ID", "이름", "설명"}, []string{"id", "name", "description"}},
	"agreements":    {"/api/v1/orgs/{organizationId}/agreements", "계약", "조직 간 계약을 조회하고 상태를 변경합니다. 상태를 바꾸어도 계약 기록은 유지됩니다.", []string{"ID", "제목", "계약 유형", "상태", "권한 제공 조직 ID", "권한 수신 조직 ID"}, []string{"id", "title", "agreementType", "status", "grantorOrganizationId", "granteeOrganizationId"}},
	"roles":         {"/api/v1/orgs/{organizationId}/roles", "역할", "역할에 연결된 액션 권한을 관리합니다.", []string{"ID", "코드", "이름", "설명"}, []string{"id", "code", "name", "description"}},
	"bindings":      {"/api/v1/orgs/{organizationId}/bindings", "역할 바인딩", "주체와 리소스를 역할에 연결하는 권한 바인딩을 관리합니다.", []string{"ID", "주체 유형", "주체 ID", "리소스 유형", "리소스 ID", "역할 ID", "상태"}, []string{"id", "subjectType", "subjectId", "resourceType", "resourceId", "roleId", "state"}},
	"applications":  {"/api/v1/orgs/{organizationId}/apps", "애플리케이션", "조직 애플리케이션과 권한 부여, 신뢰, ID 연동, 동의 설정을 관리합니다.", []string{"ID", "슬러그", "이름", "애플리케이션 유형", "활성", "프로비저닝 상태"}, []string{"id", "slug", "name", "applicationType", "active", "provisioningStatus"}},
}

func pageKey(path string) (string, bool) {
	if path == "/" {
		return "", true
	}
	key := strings.Trim(path, "/")
	_, ok := resources[key]
	return key, ok
}

func rowsFromJSON(data []byte, key string) ([]Row, error) {
	var raw any
	if err := json.Unmarshal(data, &raw); err != nil {
		return nil, fmt.Errorf("invalid Grant API response")
	}
	var items []any
	switch v := raw.(type) {
	case []any:
		items = v
	case map[string]any:
		if value, ok := v["Value"]; ok {
			switch nested := value.(type) {
			case []any:
				items = nested
			case map[string]any:
				if list, ok := nested["actions"].([]any); ok {
					items = list
				} else {
					items = []any{nested}
				}
			}
		}
		if items != nil {
			break
		}
		for _, candidate := range []string{"items", "actions", "members", "applications", "groups", "agreements", "roles", "bindings"} {
			if list, ok := v[candidate].([]any); ok {
				items = list
				break
			}
		}
		if items == nil {
			items = []any{v}
		}
	default:
		return nil, fmt.Errorf("unexpected Grant API response")
	}
	fields := resources[key].jsonFields
	rows := make([]Row, 0, len(items))
	for _, item := range items {
		obj, ok := item.(map[string]any)
		if !ok {
			continue
		}
		row := Row{Cells: make([]string, len(fields))}
		for i, field := range fields {
			row.Cells[i] = displayValue(obj[field])
		}
		if id := rowID(key, obj); id != "" {
			row.Link = "?id=" + url.QueryEscape(id)
		}
		rows = append(rows, row)
	}
	return rows, nil
}

func rowID(key string, row map[string]any) string {
	field := "id"
	if key == "members" {
		field = "userId"
	}
	id, _ := row[field].(string)
	return id
}

func displayValue(v any) string {
	switch x := v.(type) {
	case nil:
		return "—"
	case string:
		if x == "" {
			return "—"
		}
		return x
	case bool:
		return strconv.FormatBool(x)
	case float64:
		return strconv.FormatFloat(x, 'f', -1, 64)
	default:
		b, _ := json.Marshal(x)
		return string(b)
	}
}

func selectedQuery(q url.Values) string { return q.Get("q") }
