package grant

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
)

func responseFixture(t *testing.T) (StageHttpCapabilityGeneration, HttpCapability, HttpCapabilityDecisionInput) {
	t.Helper()
	stage, _, byID := batchFixture(t)
	cap := byID["permissions.get.0"]
	child := batchEvidenceChild(cap, 0, "ccccccccccccccc")
	input := batchChildInput(batchEvidenceInput(t, stage, cap, nil), child)
	input.CanonicalQuery = "expand=plant"
	input.CanonicalQueryDigest = sha256Text(input.CanonicalQuery)
	parent := uint64(0)
	input.ResponseReferences = []HttpResponseReference{{ResourceKey: "permissions", ID: "ccccccccccccccc"}, {ResourceKey: "plants", ID: "ddddddddddddddd", ParentIndex: &parent, RelationField: "plant"}}
	input.ResponseDigest = sha256Text("response")
	input.ResponseLength = 8
	input.ResponseReferencesDigest, _ = HttpResponseReferencesDigest(input.ResponseReferences)
	return stage, cap, input
}

func testResponseDecisionInGrant(t *testing.T, ctx context.Context, svc *Service, stage StageHttpCapabilityGeneration) {
	t.Helper()
	_, _, input := responseFixture(t)
	check := func(in HttpCapabilityDecisionInput, want bool) {
		t.Helper()
		got, err := svc.DecideHttpCapability(ctx, "token", "conalog-grant", in)
		if err != nil || got.Allowed != want || got.ResponseDigest != in.ResponseDigest || got.ResponseReferencesDigest != in.ResponseReferencesDigest || got.ResponseLength != in.ResponseLength {
			t.Fatal("response decision", want, got, err)
		}
	}
	initial := input
	initial.ResponseDigest = ""
	initial.ResponseLength = 0
	initial.ResponseReferences = nil
	initial.ResponseReferencesDigest = ""
	check(initial, false)
	bind := func(id, kind, resource string) {
		t.Helper()
		if _, err := svc.store.Exec(ctx, "INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,state,created_by_user_id,created_at,updated_at) VALUES(?,'user','conalog-grant','conalog-grant','patch-data',?,?,'role-system-bootstrap-owner','active','conalog-grant','now','now')", id, kind, resource); err != nil {
			t.Fatal(err)
		}
	}
	bind("response-root", "permissions", "ccccccccccccccc")
	check(initial, true)
	check(input, false)
	bind("response-related", "plant", "ddddddddddddddd")
	check(input, true)
	if _, err := svc.store.Exec(ctx, "UPDATE role_bindings SET state='revoked' WHERE id='response-related'"); err != nil {
		t.Fatal(err)
	}
	check(input, false)
	if _, err := svc.store.Exec(ctx, "UPDATE role_bindings SET state='revoked' WHERE id='response-root'"); err != nil {
		t.Fatal(err)
	}
	check(input, false)
	var list HttpCapability
	for _, cap := range stage.Capabilities {
		if cap.CapabilityID == "permissions.list.0" {
			list = cap
		}
	}
	empty := batchChildInput(batchEvidenceInput(t, stage, list, nil), batchEvidenceChild(list, 0, ""))
	empty.CanonicalQuery = "expand=plant"
	empty.CanonicalQueryDigest = sha256Text(empty.CanonicalQuery)
	empty.ResponseDigest = sha256Text(`{"items":[]}`)
	empty.ResponseLength = 12
	empty.ResponseReferences = []HttpResponseReference{}
	empty.ResponseReferencesDigest, _ = HttpResponseReferencesDigest(empty.ResponseReferences)
	check(empty, true)
	if _, err := svc.store.Exec(ctx, "DELETE FROM application_actions WHERE application_id='batch-app' AND action_id='patch-data.permissions.read'"); err != nil {
		t.Fatal(err)
	}
	if _, err := svc.DecideHttpCapability(ctx, "token", "conalog-grant", empty); ErrorStatus(err) != 403 {
		t.Fatal("empty list lost admission check", err)
	}
}

func TestResponseReferencesFollowRequestedGraph(t *testing.T) {
	stage, cap, input := responseFixture(t)
	if checks, err := validateResponseEvidence(cap, stage.ResponseResources, input); err != nil || len(checks) != 2 {
		t.Fatal(checks, err)
	}
	for name, change := range map[string]func(*HttpCapabilityDecisionInput){
		"wrong-node":         func(in *HttpCapabilityDecisionInput) { in.ResponseReferences[1].ResourceKey = "organizations" },
		"wrong-edge":         func(in *HttpCapabilityDecisionInput) { in.ResponseReferences[1].RelationField = "users" },
		"wrong-root":         func(in *HttpCapabilityDecisionInput) { in.ResponseReferences[0].ID = "eeeeeeeeeeeeeee" },
		"forward-parent":     func(in *HttpCapabilityDecisionInput) { p := uint64(2); in.ResponseReferences[1].ParentIndex = &p },
		"missing-root":       func(in *HttpCapabilityDecisionInput) { in.ResponseReferences = nil },
		"oversized-response": func(in *HttpCapabilityDecisionInput) { in.ResponseLength = cap.Response.MaxBytes + 1 },
		"wrong-query": func(in *HttpCapabilityDecisionInput) {
			in.CanonicalQuery = "expand=users"
			in.CanonicalQueryDigest = sha256Text(in.CanonicalQuery)
		},
	} {
		t.Run(name, func(t *testing.T) {
			in := input
			in.ResponseReferences = append([]HttpResponseReference(nil), input.ResponseReferences...)
			change(&in)
			in.ResponseReferencesDigest, _ = HttpResponseReferencesDigest(in.ResponseReferences)
			if _, err := validateResponseEvidence(cap, stage.ResponseResources, in); err == nil {
				t.Fatal("invalid response evidence accepted")
			}
		})
	}
	input.ResponseReferencesDigest = sha256Text("tamper")
	if _, err := validateResponseEvidence(cap, stage.ResponseResources, input); err == nil {
		t.Fatal("unbound response refs")
	}
}

func TestResponseGraphLimitsAndDigestCompatibility(t *testing.T) {
	stage, cap, input := responseFixture(t)
	for _, query := range []string{"expand=unknown", "expand=plant%2Cplant", "expand=plant.organization.parent.parent.parent.parent.parent"} {
		if _, err := responseExpansionPaths(cap, stage.ResponseResources, query); err == nil {
			t.Fatal("invalid expansion accepted", query)
		}
	}
	input.ResponseReferences = make([]HttpResponseReference, 201)
	input.ResponseReferencesDigest, _ = HttpResponseReferencesDigest(input.ResponseReferences)
	if _, err := validateResponseEvidence(cap, stage.ResponseResources, input); err == nil {
		t.Fatal("too many response occurrences")
	}
	empty, _ := HttpResponseReferencesDigest(nil)
	array, _ := HttpResponseReferencesDigest([]HttpResponseReference{})
	if empty != array {
		t.Fatal("empty-list wire digest mismatch")
	}
	stage.ResponseResources = nil
	for i := range stage.Capabilities {
		stage.Capabilities[i].ResponseAuthorization = nil
	}
	raw, err := generationJSON("patch-data", stage)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(raw, `"responseResources"`) {
		t.Fatal("nil graph changed legacy digest projection")
	}
	var parsed map[string]json.RawMessage
	if err = json.Unmarshal([]byte(raw), &parsed); err != nil {
		t.Fatal(err)
	}
	if len(parsed) != 4 {
		t.Fatal("legacy generation shape", len(parsed))
	}
}
