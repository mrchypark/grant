package adminui

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/url"
	"strings"
	"testing"

	"github.com/mrchypark/grant/internal/grant"
)

func TestEveryFormHasFieldsAndMapsToManagementRoute(t *testing.T) {
	sections := []string{"providers", "actions", "http", "users", "organizations", "members", "groups", "agreements", "roles", "bindings", "applications"}
	renderer, err := newRenderer()
	if err != nil {
		t.Fatal(err)
	}
	for _, section := range sections {
		for _, definition := range formsFor(section) {
			t.Run(section+"/"+definition.ID, func(t *testing.T) {
				if len(definition.Fields) == 0 {
					t.Fatal("form has no inputs")
				}
				values := url.Values{"operation": {definition.Hidden["operation"]}}
				if definition.Hidden["operation"] == opGroupUpdate {
					values.Set("selectedID", "id-1")
				}
				for _, field := range definition.Fields {
					values.Set(field.Name, representativeValue(field))
				}
				if strings.Contains(definition.Hidden["operation"], "actions") || definition.Hidden["operation"] == opApplicationConsent {
					values.Set("etag", `"snapshot-v1"`)
				}
				if definition.Hidden["operation"] == opOrganizationOwner {
					values.Set("etag", `"snapshot-v1"`)
				}
				if err := validateOperation(section, values); err != nil {
					t.Fatalf("form values do not pass validation: %v; values=%v", err, values)
				}
				provider, org := "provider-1", "org-1"
				method, route, vars, body, _, err := operationRequest(section, provider, org, values)
				if err != nil {
					t.Fatalf("form has no request mapping: %v", err)
				}
				if _, ok := grant.LookupManagementRoute(method, route); !ok {
					t.Fatalf("form maps to unregistered API route %s %s", method, route)
				}
				for name, value := range vars {
					if strings.Contains(route, "{"+name+"}") && !validID(value) {
						t.Fatalf("route variable %s is invalid: %q", name, value)
					}
				}
				if definition.Hidden["operation"] == opAgreementUpdate {
					encoded, _ := json.Marshal(body)
					for _, forbidden := range []string{"id", "grantorOrganizationId", "granteeOrganizationId"} {
						if strings.Contains(string(encoded), `"`+forbidden+`"`) {
							t.Fatalf("agreement PATCH includes disallowed field %s: %s", forbidden, encoded)
						}
					}
				}
				var output bytes.Buffer
				page := Page{Title: section, Section: section, Forms: []Form{definition}}
				if err := renderer.render(&output, page, false); err != nil {
					t.Fatalf("render form: %v", err)
				}
				for _, field := range definition.Fields {
					if !strings.Contains(output.String(), `name="`+field.Name+`"`) {
						t.Errorf("rendered form omitted input %q", field.Name)
					}
				}
			})
		}
	}
}

func representativeValue(field Field) string {
	if len(field.Options) > 0 {
		return field.Options[0].Value
	}
	switch field.Name {
	case "id", "userId", "ownerUserId", "subjectId", "resourceId", "roleId", "agreementId", "applicationId", "providerId":
		return "id-1"
	case "actionIds":
		return ""
	case "generation":
		return `{"schemaVersion":1,"generation":1,"expectedLatestGeneration":0,"capabilities":[]}`
	case "targetGeneration":
		return "1"
	case "targetCatalogDigest":
		return "digest-v1"
	case "expectedGeneration", "expectedCatalogDigest":
		return ""
	default:
		if field.Required {
			return "value"
		}
		return ""
	}
}

func TestConsentAndAgreementPatchContracts(t *testing.T) {
	consent := url.Values{"operation": {opApplicationConsent}, "id": {"app-1"}, "etag": {`"consent-v1"`}}
	if err := validateOperation("applications", consent); err != nil {
		t.Fatal(err)
	}
	method, route, _, _, headers, err := operationRequest("applications", "provider-1", "org-1", consent)
	if err != nil || method != http.MethodPut || route != "/api/v1/orgs/{organizationId}/apps/{applicationId}/consent" || headers.Get("If-Match") != `"consent-v1"` || headers.Get("If-None-Match") != "" {
		t.Fatalf("existing consent replacement contract: method=%s route=%s headers=%v err=%v", method, route, headers, err)
	}
	consent.Set("etag", "*")
	_, _, _, _, headers, err = operationRequest("applications", "provider-1", "org-1", consent)
	if err != nil || headers.Get("If-None-Match") != "*" || headers.Get("If-Match") != "" {
		t.Fatalf("new consent contract: headers=%v err=%v", headers, err)
	}
	agreement := url.Values{"operation": {opAgreementUpdate}, "id": {"agreement-1"}, "title": {"Updated"}, "status": {"active"}}
	if err := validateOperation("agreements", agreement); err != nil {
		t.Fatal(err)
	}
	method, route, _, body, _, err := operationRequest("agreements", "provider-1", "org-1", agreement)
	if err != nil || method != http.MethodPatch || route != "/api/v1/orgs/{organizationId}/agreements/{agreementId}" {
		t.Fatalf("agreement PATCH route: method=%s route=%s err=%v", method, route, err)
	}
	encoded, err := json.Marshal(body)
	if err != nil {
		t.Fatal(err)
	}
	for _, forbidden := range []string{"id", "grantorOrganizationId", "granteeOrganizationId"} {
		if strings.Contains(string(encoded), `"`+forbidden+`"`) {
			t.Fatalf("PATCH body contains immutable field %s: %s", forbidden, encoded)
		}
	}
}

func TestHTTPActivationRequiresBothCurrentCASValues(t *testing.T) {
	base := url.Values{"operation": {opHTTPActivate}, "targetGeneration": {"2"}, "targetCatalogDigest": {"sha256:target"}}
	if err := validateOperation("http", base); err != nil {
		t.Fatalf("first activation with no current catalog: %v", err)
	}
	base.Set("expectedGeneration", "1")
	if err := validateOperation("http", base); err == nil {
		t.Fatal("accepted only expectedGeneration")
	}
	base.Del("expectedGeneration")
	base.Set("expectedCatalogDigest", "sha256:current")
	if err := validateOperation("http", base); err == nil {
		t.Fatal("accepted only expectedCatalogDigest")
	}
	base.Set("expectedGeneration", "1")
	if err := validateOperation("http", base); err != nil {
		t.Fatalf("accepted complete current CAS pair: %v", err)
	}
}

func TestNoActiveCatalogRequiresExactAPIError(t *testing.T) {
	if !isNoActiveHTTPCatalog([]byte(`{"error":"HTTP capability catalog not active"}`)) {
		t.Fatal("did not recognize the API's inactive-catalog response")
	}
	for _, response := range []string{
		`{"error":"service provider not found"}`,
		`{"error":"not authorized"}`,
		`{"error":"HTTP capability catalog not active\nextra"}`,
	} {
		if isNoActiveHTTPCatalog([]byte(response)) {
			t.Fatalf("treated a distinct API error as an empty catalog: %s", response)
		}
	}
}

func TestHTTPGenerationReadUsesRegisteredRouteAndPrefillsActivation(t *testing.T) {
	const route = "/api/v1/admin/providers/{providerId}/http-capability-generations/{generation}"
	if _, ok := grant.LookupManagementRoute(http.MethodGet, route); !ok {
		t.Fatal("HTTP generation review route is missing from the management registry")
	}
	forms := formsFor("http")
	setHTTPActivationCurrent(forms, []byte(`{"generation":4,"catalogDigest":"current-digest"}`))
	setHTTPActivationTarget(forms, []byte(`{"providerId":"grant","generation":5,"catalogDigest":"staged-digest","capabilityCount":2,"responseResourceCount":1,"capabilities":[],"responseResources":[]}`))
	values := map[string]string{}
	for _, form := range forms {
		if form.Hidden["operation"] != opHTTPActivate {
			continue
		}
		for _, field := range form.Fields {
			values[field.Name] = field.Value
		}
	}
	if values["expectedGeneration"] != "4" || values["expectedCatalogDigest"] != "current-digest" || values["targetGeneration"] != "5" || values["targetCatalogDigest"] != "staged-digest" {
		t.Fatalf("activation form not populated from both catalog reads: %#v", values)
	}
}

func TestHTTPStagePreservesUint64GenerationPrecision(t *testing.T) {
	const exact = uint64(9007199254740993)
	values := url.Values{
		"operation":  {opHTTPStage},
		"generation": {`{"schemaVersion":1,"generation":9007199254740993,"expectedLatestGeneration":9007199254740992,"capabilities":[],"catalogDigest":"sha256:test"}`},
	}
	method, route, _, body, _, err := operationRequest("http", "provider-1", "", values)
	if err != nil || method != http.MethodPost || route != "/api/v1/admin/providers/{providerId}/http-capability-generations" {
		t.Fatalf("stage request route: method=%s route=%s err=%v", method, route, err)
	}
	stage, ok := body.(grant.StageHttpCapabilityGeneration)
	if !ok {
		t.Fatalf("stage request body has lossy/untyped form %T", body)
	}
	if stage.Generation != exact || stage.ExpectedLatestGeneration == nil || *stage.ExpectedLatestGeneration != exact-1 {
		t.Fatalf("stage generations lost precision: %#v", stage)
	}
	encoded, err := json.Marshal(stage)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(encoded), `"generation":9007199254740993`) || !strings.Contains(string(encoded), `"expectedLatestGeneration":9007199254740992`) {
		t.Fatalf("serialized stage request rounded integer values: %s", encoded)
	}
}

func TestSelectedRowPrefillsOnlyMatchingExistingEntityForms(t *testing.T) {
	tests := []struct {
		section string
		want    map[string]map[string]string
	}{
		{"groups", map[string]map[string]string{
			"group.create": {"id": ""}, "group.update": {"id": "group-1"},
			"group.member.add": {"id": "group-1", "userId": ""},
		}},
		{"organizations", map[string]map[string]string{
			"organization.create": {"id": ""}, "organization.owner": {"id": "org-1", "userId": ""},
		}},
		{"members", map[string]map[string]string{
			"member.enroll": {"userId": ""}, "member.remove": {"userId": "user-1"},
		}},
		{"applications", map[string]map[string]string{
			"application.create": {"id": ""}, "application.delete": {"id": "app-1"}, "application.grant.revoke": {"id": ""},
		}},
	}
	for _, test := range tests {
		t.Run(test.section, func(t *testing.T) {
			forms := formsFor(test.section)
			selected := "group-1"
			if test.section == "organizations" {
				selected = "org-1"
			} else if test.section == "members" {
				selected = "user-1"
			} else if test.section == "applications" {
				selected = "app-1"
			}
			prefillSelectedEntity(forms, test.section, selected)
			for operation, fields := range test.want {
				var got Form
				for _, form := range forms {
					if form.Hidden["operation"] == operation {
						got = form
						break
					}
				}
				if got.ID == "" {
					t.Fatalf("missing form for operation %q", operation)
				}
				for name, want := range fields {
					value := ""
					for _, field := range got.Fields {
						if field.Name == name {
							value = field.Value
						}
					}
					if value != want {
						t.Errorf("%s.%s value = %q, want %q", operation, name, value, want)
					}
				}
			}
		})
	}
}

func TestGroupUpdateRequiresExplicitDescriptionAndSendsEmptyToClear(t *testing.T) {
	values := url.Values{"operation": {opGroupUpdate}, "id": {"group-1"}, "selectedID": {"group-1"}, "name": {"Renamed"}}
	if err := validateOperation("groups", values); err == nil {
		t.Fatal("omitted group description must not silently clear stored data")
	}
	values.Set("description", "")
	if err := validateOperation("groups", values); err != nil {
		t.Fatalf("explicit blank description should mean clear: %v", err)
	}
	values.Set("selectedID", "other-group")
	if err := validateOperation("groups", values); err == nil {
		t.Fatal("group update must match the group selected and read into the form")
	}
	values.Set("selectedID", "group-1")
	method, route, _, body, _, err := operationRequest("groups", "provider-1", "org-1", values)
	if err != nil || method != http.MethodPut || route != "/api/v1/orgs/{organizationId}/groups/{groupId}" {
		t.Fatalf("group update mapping: %s %s err=%v", method, route, err)
	}
	got, ok := body.(map[string]any)
	if !ok || got["name"] != "Renamed" || got["description"] != "" {
		t.Fatalf("group full-replacement body must carry explicit empty description: %#v", body)
	}
}

func TestOptionalBooleanFieldsIgnoreBlankSelectors(t *testing.T) {
	for _, tc := range []struct {
		operation string
		section   string
		fields    []string
	}{
		{opProviderUpdate, "providers", []string{"active"}},
		{opActionUpdate, "actions", []string{"visibilityGranting", "grantableToApplications"}},
	} {
		t.Run(tc.operation, func(t *testing.T) {
			values := url.Values{"operation": {tc.operation}, "id": {"entity-1"}}
			for _, field := range tc.fields {
				values.Set(field, "")
			}
			_, _, _, body, _, err := operationRequest(tc.section, "provider-1", "org-1", values)
			if err != nil {
				t.Fatalf("blank optional bool should be omitted: %v", err)
			}
			got, ok := body.(map[string]any)
			if !ok {
				t.Fatalf("unexpected body type %T", body)
			}
			for _, field := range tc.fields {
				if _, exists := got[field]; exists {
					t.Errorf("blank optional field %q was sent: %#v", field, got)
				}
			}
		})
	}
}

func TestManagementPathIgnoresUnusedBlankVariablesButRejectsRequiredBlank(t *testing.T) {
	path, err := expandManagementPath("/api/v1/admin/providers", map[string]string{"organizationId": ""})
	if err != nil || path != "/api/v1/admin/providers" {
		t.Fatalf("unused organization variable blocked platform route: path=%q err=%v", path, err)
	}
	if _, err := expandManagementPath("/api/v1/orgs/{organizationId}/groups", map[string]string{"organizationId": ""}); err == nil {
		t.Fatal("blank required route placeholder must be rejected")
	}
	method, route, vars, _, _, err := operationRequest("providers", "provider-1", "", url.Values{"operation": {opProviderCreate}, "id": {"provider-new"}, "name": {"New"}, "active": {"true"}})
	if err != nil || method != http.MethodPost || route != "/api/v1/admin/providers" {
		t.Fatalf("platform create operation did not map: %s %s err=%v", method, route, err)
	}
	path, err = expandManagementPath(route, vars)
	if err != nil || path != route {
		t.Fatalf("platform create expansion failed due to unused blank organization: %q %v", path, err)
	}
}

func TestOrganizationLifecycleFieldUsesJSONFieldMapping(t *testing.T) {
	rows, err := rowsFromJSON([]byte(`[{"id":"org-1","name":"조직","partyId":"party-1","inactivatedAt":"2026-10-01T00:00:00Z"}]`), "organizations")
	if err != nil || len(rows) != 1 {
		t.Fatalf("organization rows: %+v err=%v", rows, err)
	}
	if got := rowCellForJSONField("organizations", "inactivatedAt", rows[0].Cells); got != "2026-10-01T00:00:00Z" {
		t.Fatalf("inactive marker lookup = %q", got)
	}
}
