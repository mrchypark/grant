package grant

import (
	"context"
	"reflect"
	"testing"
)

func firstPartyAdmissionFixture(t *testing.T, ctx context.Context, s *Service) {
	t.Helper()
	for _, sql := range []string{
		`INSERT INTO applications(id,organization_id,slug,name,application_type,app_type,first_party,consent_policy,active,provisioning_status,created_by_user_id,created_at,updated_at) VALUES('admit-app','owner','admit-app','App','confidential','server',1,'auto_approve',1,'ready','owner','2026-01-01','2026-01-01')`,
		`INSERT INTO service_providers(id,name,active,created_at,updated_at) VALUES('admit','Admit',1,'2026-01-01','2026-01-01')`,
		`INSERT INTO actions(id,provider_id,code,name,risk_level,grantable_to_applications,created_at,updated_at) VALUES('admit.read','admit','read','Read','low',1,'2026-01-01','2026-01-01')`,
		`INSERT INTO application_actions(application_id,action_id) VALUES('admit-app','organization.profile.read')`,
		`INSERT INTO application_grants(id,user_id,application_id,organization_id,status,consented_at,created_at,updated_at) VALUES('admit-grant','owner','admit-app','owner','revoked','2026-01-01','2026-01-01','2026-01-01')`,
		`INSERT INTO application_grant_actions(application_grant_id,action_id) VALUES('admit-grant','organization.profile.read')`,
	} {
		if _, err := s.store.Exec(ctx, sql); err != nil {
			t.Fatal(err)
		}
	}
}

func TestFirstPartyAdmissionPreservesState(t *testing.T) {
	ctx, s := serviceFixture(t)
	firstPartyAdmissionFixture(t, ctx, s)
	tables := []string{"applications", "application_grants", "application_grant_actions", "user_identities", "role_actions", "role_bindings"}
	before := map[string]any{}
	for _, table := range tables {
		rows, err := s.store.Query(ctx, ReadStrong, "SELECT * FROM "+table)
		if err != nil {
			t.Fatal(err)
		}
		before[table] = rows
	}
	for i := 0; i < 2; i++ {
		if err := s.AdmitFirstPartyApplicationActionAdmin(ctx, "owner", "admit-app", "admit.read"); err != nil {
			t.Fatal(err)
		}
	}
	for _, table := range tables {
		rows, err := s.store.Query(ctx, ReadStrong, "SELECT * FROM "+table)
		if err != nil || !reflect.DeepEqual(before[table], rows) {
			t.Fatalf("changed %s: %v", table, err)
		}
	}
	rows, err := s.store.ListApplicationActionsWithConsistency(ctx, "admit-app", ReadStrong)
	if err != nil || len(rows) != 2 {
		t.Fatal(rows, err)
	}
	if err := s.SetApplicationAction(ctx, "owner", "owner", "admit-app", "admit.read", true); ErrorStatus(err) != 403 {
		t.Fatalf("tenant individual mutation: %v", err)
	}
	if _, err := s.ReplaceApplicationActionsAuthorized(ctx, "owner", "owner", "admit-app", []string{"admit.read"}); ErrorStatus(err) != 403 {
		t.Fatalf("tenant replacement: %v", err)
	}
}

func TestFirstPartyAdmissionRejectsInvalidState(t *testing.T) {
	for name, sql := range map[string]string{
		"third-party":       `UPDATE applications SET first_party=0 WHERE id='admit-app'`,
		"explicit":          `UPDATE applications SET consent_policy='explicit' WHERE id='admit-app'`,
		"disabled":          `UPDATE applications SET active=0 WHERE id='admit-app'`,
		"unprovisioned":     `UPDATE applications SET provisioning_status='unprovisioned' WHERE id='admit-app'`,
		"inactive-org":      `UPDATE organizations SET inactivated_at='2026-01-01' WHERE id='owner'`,
		"inactive-provider": `UPDATE service_providers SET active=0 WHERE id='admit'`,
		"ungrantable":       `UPDATE actions SET grantable_to_applications=0 WHERE id='admit.read'`,
		"noncanonical":      `UPDATE actions SET code='other' WHERE id='admit.read'`,
		"unknown-action":    `DELETE FROM actions WHERE id='admit.read'`,
		"lease":             `UPDATE applications SET security_mutation_token='busy',security_mutation_expires_at='2999-01-01' WHERE id='admit-app'`,
		"wrong-org":         `UPDATE applications SET organization_id='absent' WHERE id='admit-app'`,
	} {
		t.Run(name, func(t *testing.T) {
			ctx, s := serviceFixture(t)
			firstPartyAdmissionFixture(t, ctx, s)
			if name == "wrong-org" {
				if err := s.AdmitFirstPartyApplicationActionAdmin(ctx, "absent", "admit-app", "admit.read"); ErrorStatus(err) != 409 {
					t.Fatal(err)
				}
				return
			}
			if _, err := s.store.Exec(ctx, sql); err != nil {
				t.Fatal(err)
			}
			if err := s.AdmitFirstPartyApplicationActionAdmin(ctx, "owner", "admit-app", "admit.read"); ErrorStatus(err) != 409 {
				t.Fatalf("expected 409: %v", err)
			}
			rows, err := s.store.Query(ctx, ReadStrong, `SELECT * FROM application_actions WHERE application_id='admit-app' AND action_id='admit.read'`)
			if err != nil || len(rows) != 0 {
				t.Fatal(rows, err)
			}
		})
	}
}

func TestFirstPartyAdmissionRechecksConcurrentPolicy(t *testing.T) {
	for name, sql := range map[string]string{
		"trust":  `UPDATE applications SET first_party=0 WHERE id='admit-app'`,
		"action": `UPDATE actions SET grantable_to_applications=0 WHERE id='admit.read'`,
	} {
		t.Run(name, func(t *testing.T) {
			ctx, local, remote, gate := actorRecheckFixture(t)
			firstPartyAdmissionFixture(t, ctx, local)
			gate.arm()
			defer gate.release()
			done := make(chan error, 1)
			go func() { done <- remote.AdmitFirstPartyApplicationActionAdmin(ctx, "owner", "admit-app", "admit.read") }()
			select {
			case <-gate.reached:
			case <-ctx.Done():
				t.Fatal(ctx.Err())
			}
			if _, err := local.store.Exec(ctx, sql); err != nil {
				t.Fatal(err)
			}
			gate.release()
			if err := <-done; ErrorStatus(err) != 409 {
				t.Fatalf("stale policy accepted: %v", err)
			}
			rows, err := local.store.Query(ctx, ReadStrong, `SELECT * FROM application_actions WHERE application_id='admit-app' AND action_id='admit.read'`)
			if err != nil || len(rows) != 0 {
				t.Fatal(rows, err)
			}
		})
	}
}
