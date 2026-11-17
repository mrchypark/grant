package grant

import (
	"context"
	"slices"
	"testing"
)

func TestLookupResourcesAndLearnerAuthz(t *testing.T) {
	f := newUserAuthzFixture(t)
	ctx := context.Background()

	// Seed additional resources across user, group, organization, owner, agreement, and consent paths.
	for _, sql := range []string{
		"INSERT INTO organizations(id,name,inactivated_at,created_at,updated_at) VALUES('org-inactive','Inactive','2026-01-01T00:00:00Z','now','now')",
		"INSERT INTO groups(id,organization_id,name,created_at,updated_at) VALUES('grp-1','conalog-grant','Group 1','now','now')",
		"INSERT INTO group_memberships(group_id,organization_membership_id) VALUES('grp-1','user-member')",
		"INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,state,created_by_user_id,created_at,updated_at) VALUES" +
			"('rb-user-2','user','user','conalog-grant','compos','object_type','collection-3','reader','active','conalog-grant','now','now')," +
			"('rb-group-1','group','grp-1','conalog-grant','compos','object_type','collection-2','reader','active','conalog-grant','now','now')," +
			"('rb-org-1','organization','conalog-grant','conalog-grant','compos','object_type','collection-4','reader','active','conalog-grant','now','now')," +
			"('rb-revoked','user','user','conalog-grant','compos','object_type','collection-revoked','reader','revoked','conalog-grant','now','now')",
		// Agreement-backed resource binding.
		"INSERT INTO agreements(id,agreement_type,grantor_organization_id,grantee_organization_id,status,title,starts_at,ends_at,created_at,updated_at) VALUES" +
			"('agr-active','delegation','conalog-grant','conalog-grant','active','Active','2020-01-01T00:00:00.000000000Z','2099-01-01T00:00:00.000000000Z','now','now')," +
			"('agr-expired','delegation','conalog-grant','conalog-grant','active','Expired','2020-01-01T00:00:00.000000000Z','2021-01-01T00:00:00.000000000Z','now','now')",
		"INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,agreement_id,state,created_by_user_id,created_at,updated_at) VALUES" +
			"('rb-agr-active','user','user','conalog-grant','compos','object_type','collection-5','reader','agr-active','active','conalog-grant','now','now')," +
			"('rb-agr-expired','user','user','conalog-grant','compos','object_type','collection-expired','reader','agr-expired','active','conalog-grant','now','now')",
		// Synthetic owner branch on grant provider.
		"INSERT INTO users(id,display_name,active,created_at) VALUES('owner-user','Owner',1,'now')",
		"INSERT INTO organization_memberships(id,organization_id,user_id,membership_role,status,created_at,updated_at) VALUES('owner-member','conalog-grant','owner-user','owner','active','now','now')",
	} {
		f.exec(t, sql)
	}

	sub := AuthorizationSubject{SubjectType: "user", ID: "user", OrganizationID: "conalog-grant"}

	// 1. Paginated LookupResources on voter and learner (collection-1..5).
	limit2 := 2
	page1, err := f.voter.LookupResources(ctx, sub, "compos", "object_type", "compos.read", &limit2, nil, ReadStrong)
	if err != nil {
		t.Fatalf("voter LookupResources page1: %v", err)
	}
	if !slices.Equal(page1.ResourceIDs, []string{"collection-1", "collection-2"}) || page1.NextCursor != "collection-2" {
		t.Fatalf("unexpected page1: %+v", page1)
	}

	fencesBefore := f.fences.Load()
	page2, err := f.learner.LookupResources(ctx, sub, "compos", "object_type", "compos.read", &limit2, &page1.NextCursor, ReadLocal)
	if err != nil {
		t.Fatalf("learner LookupResources page2: %v", err)
	}
	if f.fences.Load() <= fencesBefore {
		t.Fatal("expected learner LookupResources to obtain readFence")
	}
	if !slices.Equal(page2.ResourceIDs, []string{"collection-3", "collection-4"}) || page2.NextCursor != "collection-4" {
		t.Fatalf("unexpected page2: %+v", page2)
	}

	page3, err := f.learner.LookupResources(ctx, sub, "compos", "object_type", "compos.read", &limit2, &page2.NextCursor, ReadLocal)
	if err != nil {
		t.Fatalf("learner LookupResources page3: %v", err)
	}
	if !slices.Equal(page3.ResourceIDs, []string{"collection-5"}) || page3.NextCursor != "" {
		t.Fatalf("unexpected page3: %+v", page3)
	}

	// 2. Limit validation bounds.
	zero := 0
	if _, err := f.voter.LookupResources(ctx, sub, "compos", "object_type", "compos.read", &zero, nil, ReadLocal); ErrorStatus(err) != 400 {
		t.Fatalf("limit=0 status=%d, want 400", ErrorStatus(err))
	}
	over := 1001
	if _, err := f.voter.LookupResources(ctx, sub, "compos", "object_type", "compos.read", &over, nil, ReadLocal); ErrorStatus(err) != 400 {
		t.Fatalf("limit=1001 status=%d, want 400", ErrorStatus(err))
	}

	// 3. Synthetic owner branch on grant provider.
	ownerSub := AuthorizationSubject{SubjectType: "user", ID: "owner-user", OrganizationID: "conalog-grant"}
	ownerRes, err := f.learner.LookupResources(ctx, ownerSub, "grant", "organization", "organization.member.manage", nil, nil, ReadLocal)
	if err != nil {
		t.Fatalf("owner LookupResources: %v", err)
	}
	if !slices.Equal(ownerRes.ResourceIDs, []string{"conalog-grant"}) {
		t.Fatalf("unexpected owner resource_ids: %+v", ownerRes)
	}

	// 4. Master CheckAuthorizations (/api/v1/authz/*) works on learner with readFence.
	fencesBefore = f.fences.Load()
	decisions, err := f.learner.CheckAuthorizations(ctx, []AuthorizationCheck{
		{Subject: sub, Action: "compos.read", Resource: AuthorizationResource{ProviderID: "compos", ResourceType: "object_type", ID: "collection-2"}},
		{Subject: sub, Action: "compos.read", Resource: AuthorizationResource{ProviderID: "compos", ResourceType: "object_type", ID: "collection-expired"}},
	}, ReadLocal)
	if err != nil {
		t.Fatalf("learner CheckAuthorizations: %v", err)
	}
	if f.fences.Load() <= fencesBefore {
		t.Fatal("expected learner CheckAuthorizations to obtain readFence")
	}
	if len(decisions) != 2 || !decisions[0].Allowed || decisions[1].Allowed {
		t.Fatalf("unexpected learner CheckAuthorizations decisions: %+v", decisions)
	}

	// 5. UserLookupResources (v2) on voter and learner, including action validation.
	userRes, err := f.learner.UserLookupResources(ctx, "Bearer opaque-token", "conalog-grant", "compos", "object_type", "compos.read", &limit2, nil)
	if err != nil {
		t.Fatalf("learner UserLookupResources: %v", err)
	}
	if !slices.Equal(userRes.ResourceIDs, []string{"collection-1", "collection-2"}) || userRes.NextCursor != "collection-2" || userRes.PEPPolicyRevision <= 0 {
		t.Fatalf("unexpected UserLookupResources result: %+v", userRes)
	}
	if _, err := f.voter.UserLookupResources(ctx, "Bearer opaque-token", "conalog-grant", "compos", "object_type", "compos.missing", nil, nil); ErrorStatus(err) != 404 {
		t.Fatalf("unknown action status=%d, want 404", ErrorStatus(err))
	}
	if _, err := f.voter.UserLookupResources(ctx, "Bearer opaque-token", "conalog-grant", "compos", "object_type", "other.read", nil, nil); ErrorStatus(err) != 400 {
		t.Fatalf("mismatched action provider status=%d, want 400", ErrorStatus(err))
	}

	// 6. Application consent gate via store.LookupResources and LookupApplicationResources.
	for _, sql := range []string{
		"UPDATE actions SET grantable_to_applications=1 WHERE id='compos.read'",
		"INSERT INTO applications(id,organization_id,slug,name,application_type,app_type,first_party,consent_policy,active,provisioning_status,rauthy_client_id,created_by_user_id,created_at,updated_at) VALUES" +
			"('app-trusted','conalog-grant','app-trusted','Trusted App','web','first_party',1,'auto_approve',1,'ready','login','conalog-grant','now','now')," +
			"('app-untrusted','conalog-grant','app-untrusted','Untrusted App','web','third_party',0,'explicit_consent',1,'ready','untrusted-client','conalog-grant','now','now')",
		"INSERT INTO application_actions(application_id,action_id) VALUES('app-trusted','compos.read'),('app-untrusted','compos.read')",
		"INSERT INTO application_grants(id,user_id,application_id,organization_id,status,consented_at,created_at,updated_at) VALUES" +
			"('grant-1','user','app-untrusted','conalog-grant','active','now','now','now')",
		"INSERT INTO application_grant_actions(application_grant_id,action_id) VALUES('grant-1','compos.read')",
	} {
		f.exec(t, sql)
	}
	f.learner.cfg.Auth.Goauthy.ApplicationID = "app-trusted"
	appTrustedRes, err := f.learner.LookupApplicationResources(ctx, "Bearer opaque-token", "conalog-grant", "compos", "object_type", "compos.read", nil, nil)
	if err != nil {
		t.Fatalf("learner LookupApplicationResources: %v", err)
	}
	if !slices.Equal(appTrustedRes.ResourceIDs, []string{"collection-1", "collection-2", "collection-3", "collection-4", "collection-5"}) {
		t.Fatalf("unexpected trusted app LookupApplicationResources: %+v", appTrustedRes)
	}
	// Untrusted app without grant is denied; with active grant + application_grant_actions it is allowed.
	appUntrustedRes, err := f.voter.store.LookupResources(ctx, LookupResourcesInput{
		UserID:         "user",
		OrganizationID: "conalog-grant",
		ProviderID:     "compos",
		ResourceType:   "object_type",
		ActionID:       "compos.read",
		ApplicationID:  "app-untrusted",
		Consistency:    ReadStrong,
	})
	if err != nil {
		t.Fatalf("untrusted app store LookupResources: %v", err)
	}
	if !slices.Equal(appUntrustedRes.ResourceIDs, []string{"collection-1", "collection-2", "collection-3", "collection-4", "collection-5"}) {
		t.Fatalf("unexpected untrusted app LookupResources with grant: %+v", appUntrustedRes)
	}
	f.exec(t, "DELETE FROM application_grant_actions WHERE application_grant_id='grant-1'")
	appRevokedRes, err := f.voter.store.LookupResources(ctx, LookupResourcesInput{
		UserID:         "user",
		OrganizationID: "conalog-grant",
		ProviderID:     "compos",
		ResourceType:   "object_type",
		ActionID:       "compos.read",
		ApplicationID:  "app-untrusted",
		Consistency:    ReadStrong,
	})
	if err != nil {
		t.Fatalf("untrusted app store LookupResources without grant action: %v", err)
	}
	if len(appRevokedRes.ResourceIDs) != 0 {
		t.Fatalf("expected empty resource_ids without consent grant action, got %+v", appRevokedRes)
	}
}
