package grant

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestIndependentObjectDRHelper(t *testing.T) {
	if os.Getenv("GRANT_INDEPENDENT_HELPER") != "1" {
		t.Skip("integration helper; use scripts/verify-independent-object-dr.sh")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 45*time.Second)
	defer cancel()
	cfg, e := ConfigFromEnv()
	if e != nil {
		t.Fatal(e)
	}
	root := os.Getenv("GRANT_INDEPENDENT_HELPER_ROOT")
	if !filepath.IsAbs(root) || cfg.Database.DataDir != filepath.Join(root, "data") || !cfg.Database.Rhiza.Local || !strings.HasPrefix(cfg.Database.NodeID, "grant-") {
		t.Fatal("helper requires the owned Local fixture")
	}
	marker, e := os.ReadFile(filepath.Join(root, "owner"))
	if e != nil || string(marker) != cfg.Database.NodeID {
		t.Fatal("helper ownership marker mismatch")
	}
	switch os.Getenv("OWNED_PHASE") {
	case "seed", "verify", "verify-restored":
	default:
		t.Fatal("unknown authority phase")
	}
	t.Logf("OPEN phase=%s node=%s utc=%s", os.Getenv("OWNED_PHASE"), cfg.Database.NodeID, time.Now().UTC().Format(time.RFC3339Nano))
	st, e := OpenStore(ctx, cfg)
	t.Logf("OPEN RESULT phase=%s utc=%s error=%v", os.Getenv("OWNED_PHASE"), time.Now().UTC().Format(time.RFC3339Nano), e)
	if e != nil {
		t.Fatal(e)
	}
	defer func() {
		e := st.Close()
		t.Logf("CLOSE phase=%s utc=%s error=%v", os.Getenv("OWNED_PHASE"), time.Now().UTC().Format(time.RFC3339Nano), e)
		if e != nil {
			t.Error(e)
		}
	}()
	s := NewService(cfg, st)
	if cfg.Database.Rhiza.ObjStoreProvider == "gcs" && os.Getenv("GRANT_INDEPENDENT_GCS_SCALE") == "1" && os.Getenv("OWNED_PHASE") != "seed" {
		for i := 0; i < 6; i++ {
			provider := fmt.Sprintf("%s-provider-%d", cfg.Database.NodeID, i)
			pep := fmt.Sprintf("%s-pep-%d", cfg.Database.NodeID, i)
			rows, err := st.Query(ctx, ReadStrong, "SELECT id,active FROM service_providers WHERE id=? AND active=1", provider)
			if err != nil || len(rows) != 1 || rowString(rows[0], "id") != provider {
				t.Fatalf("synthetic provider %s missing: %v %v", provider, rows, err)
			}
			binding, err := st.ProviderPepClient(ctx, provider, pep)
			if err != nil || binding.ProviderID != provider || binding.PepClientID != pep || binding.Lifecycle != "active" {
				t.Fatalf("synthetic active PEP %s lost: %+v %v", pep, binding, err)
			}
			t.Logf("READBACK utc=%s provider=%s active=true pep=%s lifecycle=%s", time.Now().UTC().Format(time.RFC3339Nano), provider, binding.PepClientID, binding.Lifecycle)
		}
		t.Log("SCALE ORACLE PASS six synthetic providers and six active PEPs")
	}
	if os.Getenv("OWNED_PHASE") == "seed" {
		ownershipMember(t, ctx, s, "next")
		if _, e = s.CreateRoleAuthorized(ctx, "owner", "owner", CreateRoleInput{ID: "reader", Code: "reader", Name: "Reader"}); e != nil {
			t.Fatal(e)
		}
		if _, e = s.ReplaceRoleActionsAuthorized(ctx, "owner", "owner", "reader", []string{"organization.profile.read"}); e != nil {
			t.Fatal(e)
		}
		if _, e = s.CreateRoleBindingAuthorized(ctx, "owner", "owner", CreateRoleBindingInput{ID: "dr-reader", SubjectType: "user", SubjectID: "owner", SubjectOrganizationID: "owner", ProviderID: "grant", ResourceType: "organization", ResourceID: "owner", RoleID: "reader", State: "active"}); e != nil {
			t.Fatal(e)
		}
		before, e := s.GetOrganizationOwnerAuthorized(ctx, "owner", "owner")
		if e != nil {
			t.Fatal(e)
		}
		after, e := s.TransferOrganizationOwnerAuthorized(ctx, "owner", "owner", "next", before.ETag)
		if e != nil {
			t.Fatal(e)
		}
		if e = os.WriteFile(os.Getenv("OWNED_EXPECTED"), []byte(after.ETag), 0600); e != nil {
			t.Fatal(e)
		}
	}
	want, e := os.ReadFile(os.Getenv("OWNED_EXPECTED"))
	if e != nil {
		t.Fatal(e)
	}
	owner, e := s.GetOrganizationOwnerAuthorized(ctx, "next", "owner")
	if e != nil || owner.OwnerUserID != "next" || owner.ETag != string(want) {
		t.Fatalf("owner/revision %+v %v", owner, e)
	}
	rows, e := st.Query(ctx, ReadStrong, "SELECT action_id FROM role_actions WHERE role_id='reader'")
	if e != nil || len(rows) != 1 || rowString(rows[0], "action_id") != "organization.profile.read" {
		t.Fatalf("role actions %+v %v", rows, e)
	}
	bindings, e := s.ListRoleBindingsAuthorized(ctx, "next", "owner", ReadStrong)
	if e != nil {
		t.Fatal(e)
	}
	found := false
	for _, b := range bindings {
		if b.ID == "dr-reader" && b.RoleID == "reader" && b.SubjectID == "owner" && b.State == "active" {
			found = true
		}
	}
	if !found {
		t.Fatal("binding lost")
	}
	for _, tc := range []struct {
		user, action string
		allowed      bool
	}{{"owner", "organization.profile.read", true}, {"owner", "organization.application.manage", false}, {"next", "organization.application.manage", true}} {
		d, e := s.CheckAuthorization(ctx, AuthorizationSubject{SubjectType: "user", ID: tc.user, OrganizationID: "owner"}, tc.action, AuthorizationResource{ProviderID: "grant", ResourceType: "organization", ID: "owner"}, ReadStrong)
		if e != nil || d.Allowed != tc.allowed {
			t.Fatalf("decision %+v %+v %v", tc, d, e)
		}
		t.Logf("AUTHZ utc=%s user=%s action=%s allowed=%t", time.Now().UTC().Format(time.RFC3339Nano), tc.user, tc.action, d.Allowed)
	}
	if _, e = s.CreateGroupAuthorized(ctx, "owner", "owner", UpsertGroupInput{ID: "forbidden", Name: "Forbidden"}); ErrorStatus(e) != 403 {
		t.Fatalf("former owner write %v", e)
	}
	t.Logf("AUTHORITY PASS owner=next etag=%s role/binding preserved decisions=true,false,true former-owner-write=403", owner.ETag)
}
