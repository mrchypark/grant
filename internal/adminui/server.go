package adminui

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strconv"
	"strings"

	"github.com/Conalog/grant/internal/adminauth"
	"github.com/Conalog/grant/internal/grant"
)

type Config struct {
	APIURL                   string
	ManagementOrganizationID string
	HTTPClient               *http.Client
}

type server struct {
	cfg      Config
	api      *apiClient
	renderer *renderer
	mux      *http.ServeMux
}

// New builds the authenticated admin UI HTTP handler.
func New(cfg Config) (http.Handler, error) {
	if !validID(cfg.ManagementOrganizationID) {
		return nil, errors.New("management organization ID is required")
	}
	api, err := newAPIClient(cfg.APIURL, cfg.HTTPClient)
	if err != nil {
		return nil, err
	}
	renderer, err := newRenderer()
	if err != nil {
		return nil, err
	}
	s := &server{cfg: cfg, api: api, renderer: renderer, mux: http.NewServeMux()}
	s.mux.HandleFunc("GET /healthz", func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		_, _ = io.WriteString(w, `{"status":"ok"}`)
	})
	s.mux.HandleFunc("POST /operations", s.operation)
	for path := range resources {
		pattern := "/" + path
		if path == "" {
			pattern = "/"
		}
		s.mux.HandleFunc("GET "+pattern, s.view)
	}
	if err := bindAssets(s.mux); err != nil {
		return nil, err
	}
	return s.mux, nil
}

func (s *server) view(w http.ResponseWriter, r *http.Request) {
	key, ok := pageKey(r.URL.Path)
	if !ok {
		http.NotFound(w, r)
		return
	}
	org := r.URL.Query().Get("org")
	if org != "" && !validID(org) {
		http.Error(w, "invalid organization", http.StatusBadRequest)
		return
	}
	provider := r.URL.Query().Get("provider")
	if provider == "" {
		provider = "grant-management"
	}
	if !validID(provider) {
		http.Error(w, "invalid provider", http.StatusBadRequest)
		return
	}
	grantID := r.URL.Query().Get("grant")
	if grantID != "" && !validID(grantID) {
		http.Error(w, "invalid grant ID", http.StatusBadRequest)
		return
	}
	res := resources[key]
	orgOptions := s.orgOptions(r)
	if org == "" && strings.Contains(res.path, "{organizationId}") && len(orgOptions) > 0 {
		org = orgOptions[0].Value
	}
	if org == "" && strings.Contains(res.path, "{organizationId}") {
		s.renderError(w, r, key, org, provider, http.StatusBadRequest, "조직을 선택하세요.")
		return
	}
	vars := map[string]string{}
	if strings.Contains(res.path, "{organizationId}") {
		vars["organizationId"] = org
	}
	if strings.Contains(res.path, "{providerId}") {
		vars["providerId"] = provider
	}
	query := url.Values{}
	if key == "users" {
		query.Set("limit", "50")
		for _, name := range []string{"q", "cursor"} {
			if v := r.URL.Query().Get(name); v != "" {
				query.Set(name, v)
			}
		}
	}
	data, status, err := s.api.call(r.Context(), http.MethodGet, res.path, vars, query, nil, org, s.cfg.ManagementOrganizationID)
	noActiveCatalog := false
	if err != nil {
		if key == "http" && status == http.StatusNotFound && isNoActiveHTTPCatalog(data) {
			data, noActiveCatalog = []byte("[]"), true
		} else {
			s.renderError(w, r, key, org, provider, statusFor(status), safeAPIError(data, status))
			return
		}
	}
	rows, err := rowsFromJSON(data, key)
	if err != nil {
		s.renderError(w, r, key, org, provider, http.StatusBadGateway, err.Error())
		return
	}
	page := Page{Title: res.title, Description: res.description, Section: key, OrganizationID: org, ProviderID: provider, Search: r.URL.Query().Get("q"), CSRF: adminauth.CSRFToken(r.Context()), Columns: res.columns, Rows: rows, Forms: formsFor(key), Navigation: navigation(), Organizations: orgOptions, Providers: s.providerOptions(r)}
	if key == "http" {
		if noActiveCatalog {
			page.Notice = "이 프로바이더에는 아직 활성 HTTP 기능 세대가 없습니다. 새 세대를 생성하고 검토한 뒤 최초 활성화를 진행할 수 있습니다."
			page.Detail = "활성 카탈로그 없음"
		} else {
			setHTTPActivationCurrent(page.Forms, data)
			var pretty bytes.Buffer
			if json.Indent(&pretty, data, "", "  ") == nil {
				page.Detail = pretty.String()
			}
		}
		if generation := r.URL.Query().Get("generation"); generation != "" {
			if parsed, err := strconv.ParseUint(generation, 10, 64); err != nil || parsed == 0 {
				http.Error(w, "invalid generation", http.StatusBadRequest)
				return
			}
			staged, stagedStatus, stagedErr := s.api.call(r.Context(), http.MethodGet, "/api/v1/admin/providers/{providerId}/http-capability-generations/{generation}", map[string]string{"providerId": provider, "generation": generation}, nil, nil, "", s.cfg.ManagementOrganizationID)
			if stagedErr != nil {
				s.renderError(w, r, key, org, provider, statusFor(stagedStatus), safeAPIError(staged, stagedStatus))
				return
			}
			setHTTPActivationTarget(page.Forms, staged)
			active := json.RawMessage(page.Detail)
			if noActiveCatalog {
				active = nil
			}
			combined, _ := json.MarshalIndent(struct {
				ActiveCatalog json.RawMessage `json:"activeCatalog"`
				Generation    json.RawMessage `json:"generation"`
			}{active, staged}, "", "  ")
			page.Detail = string(combined)
			page.Notice = "생성된 HTTP 기능 세대를 다시 읽었습니다. 내용과 활성 버전 비교 조건을 검토한 뒤 별도로 활성화하세요."
		}
	}
	for i := range page.Forms {
		page.Forms[i].Hidden["section"] = key
		page.Forms[i].Hidden["org"] = org
		page.Forms[i].Hidden["provider"] = provider
	}
	for i := range page.Rows {
		if page.Rows[i].Link == "" {
			continue
		}
		link, _ := url.Parse(page.Rows[i].Link)
		q := link.Query()
		if key == "organizations" {
			q.Set("org", q.Get("id"))
		} else if org != "" {
			q.Set("org", org)
		}
		if provider != "" {
			q.Set("provider", provider)
		}
		page.Rows[i].Link = r.URL.Path + "?" + q.Encode()
	}
	selectedID := r.URL.Query().Get("id")
	if selectedID != "" {
		if !validID(selectedID) {
			http.Error(w, "invalid record ID", http.StatusBadRequest)
			return
		}
		found, inactive := false, false
		for _, row := range rows {
			rowURL, _ := url.Parse(row.Link)
			if rowURL.Query().Get("id") == selectedID {
				found = true
				inactiveAt := rowCellForJSONField(key, "inactivatedAt", row.Cells)
				inactive = key == "organizations" && inactiveAt != "" && inactiveAt != "—"
				break
			}
		}
		if !found {
			http.NotFound(w, r)
			return
		}
		prefillSelectedEntity(page.Forms, key, selectedID)
		if key == "groups" {
			for i := range page.Forms {
				if page.Forms[i].Hidden["operation"] == opGroupUpdate {
					page.Forms[i].Hidden["selectedID"] = selectedID
				}
			}
		}
		if inactive {
			page.Forms = removeForm(page.Forms, opOrganizationOwner)
			page.Notice = "조직이 비활성 상태입니다. 재활성화한 뒤 소유권을 관리할 수 있습니다."
		} else {
			detail, selectedErr := s.loadSelectedState(r, key, org, provider, selectedID, page.Forms)
			if selectedErr != nil {
				if key == "organizations" {
					page.Forms = removeForm(page.Forms, opOrganizationOwner)
					page.Error = selectedErr.message
				} else {
					s.renderError(w, r, key, org, provider, selectedErr.status, selectedErr.message)
					return
				}
			} else {
				page.Detail = detail
			}
		}
	}
	tables, relatedErr := s.loadRelated(r, key, org, provider, selectedID)
	if relatedErr != nil {
		page.Error = "연결된 정보를 불러오지 못했습니다: " + relatedErr.Error()
	} else {
		page.Tables = tables
		if grantID != "" {
			found := false
			for _, table := range tables {
				for _, row := range table.Rows {
					link, _ := url.Parse(row.Link)
					rowQuery := link.Query()
					if rowQuery.Get("grant") == grantID && rowQuery.Get("id") == selectedID {
						found = true
					}
				}
			}
			if !found {
				http.NotFound(w, r)
				return
			}
			setFormValue(page.Forms, opGrantRevoke, "id", grantID)
		}
	}
	if key == "users" {
		var response struct {
			NextCursor string `json:"nextCursor"`
		}
		if json.Unmarshal(data, &response) == nil && response.NextCursor != "" {
			q := url.Values{"cursor": []string{response.NextCursor}, "limit": []string{"50"}}
			if page.Search != "" {
				q.Set("q", page.Search)
			}
			page.NextURL = "/users?" + q.Encode()
		}
	}
	partial := strings.EqualFold(r.Header.Get("HX-Request"), "true")
	if err := s.renderer.render(w, page, partial); err != nil {
		http.Error(w, "unable to render admin page", http.StatusInternalServerError)
	}
}

func rowCellForJSONField(key, field string, cells []string) string {
	for index, jsonField := range resources[key].jsonFields {
		if jsonField == field && index < len(cells) {
			return cells[index]
		}
	}
	return ""
}

func removeForm(forms []Form, operation string) []Form {
	kept := forms[:0]
	for _, definition := range forms {
		if definition.Hidden["operation"] != operation {
			kept = append(kept, definition)
		}
	}
	return kept
}

func prefillSelectedEntity(forms []Form, section, id string) {
	for i := range forms {
		operation := forms[i].Hidden["operation"]
		fieldName := "id"
		if section == "members" {
			if operation != opMemberRemove {
				continue
			}
			fieldName = "userId"
		} else {
			switch operation {
			case opProviderUpdate, opActionUpdate, opActionDelete, opUserActive,
				opOrganizationUpdate, opOrganizationDeactivate, opOrganizationReactivate, opOrganizationOwner,
				opGroupUpdate, opGroupDelete, opGroupMemberAdd, opGroupMemberRemove,
				opAgreementUpdate, opRoleUpdate, opRoleDelete, opRoleActions, opBindingDelete,
				opApplicationUpdate, opApplicationDelete, opApplicationActive, opApplicationActions,
				opApplicationTrust, opApplicationIdentity, opApplicationConsent, opApplicationConsentRevoke:
			default:
				continue
			}
			if section == "applications" && operation == opGrantRevoke {
				continue
			}
		}
		for j := range forms[i].Fields {
			if forms[i].Fields[j].Name == fieldName {
				forms[i].Fields[j].Value = id
			}
		}
	}
}

func isNoActiveHTTPCatalog(body []byte) bool {
	var payload struct {
		Error string `json:"error"`
	}
	return json.Unmarshal(body, &payload) == nil && payload.Error == "HTTP capability catalog not active"
}

func setHTTPActivationCurrent(forms []Form, data []byte) {
	var current struct {
		Generation    uint64 `json:"generation"`
		CatalogDigest string `json:"catalogDigest"`
	}
	if json.Unmarshal(data, &current) != nil {
		return
	}
	setFormValue(forms, opHTTPActivate, "expectedGeneration", strconv.FormatUint(current.Generation, 10))
	setFormValue(forms, opHTTPActivate, "expectedCatalogDigest", current.CatalogDigest)
}

func setHTTPActivationTarget(forms []Form, data []byte) {
	var staged struct {
		Generation    uint64 `json:"generation"`
		CatalogDigest string `json:"catalogDigest"`
	}
	if json.Unmarshal(data, &staged) != nil {
		return
	}
	setFormValue(forms, opHTTPActivate, "targetGeneration", strconv.FormatUint(staged.Generation, 10))
	setFormValue(forms, opHTTPActivate, "targetCatalogDigest", staged.CatalogDigest)
}

type selectedReadError struct {
	status  int
	message string
}

func (s *server) loadSelectedState(r *http.Request, key, org, provider, id string, forms []Form) (string, *selectedReadError) {
	type snapshotRead struct {
		path, operation string
		vars            map[string]string
		allowMissing    bool
	}
	vars := map[string]string{"organizationId": org, "providerId": provider}
	reads := []snapshotRead{}
	if key == "organizations" {
		reads = append(reads, snapshotRead{path: "/api/v1/orgs/{organizationId}/owner", operation: opOrganizationOwner, vars: map[string]string{"organizationId": id}})
		org = id
	} else if key == "roles" {
		vars["roleId"] = id
		reads = append(reads, snapshotRead{path: "/api/v1/orgs/{organizationId}/roles/{roleId}/actions", operation: opRoleActions, vars: vars})
	} else if key == "groups" {
		reads = append(reads, snapshotRead{path: "/api/v1/orgs/{organizationId}/groups/{groupId}", vars: map[string]string{"organizationId": org, "groupId": id}})
	} else if key == "applications" {
		vars["applicationId"] = id
		reads = append(reads,
			snapshotRead{path: "/api/v1/orgs/{organizationId}/apps/{applicationId}/actions", operation: opApplicationActions, vars: vars},
			snapshotRead{path: "/api/v1/orgs/{organizationId}/apps/{applicationId}/consent", operation: opApplicationConsent, vars: vars, allowMissing: true},
		)
	} else {
		return "", nil
	}
	details := make([]json.RawMessage, 0, len(reads))
	for _, read := range reads {
		response, err := s.api.callWithMeta(r.Context(), http.MethodGet, read.path, read.vars, nil, nil, org, s.cfg.ManagementOrganizationID)
		if err != nil {
			if read.allowMissing && response.status == http.StatusNotFound {
				setFormETag(forms, read.operation, "*")
				continue
			}
			return "", &selectedReadError{statusFor(response.status), safeAPIError(response.body, response.status)}
		}
		if !json.Valid(response.body) || read.operation != "" && response.etag == "" {
			return "", &selectedReadError{http.StatusBadGateway, "응답 형식이 올바르지 않습니다."}
		}
		setFormETag(forms, read.operation, response.etag)
		if read.operation == opRoleActions || read.operation == opApplicationActions || read.operation == opApplicationConsent {
			setFormValue(forms, read.operation, "actionIds", strings.Join(selectedActionIDs(response.body), ", "))
		}
		if key == "groups" {
			var group struct {
				Name        string  `json:"name"`
				Description *string `json:"description"`
			}
			if json.Unmarshal(response.body, &group) != nil {
				return "", &selectedReadError{http.StatusBadGateway, "그룹 응답 형식이 올바르지 않습니다."}
			}
			setFormValue(forms, opGroupUpdate, "name", group.Name)
			description := ""
			if group.Description != nil {
				description = *group.Description
			}
			setFormValue(forms, opGroupUpdate, "description", description)
		}
		details = append(details, response.body)
	}
	pretty, _ := json.MarshalIndent(details, "", "  ")
	if len(details) == 1 {
		pretty, _ = json.MarshalIndent(details[0], "", "  ")
	}
	return string(pretty), nil
}

func setFormETag(forms []Form, operation, etag string) {
	for i := range forms {
		if forms[i].Hidden["operation"] == operation {
			if forms[i].Hidden == nil {
				forms[i].Hidden = map[string]string{}
			}
			forms[i].Hidden["etag"] = etag
		}
	}
}

func setFormValue(forms []Form, operation, name, value string) {
	for i := range forms {
		if forms[i].Hidden["operation"] == operation {
			for j := range forms[i].Fields {
				if forms[i].Fields[j].Name == name {
					forms[i].Fields[j].Value = value
				}
			}
		}
	}
}

func selectedActionIDs(value json.RawMessage) []string {
	var raw any
	if json.Unmarshal(value, &raw) != nil {
		return nil
	}
	if obj, ok := raw.(map[string]any); ok {
		raw = obj["actions"]
	}
	items, _ := raw.([]any)
	ids := make([]string, 0, len(items))
	for _, item := range items {
		if obj, ok := item.(map[string]any); ok {
			if id, ok := obj["id"].(string); ok {
				ids = append(ids, id)
			} else if id, ok := obj["actionId"].(string); ok {
				ids = append(ids, id)
			}
		}
	}
	return ids
}

func navigation() []NavItem {
	items := []NavItem{
		{ID: "", Label: "개요", Group: "플랫폼", Href: "/"},
		{ID: "providers", Label: "프로바이더", Group: "서비스 정의", Href: "/providers"},
		{ID: "actions", Label: "액션", Group: "서비스 정의", Href: "/actions"},
		{ID: "http", Label: "HTTP 기능", Group: "서비스 정의", Href: "/http"},
		{ID: "users", Label: "사용자", Group: "사용자·조직", Href: "/users"},
		{ID: "organizations", Label: "조직", Group: "사용자·조직", Href: "/organizations"},
		{ID: "members", Label: "구성원", Group: "조직 관리", Href: "/members"},
		{ID: "groups", Label: "그룹", Group: "조직 관리", Href: "/groups"},
		{ID: "agreements", Label: "계약", Group: "조직 관리", Href: "/agreements"},
		{ID: "roles", Label: "역할", Group: "권한", Href: "/roles"},
		{ID: "bindings", Label: "권한 부여", Group: "권한", Href: "/bindings"},
		{ID: "applications", Label: "애플리케이션", Group: "애플리케이션", Href: "/applications"},
	}
	return items
}

func (s *server) orgOptions(r *http.Request) []Option {
	b, _, err := s.api.call(r.Context(), http.MethodGet, "/api/v1/orgs", nil, nil, nil, "", s.cfg.ManagementOrganizationID)
	if err != nil {
		return nil
	}
	var orgs []struct {
		ID   string `json:"id"`
		Name string `json:"name"`
	}
	if json.Unmarshal(b, &orgs) != nil {
		return nil
	}
	out := make([]Option, 0, len(orgs))
	for _, org := range orgs {
		if validID(org.ID) {
			out = append(out, Option{Value: org.ID, Label: org.Name})
		}
	}
	return out
}

func (s *server) providerOptions(r *http.Request) []Option {
	b, _, err := s.api.call(r.Context(), http.MethodGet, "/api/v1/providers", nil, nil, nil, "", s.cfg.ManagementOrganizationID)
	if err != nil {
		return []Option{{Value: "grant-management", Label: "Grant management"}}
	}
	var providers []struct {
		ID   string `json:"id"`
		Name string `json:"name"`
	}
	if json.Unmarshal(b, &providers) != nil {
		return []Option{{Value: "grant-management", Label: "Grant management"}}
	}
	out := []Option{{Value: "grant-management", Label: "Grant management"}}
	seen := map[string]bool{"grant-management": true}
	for _, provider := range providers {
		if validID(provider.ID) && !seen[provider.ID] {
			seen[provider.ID] = true
			out = append(out, Option{Value: provider.ID, Label: provider.Name})
		}
	}
	return out
}

func (s *server) operation(w http.ResponseWriter, r *http.Request) {
	if err := r.ParseForm(); err != nil {
		http.Error(w, "invalid form", http.StatusBadRequest)
		return
	}
	key := r.FormValue("section")
	if _, ok := resources[key]; !ok {
		http.Error(w, "unknown section", http.StatusBadRequest)
		return
	}
	if !validID(r.FormValue("operation")) {
		http.Error(w, "invalid operation", http.StatusBadRequest)
		return
	}
	org, provider := r.FormValue("org"), r.FormValue("provider")
	if org != "" && !validID(org) {
		http.Error(w, "invalid organization", http.StatusBadRequest)
		return
	}
	if provider == "" {
		provider = "grant-management"
	}
	if !validID(provider) {
		http.Error(w, "invalid provider", http.StatusBadRequest)
		return
	}
	if err := validateOperation(key, r.Form); err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}
	method, route, vars, body, headers, err := operationRequest(key, provider, org, r.Form)
	if err != nil {
		http.Error(w, "unsupported operation", http.StatusBadRequest)
		return
	}
	data, status, err := s.api.call(r.Context(), method, route, vars, nil, body, org, s.cfg.ManagementOrganizationID, headers)
	if err != nil {
		s.renderError(w, r, key, org, provider, statusFor(status), safeAPIError(data, status))
		return
	}
	if r.FormValue("operation") == opHTTPStage {
		var staged struct {
			Generation uint64 `json:"generation"`
		}
		if json.Unmarshal(data, &staged) != nil || staged.Generation == 0 {
			s.renderError(w, r, key, org, provider, http.StatusBadGateway, "세대 생성 응답에 유효한 generation이 없습니다.")
			return
		}
		q := url.Values{"provider": []string{provider}, "generation": []string{strconv.FormatUint(staged.Generation, 10)}}
		target := "/http?" + q.Encode()
		if strings.EqualFold(r.Header.Get("HX-Request"), "true") {
			w.Header().Set("HX-Redirect", target)
			w.WriteHeader(http.StatusNoContent)
			return
		}
		http.Redirect(w, r, target, http.StatusSeeOther)
		return
	}
	_ = data
	// Redirect after a successful mutation so a refresh cannot repeat it.
	if !strings.EqualFold(r.Header.Get("HX-Request"), "true") {
		q := url.Values{}
		if org != "" {
			q.Set("org", org)
		}
		if provider != "" {
			q.Set("provider", provider)
		}
		target := "/" + key
		if key == "" {
			target = "/"
		}
		if encoded := q.Encode(); encoded != "" {
			target += "?" + encoded
		}
		http.Redirect(w, r, target, http.StatusSeeOther)
		return
	}
	r.URL.Path = "/" + key
	if key == "" {
		r.URL.Path = "/"
	}
	q := url.Values{}
	if org != "" {
		q.Set("org", org)
	}
	if provider != "" {
		q.Set("provider", provider)
	}
	r.URL.RawQuery = q.Encode()
	if method == http.MethodDelete {
		w.Header().Set("HX-Replace-Url", r.URL.RequestURI())
	}
	s.view(w, r)
}

func (s *server) renderError(w http.ResponseWriter, r *http.Request, key, org, provider string, status int, message string) {
	res := resources[key]
	if provider == "" {
		provider = "grant-management"
	}
	page := Page{Title: res.title, Description: res.description, Section: key, OrganizationID: org, ProviderID: provider, Search: r.URL.Query().Get("q"), CSRF: adminauth.CSRFToken(r.Context()), Error: message, Columns: res.columns, Forms: formsFor(key), Navigation: navigation(), Organizations: s.orgOptions(r), Providers: s.providerOptions(r)}
	for i := range page.Forms {
		page.Forms[i].Hidden["section"] = key
		page.Forms[i].Hidden["org"] = org
		page.Forms[i].Hidden["provider"] = provider
	}
	w.WriteHeader(status)
	_ = s.renderer.render(w, page, strings.EqualFold(r.Header.Get("HX-Request"), "true"))
}

func statusFor(status int) int {
	if status >= 400 && status <= 599 {
		return status
	}
	return http.StatusBadGateway
}

func validateOperation(section string, v url.Values) error {
	required := []string{}
	switch v.Get("operation") {
	case opProviderCreate:
		required = []string{"id", "name", "active"}
	case opProviderUpdate, opActionDelete, opGroupDelete, opBindingDelete, opApplicationDelete, opGrantRevoke:
		required = []string{"id"}
	case opMemberRemove:
		required = []string{"userId"}
	case opActionCreate:
		required = []string{"id", "code", "name", "riskLevel"}
	case opActionUpdate:
		required = []string{"id"}
	case opHTTPStage:
		required = []string{"generation"}
	case opHTTPActivate:
		required = []string{"targetGeneration", "targetCatalogDigest"}
	case opUserActive:
		required = []string{"id", "active"}
	case opOrganizationCreate:
		required = []string{"id", "name", "ownerUserId"}
	case opOrganizationUpdate:
		required = []string{"id", "name"}
	case opOrganizationDeactivate:
		required = []string{"id", "status"}
	case opOrganizationReactivate:
		required = []string{"id"}
	case opOrganizationOwner:
		required = []string{"id", "userId", "etag"}
	case opMemberEnroll:
		required = []string{"userId"}
	case opGroupCreate:
		required = []string{"id", "name"}
	case opGroupUpdate:
		required = []string{"id", "name"}
	case opGroupMemberAdd, opGroupMemberRemove:
		required = []string{"id", "userId"}
	case opAgreementCreate, opAgreementUpdate:
		required = []string{"id"}
	case opRoleCreate:
		required = []string{"id", "code", "name"}
	case opRoleUpdate, opRoleDelete:
		required = []string{"id"}
	case opRoleActions:
		required = []string{"id", "etag"}
	case opBindingCreate:
		required = []string{"id", "subjectType", "subjectId", "providerId", "resourceType", "resourceId", "roleId", "state"}
	case opApplicationCreate:
		required = []string{"id", "slug", "name", "applicationType", "appType"}
	case opApplicationUpdate:
		required = []string{"id"}
	case opApplicationActive:
		required = []string{"id", "active"}
	case opApplicationTrust:
		required = []string{"id", "trusted"}
	case opApplicationIdentity:
		required = []string{"id", "clientId"}
	case opApplicationActions:
		required = []string{"id", "etag"}
	case opApplicationConsent:
		required = []string{"id", "etag"}
	case opApplicationConsentRevoke:
		required = []string{"id"}
	default:
		return errors.New("unknown operation")
	}
	for _, name := range required {
		if strings.TrimSpace(v.Get(name)) == "" {
			return fmt.Errorf("%s is required", name)
		}
	}
	if v.Get("operation") == opGroupUpdate && !v.Has("description") {
		return errors.New("description must be submitted; an empty value clears it")
	}
	if v.Get("operation") == opGroupUpdate && (v.Get("selectedID") == "" || v.Get("selectedID") != v.Get("id")) {
		return errors.New("그룹을 선택한 뒤 수정하세요.")
	}
	if v.Get("operation") == opHTTPActivate && (strings.TrimSpace(v.Get("expectedGeneration")) == "") != (strings.TrimSpace(v.Get("expectedCatalogDigest")) == "") {
		return errors.New("현재 generation과 catalogDigest를 함께 입력하거나 모두 비워 두세요")
	}
	for _, name := range []string{"id", "userId", "ownerUserId", "applicationId", "providerId", "groupId", "agreementId", "roleId", "resourceId"} {
		if value := v.Get(name); value != "" && !validID(value) {
			return fmt.Errorf("invalid %s", name)
		}
	}
	owner := map[string]string{
		opProviderCreate: "providers", opProviderUpdate: "providers", opActionCreate: "actions", opActionUpdate: "actions", opActionDelete: "actions", opHTTPStage: "http", opHTTPActivate: "http", opUserActive: "users",
		opOrganizationCreate: "organizations", opOrganizationUpdate: "organizations", opOrganizationDeactivate: "organizations", opOrganizationReactivate: "organizations", opOrganizationOwner: "organizations",
		opMemberEnroll: "members", opMemberRemove: "members", opGroupCreate: "groups", opGroupUpdate: "groups", opGroupDelete: "groups", opGroupMemberAdd: "groups", opGroupMemberRemove: "groups",
		opAgreementCreate: "agreements", opAgreementUpdate: "agreements", opRoleCreate: "roles", opRoleUpdate: "roles", opRoleDelete: "roles", opRoleActions: "roles", opBindingCreate: "bindings", opBindingDelete: "bindings",
		opApplicationCreate: "applications", opApplicationUpdate: "applications", opApplicationDelete: "applications", opApplicationActive: "applications", opApplicationActions: "applications", opApplicationTrust: "applications", opApplicationIdentity: "applications", opApplicationConsent: "applications", opApplicationConsentRevoke: "applications", opGrantRevoke: "applications",
	}
	if owner[v.Get("operation")] != section {
		return errors.New("operation does not belong to this page")
	}
	return nil
}

func operationRequest(section, provider, org string, f url.Values) (string, string, map[string]string, any, http.Header, error) {
	op, id := f.Get("operation"), f.Get("id")
	vars := map[string]string{"providerId": provider, "organizationId": org}
	body := map[string]any{}
	headers := make(http.Header)
	str := func(name string) string { return f.Get(name) }
	optional := func(name string) {
		if f.Has(name) && str(name) != "" {
			body[name] = str(name)
		}
	}
	boolValue := func(name string) (bool, error) { b, e := strconv.ParseBool(str(name)); return b, e }
	list := func(name string) []string {
		raw := strings.Split(str(name), ",")
		out := []string{}
		for _, item := range raw {
			if item = strings.TrimSpace(item); item != "" {
				out = append(out, item)
			}
		}
		return out
	}
	set := func(names ...string) {
		for _, name := range names {
			if f.Has(name) && str(name) != "" {
				body[name] = str(name)
			}
		}
	}
	var method, route string
	switch op {
	case opProviderCreate:
		method, route = "POST", "/api/v1/admin/providers"
		set("id", "name", "description")
		b, e := boolValue("active")
		if e != nil {
			return "", "", nil, nil, nil, errors.New("active must be true or false")
		}
		body["active"] = b
	case opProviderUpdate:
		method, route, vars["providerId"] = "PATCH", "/api/v1/admin/providers/{providerId}", id
		set("name", "description")
		if f.Has("active") && str("active") != "" {
			b, e := boolValue("active")
			if e != nil {
				return "", "", nil, nil, nil, e
			}
			body["active"] = b
		}
	case opActionCreate:
		method, route = "POST", "/api/v1/admin/providers/{providerId}/actions"
		set("id", "code", "name", "description", "riskLevel", "consentLabel", "consentDescription")
		for _, n := range []string{"visibilityGranting", "grantableToApplications"} {
			b, e := boolValue(n)
			if e != nil {
				return "", "", nil, nil, nil, fmt.Errorf("%s must be true or false", n)
			}
			body[n] = b
		}
	case opActionUpdate:
		method, route, vars["actionId"] = "PATCH", "/api/v1/admin/providers/{providerId}/actions/{actionId}", id
		vars["providerId"] = provider
		set("name", "description", "riskLevel", "consentLabel", "consentDescription")
		for _, n := range []string{"visibilityGranting", "grantableToApplications"} {
			if f.Has(n) && str(n) != "" {
				b, e := boolValue(n)
				if e != nil {
					return "", "", nil, nil, nil, e
				}
				body[n] = b
			}
		}
	case opActionDelete:
		method, route, vars["actionId"] = "DELETE", "/api/v1/admin/providers/{providerId}/actions/{actionId}", id
	case opHTTPStage:
		method, route = "POST", "/api/v1/admin/providers/{providerId}/http-capability-generations"
		stage, err := decodeStageGeneration(str("generation"))
		if err != nil {
			return "", "", nil, nil, nil, errors.New("generation must be a JSON object")
		}
		return method, route, vars, stage, headers, nil
	case opHTTPActivate:
		method, route = "POST", "/api/v1/admin/providers/{providerId}/http-capability-active"
		gen, e := strconv.ParseUint(str("targetGeneration"), 10, 64)
		if e != nil || gen == 0 {
			return "", "", nil, nil, nil, errors.New("generation must be a positive integer")
		}
		var expected *uint64
		if str("expectedGeneration") != "" {
			value, parseErr := strconv.ParseUint(str("expectedGeneration"), 10, 64)
			if parseErr != nil || value == 0 {
				return "", "", nil, nil, nil, errors.New("expected generation must be a positive integer")
			}
			expected = &value
		}
		var expectedDigest *string
		if str("expectedCatalogDigest") != "" {
			value := str("expectedCatalogDigest")
			expectedDigest = &value
		}
		body = map[string]any{"expectedGeneration": expected, "expectedCatalogDigest": expectedDigest, "targetGeneration": gen, "targetCatalogDigest": str("targetCatalogDigest")}
	case opUserActive:
		method, route, vars["userId"] = "PUT", "/api/v1/admin/users/{userId}/active", id
		b, e := boolValue("active")
		if e != nil {
			return "", "", nil, nil, nil, errors.New("active must be true or false")
		}
		body = map[string]any{"active": b}
	case opOrganizationCreate:
		method, route = "POST", "/api/v1/admin/orgs"
		set("id", "name", "ownerUserId")
	case opOrganizationUpdate:
		method, route = "PATCH", "/api/v1/orgs/{organizationId}"
		vars["organizationId"] = id
		set("name")
	case opOrganizationDeactivate, opOrganizationReactivate:
		method = "POST"
		action := "deactivate"
		if op == opOrganizationReactivate || str("status") == "reactivate" {
			action = "reactivate"
		}
		route = "/api/v1/orgs/{organizationId}/" + action
		vars["organizationId"] = id
	case opOrganizationOwner:
		method, route = "PUT", "/api/v1/orgs/{organizationId}/owner"
		vars["organizationId"] = id
		body = map[string]any{"ownerUserId": str("userId")}
		headers.Set("If-Match", str("etag"))
	case opMemberEnroll, opMemberRemove:
		method = "PUT"
		action := ""
		if op == opMemberRemove {
			method = "DELETE"
		}
		route = "/api/v1/orgs/{organizationId}/members/{userId}"
		vars["organizationId"], vars["userId"] = org, str("userId")
		_ = action
	case opGroupCreate:
		method, route = "POST", "/api/v1/orgs/{organizationId}/groups"
		set("id", "name", "description")
	case opGroupUpdate:
		method, route = "PUT", "/api/v1/orgs/{organizationId}/groups/{groupId}"
		vars["groupId"] = id
		body["name"] = str("name")
		body["description"] = str("description")
	case opGroupDelete:
		method, route = "DELETE", "/api/v1/orgs/{organizationId}/groups/{groupId}"
		vars["groupId"] = id
	case opGroupMemberAdd, opGroupMemberRemove:
		method = "PUT"
		if op == opGroupMemberRemove {
			method = "DELETE"
		}
		route = "/api/v1/orgs/{organizationId}/groups/{groupId}/members/{userId}"
		vars["groupId"], vars["userId"] = id, str("userId")
	case opAgreementCreate, opAgreementUpdate:
		route = "/api/v1/orgs/{organizationId}/agreements"
		if op == opAgreementUpdate {
			method = "PATCH"
			route += "/{agreementId}"
			vars["agreementId"] = id
			set("title", "agreementType", "status")
		} else {
			method = "POST"
			set("id", "title", "agreementType", "status", "grantorOrganizationId", "granteeOrganizationId", "referenceNo", "startsAt", "endsAt")
		}
	case opRoleCreate:
		method, route = "POST", "/api/v1/orgs/{organizationId}/roles"
		set("id", "code", "name", "description")
	case opRoleUpdate:
		method, route = "PATCH", "/api/v1/orgs/{organizationId}/roles/{roleId}"
		vars["roleId"] = id
		set("name", "description")
	case opRoleDelete:
		method, route = "DELETE", "/api/v1/orgs/{organizationId}/roles/{roleId}"
		vars["roleId"] = id
	case opRoleActions:
		method, route = "PUT", "/api/v1/orgs/{organizationId}/roles/{roleId}/actions"
		vars["roleId"] = id
		body = map[string]any{"actionIds": list("actionIds")}
		headers.Set("If-Match", str("etag"))
	case opBindingCreate:
		method, route = "POST", "/api/v1/orgs/{organizationId}/bindings"
		set("id", "subjectType", "subjectId", "providerId", "resourceType", "resourceId", "roleId", "state")
		optional("agreementId")
		body["subjectOrganizationId"] = org
	case opBindingDelete:
		method, route = "DELETE", "/api/v1/orgs/{organizationId}/bindings/{bindingId}"
		vars["bindingId"] = id
	case opApplicationCreate:
		method, route = "POST", "/api/v1/orgs/{organizationId}/apps"
		set("id", "slug", "name", "description", "applicationType", "appType", "redirectUri")
		b, e := boolValue("active")
		if e != nil {
			return "", "", nil, nil, nil, errors.New("active must be true or false")
		}
		body["active"] = b
	case opApplicationUpdate:
		method, route = "PATCH", "/api/v1/orgs/{organizationId}/apps/{applicationId}"
		vars["applicationId"] = id
		set("name", "description")
	case opApplicationDelete:
		method, route = "DELETE", "/api/v1/orgs/{organizationId}/apps/{applicationId}"
		vars["applicationId"] = id
	case opApplicationActive:
		method, route = "PUT", "/api/v1/orgs/{organizationId}/apps/{applicationId}/active"
		vars["applicationId"] = id
		b, e := boolValue("active")
		if e != nil {
			return "", "", nil, nil, nil, e
		}
		body = map[string]any{"active": b}
	case opApplicationActions:
		method, route = "PUT", "/api/v1/orgs/{organizationId}/apps/{applicationId}/actions"
		vars["applicationId"] = id
		body = map[string]any{"actionIds": list("actionIds")}
		headers.Set("If-Match", str("etag"))
	case opApplicationTrust:
		method, route = "PUT", "/api/v1/admin/orgs/{organizationId}/apps/{applicationId}/trust"
		vars["organizationId"], vars["applicationId"] = org, id
		b, e := strconv.ParseBool(str("trusted"))
		if e != nil {
			return "", "", nil, nil, nil, errors.New("trusted must be true or false")
		}
		body = map[string]any{"trusted": b}
	case opApplicationIdentity:
		method, route = "PUT", "/api/v1/admin/orgs/{organizationId}/apps/{applicationId}/identity-binding"
		vars["organizationId"], vars["applicationId"] = org, id
		body = map[string]any{"clientId": str("clientId")}
	case opApplicationConsent:
		method, route = "PUT", "/api/v1/orgs/{organizationId}/apps/{applicationId}/consent"
		vars["applicationId"] = id
		body = map[string]any{"actionIds": list("actionIds")}
		if f.Get("etag") == "*" {
			headers.Set("If-None-Match", "*")
		} else if f.Get("etag") != "" {
			headers.Set("If-Match", str("etag"))
		}
	case opApplicationConsentRevoke:
		method, route = "DELETE", "/api/v1/orgs/{organizationId}/apps/{applicationId}/consent"
		vars["applicationId"] = id
	case opGrantRevoke:
		method, route = "DELETE", "/api/v1/orgs/{organizationId}/app-grants/{grantId}"
		vars["grantId"] = id
	default:
		return "", "", nil, nil, nil, errors.New("unsupported operation")
	}
	if method != "DELETE" && body == nil {
		body = map[string]any{}
	}
	return method, route, vars, body, headers, nil
}

func decodeStageGeneration(raw string) (grant.StageHttpCapabilityGeneration, error) {
	var stage grant.StageHttpCapabilityGeneration
	if !strings.HasPrefix(strings.TrimSpace(raw), "{") {
		return stage, errors.New("generation must be an object")
	}
	decoder := json.NewDecoder(strings.NewReader(raw))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&stage); err != nil {
		return stage, err
	}
	if err := decoder.Decode(&struct{}{}); err != io.EOF {
		return stage, errors.New("generation must contain one JSON object")
	}
	return stage, nil
}

var _ = fmt.Sprintf
