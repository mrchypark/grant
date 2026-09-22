package grant

import (
	"strings"
	"testing"
	"time"
)

func TestAgreementTimeEvaluationUsesChronologyAndFailsClosed(t *testing.T) {
	ctx, s := serviceFixture(t)
	if _, err := s.CreateOrganizationAdmin(ctx, CreateOrganizationInput{ID: "receiver", Name: "Receiver", OwnerUserID: "owner"}); err != nil {
		t.Fatal(err)
	}
	if _, err := s.CreateRoleAuthorized(ctx, "owner", "receiver", CreateRoleInput{ID: "time-role", Code: "time", Name: "Time"}); err != nil {
		t.Fatal(err)
	}
	if _, err := s.ReplaceRoleActionsAuthorized(ctx, "owner", "receiver", "time-role", []string{"organization.profile.read"}); err != nil {
		t.Fatal(err)
	}
	if _, err := s.CreateAgreementAuthorized(ctx, "owner", "owner", UpsertAgreementInput{ID: "time-agreement", Title: "Time", AgreementType: "delegation", Status: "active", GrantorOrganizationID: "owner", GranteeOrganizationID: "receiver"}); err != nil {
		t.Fatal(err)
	}
	agreement := "time-agreement"
	if _, err := s.CreateRoleBindingAuthorized(ctx, "owner", "receiver", CreateRoleBindingInput{ID: "time-binding", SubjectType: "organization", SubjectID: "receiver", SubjectOrganizationID: "receiver", ProviderID: "grant", ResourceType: "organization", ResourceID: "receiver", RoleID: "time-role", AgreementID: &agreement, State: "active"}); err != nil {
		t.Fatal(err)
	}
	lifecycleExec(t, ctx, s, "INSERT INTO users(id,display_name,active,created_at) VALUES('time-user','Time user',1,?)", nowString())
	if _, err := s.EnrollOrganizationMemberAuthorized(ctx, "owner", "receiver", "time-user"); err != nil {
		t.Fatal(err)
	}
	if _, err := s.CreateGroupAuthorized(ctx, "owner", "receiver", UpsertGroupInput{ID: "time-group", Name: "Time group"}); err != nil {
		t.Fatal(err)
	}
	if _, err := s.AddGroupMemberAuthorized(ctx, "owner", "receiver", "time-group", "time-user"); err != nil {
		t.Fatal(err)
	}
	query := strings.Replace(authorizationSQL, "{values}", "(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", 1)
	check := func(at string, want bool) {
		t.Helper()
		parsed, err := time.Parse(time.RFC3339Nano, at)
		if err != nil {
			t.Fatal(err)
		}
		rows, err := s.store.Query(ctx, ReadStrong, query, int64(0), "time-user", "receiver", "grant", "organization", "receiver", "organization.profile.read", nil, parsed.UTC().Format(agreementTimeLayout), false)
		if err != nil || len(rows) != 1 {
			t.Fatal(rows, err)
		}
		if got := rowInt(rows[0], "allowed") != 0; got != want {
			t.Fatalf("allowed=%v want=%v", got, want)
		}
	}
	cases := []struct {
		name, start, end, now string
		allowed               bool
	}{
		{"past whole second end", "2029-01-01T00:00:00.000000000Z", "2030-01-01T12:00:00.000000000Z", "2030-01-01T12:00:00.5Z", false},
		{"legacy offset denies", "2030-01-01T20:00:00+09:00", "2030-01-01T22:00:00+09:00", "2030-01-01T12:00:00.5Z", false},
		{"legacy variable precision denies", "2029-01-01T00:00:00Z", "2031-01-01T00:00:00Z", "2030-01-01T12:00:00Z", false},
		{"malformed legacy end", "2029-01-01T00:00:00.000000000Z", "never", "2030-01-01T12:00:00Z", false},
		{"invalid calendar date", "2029-02-29T00:00:00.000000000Z", "2031-01-01T00:00:00.000000000Z", "2030-01-01T12:00:00Z", false},
		{"invalid month", "2029-13-01T00:00:00.000000000Z", "2031-01-01T00:00:00.000000000Z", "2030-01-01T12:00:00Z", false},
		{"invalid time", "2029-01-01T25:00:00.000000000Z", "2031-01-01T00:00:00.000000000Z", "2030-01-01T12:00:00Z", false},
		{"nanosecond after end", "2029-01-01T00:00:00.000000000Z", "2030-01-01T12:00:00.000000004Z", "2030-01-01T12:00:00.000000005Z", false},
		{"nanosecond before start", "2030-01-01T12:00:00.000000004Z", "2031-01-01T00:00:00.000000000Z", "2030-01-01T12:00:00.000000003Z", false},
		{"exact start inclusive", "2030-01-01T12:00:00.000000000Z", "2031-01-01T00:00:00.000000000Z", "2030-01-01T12:00:00Z", true},
		{"exact end inclusive", "2029-01-01T00:00:00.000000000Z", "2030-01-01T12:00:00.000000000Z", "2030-01-01T12:00:00Z", true},
	}
	for _, subject := range []struct{ kind, id string }{{"organization", "receiver"}, {"user", "time-user"}, {"group", "time-group"}} {
		t.Run(subject.kind, func(t *testing.T) {
			lifecycleExec(t, ctx, s, "UPDATE role_bindings SET subject_type=?,subject_id=?,agreement_id=? WHERE id='time-binding'", subject.kind, subject.id, agreement)
			for _, tc := range cases {
				t.Run(tc.name, func(t *testing.T) {
					lifecycleExec(t, ctx, s, "UPDATE agreements SET status='active',starts_at=?,ends_at=? WHERE id=?", tc.start, tc.end, agreement)
					check(tc.now, tc.allowed)
				})
			}
			lifecycleExec(t, ctx, s, "UPDATE agreements SET status='terminated',starts_at=NULL,ends_at=NULL WHERE id=?", agreement)
			check("2030-01-01T12:00:00Z", false)
			lifecycleExec(t, ctx, s, "UPDATE role_bindings SET agreement_id='missing' WHERE id='time-binding'")
			check("2030-01-01T12:00:00Z", false)
			lifecycleExec(t, ctx, s, "UPDATE role_bindings SET agreement_id=NULL WHERE id='time-binding'")
			check("2030-01-01T12:00:00Z", true)
		})
	}
	// Valid legacy offsets remain denied until an authorized grantor explicitly normalizes them.
	lifecycleExec(t, ctx, s, "UPDATE role_bindings SET agreement_id=? WHERE id='time-binding'", agreement)
	start, end := "2030-01-01T20:00:00+09:00", "2030-01-01T22:00:00+09:00"
	lifecycleExec(t, ctx, s, "UPDATE agreements SET status='active',starts_at=?,ends_at=? WHERE id=?", start, end, agreement)
	check("2030-01-01T12:00:00Z", false)
	for _, legacy := range []struct{ start, end string }{
		{start, end},
		{"2030-01-01T11:00:00Z", "2030-01-01T13:00:00.5Z"},
	} {
		lifecycleExec(t, ctx, s, "UPDATE agreements SET starts_at=?,ends_at=? WHERE id=?", legacy.start, legacy.end, agreement)
		in := UpsertAgreementInput{ID: agreement, Title: "Grantee metadata", AgreementType: "delegation", Status: "active", GrantorOrganizationID: "owner", GranteeOrganizationID: "receiver", StartsAt: &legacy.start, EndsAt: &legacy.end}
		if _, err := s.UpdateAgreementAuthorized(ctx, "owner", "receiver", agreement, in); ErrorStatus(err) != 403 {
			t.Fatal("grantee PUT normalized legacy authorization bounds", err)
		}
		check("2030-01-01T12:00:00Z", false)
		if _, err := s.PatchAgreementAuthorized(ctx, "owner", "receiver", agreement, PatchAgreementInput{Title: &in.Title}); err != nil {
			t.Fatal("grantee metadata PATCH rejected", err)
		}
		got, err := s.GetAgreementAuthorized(ctx, "owner", "receiver", agreement, ReadStrong)
		if err != nil || ptrValue(got.StartsAt) != legacy.start || ptrValue(got.EndsAt) != legacy.end {
			t.Fatal("metadata PATCH changed authorization bounds", got, err)
		}
		check("2030-01-01T12:00:00Z", false)
	}
	if _, err := s.PatchAgreementAuthorized(ctx, "owner", "owner", agreement, PatchAgreementInput{StartsAt: PatchText{Present: true, Value: &start}, EndsAt: PatchText{Present: true, Value: &end}}); err != nil {
		t.Fatal(err)
	}
	check("2030-01-01T12:00:00Z", true)
	// The insertion guard rejects noncanonical/expired dates through the same CTE.
	lifecycleExec(t, ctx, s, "DELETE FROM role_bindings WHERE id='time-binding'")
	binding := CreateRoleBindingInput{ID: "time-binding", SubjectType: "organization", SubjectID: "receiver", SubjectOrganizationID: "receiver", ProviderID: "grant", ResourceType: "organization", ResourceID: "receiver", RoleID: "time-role", AgreementID: &agreement, State: "active"}
	for _, bound := range []string{"never", "2000-01-01T12:00:00.000000000Z", "9999-01-01T00:00:00Z"} {
		lifecycleExec(t, ctx, s, "UPDATE agreements SET starts_at=NULL,ends_at=? WHERE id=?", bound, agreement)
		if _, err := s.CreateRoleBindingAuthorized(ctx, "owner", "receiver", binding); err == nil {
			t.Fatalf("binding accepted invalid interval %q", bound)
		}
	}
	start = time.Now().Add(-time.Hour).UTC().Format(agreementTimeLayout)
	end = time.Now().Add(time.Hour).UTC().Format(agreementTimeLayout)
	lifecycleExec(t, ctx, s, "UPDATE agreements SET starts_at=?,ends_at=? WHERE id=?", start, end, agreement)
	if _, err := s.CreateRoleBindingAuthorized(ctx, "owner", "receiver", binding); err != nil {
		t.Fatal("binding rejected canonical interior", err)
	}
}
