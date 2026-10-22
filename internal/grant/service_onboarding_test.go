package grant

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestOrglessOnboardingCreatesOnlySelfAndEnablesScopedManagement(t *testing.T) {
	ctx, s, input := managementMaterializeFixture(t, "mgmt-org")
	lifecycleExec(t, ctx, s, "UPDATE applications SET first_party=1,consent_policy='auto_approve' WHERE id='app-1'")
	lifecycleExec(t, ctx, s, "INSERT OR IGNORE INTO application_actions(application_id,action_id) SELECT 'app-1',id FROM actions WHERE provider_id='grant-management'")
	lifecycleExec(t, ctx, s, "DELETE FROM user_identities WHERE user_id='owner'")
	before := lifecycleCount(t, ctx, s, "SELECT id FROM users")
	decision, err := s.DecideHttpCapability(ctx, "tok", "", input)
	if err != nil || !decision.Allowed {
		t.Fatal(decision, err)
	}
	if after := lifecycleCount(t, ctx, s, "SELECT id FROM users"); after != before {
		t.Fatal("PDP materialized identity")
	}
	ensured, err := s.EnsureCurrentUser(ctx, "tok")
	if err != nil || ensured.UserID == "owner" {
		t.Fatal(ensured, err)
	}
	orgs, err := s.ListOrganizationsAuthorized(ctx, ensured.UserID, ReadStrong)
	if err != nil || len(orgs) != 0 {
		t.Fatal(orgs, err)
	}
	var rejected SelfOrganizationInput
	if json.Unmarshal([]byte(`{"id":"steal","name":"Steal","ownerUserId":"owner"}`), &rejected) == nil {
		t.Fatal("caller supplied owner")
	}
	created, err := s.CreateOwnOrganization(ctx, "tok", SelfOrganizationInput{ID: "my-new-org", Name: "Mine"})
	if err != nil {
		t.Fatal(err)
	}
	if _, err = s.CreateOwnOrganization(ctx, "tok", SelfOrganizationInput{ID: created.ID, Name: "Overwrite"}); ErrorStatus(err) != 409 {
		t.Fatal("duplicate create", err)
	}
	if got, err := s.GetOrganizationAuthorized(ctx, ensured.UserID, created.ID, ReadStrong); err != nil || got.Name != "Mine" {
		t.Fatal(got, err)
	}
	stage, _, err := ManagementCatalog(1, nil)
	if err != nil {
		t.Fatal(err)
	}
	request := func(method, path, org, resource, body string) HttpCapabilityDecision {
		t.Helper()
		r := input
		for _, cap := range stage.Capabilities {
			concrete := strings.ReplaceAll(cap.PathTemplate, "{organizationId}", org)
			if cap.Method != method || concrete != path {
				continue
			}
			r.CapabilityID, r.CapabilityVersion, r.ContractDigest = cap.CapabilityID, cap.Version, cap.ContractDigest
			r.Method, r.CanonicalPath = method, path
			r.ResourceType, r.ResourceID = cap.Resource.ResourceType, resource
			r.ResourceEvidence = HttpCapabilityResourceEvidence{Kind: cap.Resource.ID.Kind, Name: &cap.Resource.ID.Name, CanonicalValue: resource}
			if cap.Resource.ID.Kind == "constant" {
				r.ResourceEvidence.Name = nil
			}
			r.BodyLength = uint64(len(body))
			r.BodyDigest = sha256Text(body)
			d, e := s.DecideHttpCapability(ctx, "tok", org, r)
			if e != nil {
				t.Fatal(e)
			}
			return d
		}
		t.Fatal("missing capability", path)
		return HttpCapabilityDecision{}
	}
	if !request("GET", "/api/v1/me", "", "grant", "").Allowed {
		t.Fatal("self read denied")
	}
	if !request("GET", "/api/v1/orgs", "", "grant", "").Allowed {
		t.Fatal("org list denied")
	}
	if !request("POST", "/api/v1/orgs", "", "grant", `{"id":"other","name":"Other"}`).Allowed {
		t.Fatal("create admission denied")
	}
	if !request("GET", "/api/v1/orgs/my-new-org", created.ID, created.ID, "").Allowed {
		t.Fatal("new owner cannot manage through configured app")
	}
	if _, err = s.DeactivateOrganization(ctx, ensured.UserID, created.ID, created.ID); err != nil {
		t.Fatal(err)
	}
	if !request("POST", "/api/v1/orgs/my-new-org/reactivate", created.ID, created.ID, "").Allowed {
		t.Fatal("trusted owner cannot reactivate")
	}
	if _, err = s.ReactivateOrganization(ctx, ensured.UserID, created.ID, created.ID); err != nil {
		t.Fatal(err)
	}
	lifecycleExec(t, ctx, s, "INSERT INTO users(id,display_name,active,created_at) VALUES('future-owner','Future',1,?)", nowString())
	if _, err = s.EnrollOrganizationMemberAuthorized(ctx, ensured.UserID, created.ID, "future-owner"); err != nil {
		t.Fatal(err)
	}
	ownerActions, e := s.store.Query(ctx, ReadStrong, "SELECT action_id FROM role_actions WHERE role_id='role-system-management-owner'")
	if e != nil {
		t.Fatal(e)
	}
	for _, row := range ownerActions {
		id := rowString(row, "action_id")
		safe := false
		for _, cap := range stage.Capabilities {
			if cap.ActionID != nil && *cap.ActionID == id {
				safe = managementOrganizationCapability("grant-management", cap)
				break
			}
		}
		if !safe {
			t.Fatal("system owner gained non-organization rights", id)
		}
	}
	if n := lifecycleCount(t, ctx, s, "SELECT id FROM role_bindings WHERE role_id='role-system-management-owner' AND (resource_type<>'organization' OR resource_id<>'my-new-org' OR subject_id<>?)", ensured.UserID); n != 0 {
		t.Fatal("owner gained unrelated resource rights")
	}
	owner, err := s.GetOrganizationOwnerAuthorized(ctx, ensured.UserID, created.ID)
	if err != nil {
		t.Fatal(err)
	}
	if _, err = s.TransferOrganizationOwnerAuthorized(ctx, ensured.UserID, created.ID, "future-owner", owner.ETag); err != nil {
		t.Fatal(err)
	}
	if request("GET", "/api/v1/orgs/my-new-org", created.ID, created.ID, "").Allowed {
		t.Fatal("previous owner retained fine management")
	}
	for _, mutation := range []string{
		"UPDATE applications SET first_party=0 WHERE id='app-1'",
		"UPDATE applications SET active=0 WHERE id='app-1'",
		"UPDATE organizations SET inactivated_at='now' WHERE id='mgmt-org'",
		"UPDATE provider_pep_clients SET lifecycle='disabled' WHERE provider_id='grant-management'",
		"UPDATE service_providers SET active=0 WHERE id='grant-management'",
		"DELETE FROM application_actions WHERE application_id='app-1' AND action_id='grant-management.user.materialize'",
	} {
		lifecycleExec(t, ctx, s, mutation)
		d, e := s.DecideHttpCapability(ctx, "tok", "", input)
		if e == nil && d.Allowed {
			t.Fatal("unsafe onboarding allowed", mutation)
		}
		lifecycleExec(t, ctx, s, "UPDATE applications SET first_party=1,active=1 WHERE id='app-1'")
		lifecycleExec(t, ctx, s, "UPDATE organizations SET inactivated_at=NULL WHERE id='mgmt-org'")
		lifecycleExec(t, ctx, s, "UPDATE provider_pep_clients SET lifecycle='active' WHERE provider_id='grant-management'")
		lifecycleExec(t, ctx, s, "UPDATE service_providers SET active=1 WHERE id='grant-management'")
		lifecycleExec(t, ctx, s, "INSERT OR IGNORE INTO application_actions(application_id,action_id) VALUES('app-1','grant-management.user.materialize')")
	}
	lifecycleExec(t, ctx, s, "UPDATE users SET active=0 WHERE id=?", ensured.UserID)
	if _, err = s.DecideHttpCapability(ctx, "tok", "", input); ErrorStatus(err) != 403 {
		t.Fatal("inactive identity admitted", err)
	}
	if _, err = s.CreateOwnOrganization(ctx, "tok", SelfOrganizationInput{ID: "inactive", Name: "No"}); ErrorStatus(err) != 403 {
		t.Fatal("inactive self create admitted", err)
	}
}

func TestOnboardingCapabilityCannotBeBroadened(t *testing.T) {
	stage, _, err := ManagementCatalog(1, nil)
	if err != nil {
		t.Fatal(err)
	}
	for _, cap := range stage.Capabilities {
		if !managementOnboardingCapability("grant-management", cap) {
			continue
		}
		if managementOnboardingCapability("other", cap) {
			t.Fatal("other provider")
		}
		for name, mutate := range map[string]func(*HttpCapability){
			"path":     func(c *HttpCapability) { c.PathTemplate = "/api/v1/providers" },
			"method":   func(c *HttpCapability) { c.Method = "DELETE" },
			"action":   func(c *HttpCapability) { v := "grant-management.admin.provider.create"; c.ActionID = &v },
			"resource": func(c *HttpCapability) { v := *c.Resource; v.ResourceType = "platform"; c.Resource = &v },
			"auth":     func(c *HttpCapability) { c.Request.Authorization = "none" },
			"body":     func(c *HttpCapability) { c.Request.Body = "opaque" },
			"record":   func(c *HttpCapability) { c.RecordScope = &HttpRecordScopePolicy{} },
		} {
			copy := cap
			mutate(&copy)
			if managementOnboardingCapability("grant-management", copy) {
				t.Fatal(name)
			}
		}
	}
}

func TestTenantCannotReserveSystemRoleIdentifier(t *testing.T) {
	ctx, s := serviceFixture(t)
	for _, id := range []string{"role-system-management-owner", "role-system-anything"} {
		if _, err := s.CreateRoleAuthorized(ctx, "owner", "owner", CreateRoleInput{ID: id, Code: "mine", Name: "Mine"}); ErrorStatus(err) != 400 {
			t.Fatal("reserved role accepted", id, err)
		}
	}
	if _, err := s.CreateRoleAuthorized(ctx, "owner", "owner", CreateRoleInput{ID: "tenant-role", Code: "mine", Name: "Mine"}); err != nil {
		t.Fatal("ordinary role rejected", err)
	}
}
