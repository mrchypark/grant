package api

import (
	"encoding/json"
	"testing"

	"github.com/Conalog/grant/internal/grant"
)

func TestAgreementClassificationHTTPCreatePutPatch(t *testing.T) {
	s := managementTestServer(t)
	lifecycleCall(t, s, "POST", "/api/v1/admin/orgs", `{"id":"receiver","name":"Receiver","ownerUserId":"owner"}`, "mgmt-org", true, 201)
	root := "/api/v1/orgs/owner/agreements"
	input := grant.UpsertAgreementInput{ID: "custom-type", AgreementType: "maintenance-contract", Title: "Custom", Status: "active", GrantorOrganizationID: "owner", GranteeOrganizationID: "receiver"}
	request := func(method, path string, body any, want int) grant.Agreement {
		t.Helper()
		encoded, err := json.Marshal(body)
		if err != nil {
			t.Fatal(err)
		}
		response := lifecycleCall(t, s, method, path, string(encoded), "owner", false, want)
		var got grant.Agreement
		if want < 300 {
			if err = json.Unmarshal(response.Body.Bytes(), &got); err != nil {
				t.Fatal(err)
			}
		}
		return got
	}
	if got := request("POST", root, input, 201); got.AgreementType != input.AgreementType {
		t.Fatal(got)
	}
	input.AgreementType = "shared-energy-service"
	if got := request("PUT", root+"/custom-type", input, 200); got.AgreementType != input.AgreementType {
		t.Fatal(got)
	}
	if got := request("PATCH", root+"/custom-type", map[string]any{"agreementType": "  공동 운영  "}, 200); got.AgreementType != "공동 운영" {
		t.Fatal(got)
	}
	input.AgreementType = ""
	request("POST", root, input, 400)
	request("PUT", root+"/custom-type", input, 400)
	request("PATCH", root+"/custom-type", map[string]any{"agreementType": ""}, 400)
	request("PATCH", root+"/custom-type", map[string]any{"agreementType": nil}, 422)
	input.AgreementType = "custom-type"
	input.Status = "unknown"
	request("PUT", root+"/custom-type", input, 422)
}
