package grant

import (
	"context"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/mrchypark/rhiza"
)

// Exercise Grant startup and authority state, not just Rhiza SQL. Only the
// disposable data directory is lost; the independent archive survives.
func TestRestoreLocalStorageAuthorityContinuity(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 90*time.Second)
	defer cancel()
	cfg := Config{
		Security: SecurityConfig{BootstrapUserID: "owner", BootstrapUserName: "Owner"},
		Database: DatabaseConfig{
			DataDir: filepath.Join(t.TempDir(), "data"), DefaultReadConsistency: ReadStrong,
			Rhiza: rhiza.Config{
				Local: true, NodeID: "grant-local-recovery", ClusterID: "grant-local-recovery",
				ObjStoreProvider: rhiza.ObjectStoreProviderFilesystem, ObjStoreDir: t.TempDir(),
				ObjStoreDurability: rhiza.ObjectStoreDurabilityBeforeAck,
			},
		},
	}
	var store *Store
	t.Cleanup(func() {
		if store != nil {
			if err := store.Close(); err != nil {
				t.Errorf("cleanup store: %v", err)
			}
		}
	})
	open := func() *Service {
		t.Helper()
		var err error
		store, err = OpenStore(ctx, cfg)
		if err != nil {
			t.Fatalf("Grant startup: %v", err)
		}
		if err := store.EnsureBootstrapData(ctx); err != nil {
			t.Fatal(err)
		}
		return NewService(cfg, store)
	}
	s := open()
	ownershipMember(t, ctx, s, "next")
	if _, err := s.CreateRoleAuthorized(ctx, "owner", "owner", CreateRoleInput{ID: "reader", Code: "reader", Name: "Reader"}); err != nil {
		t.Fatal(err)
	}
	if _, err := s.ReplaceRoleActionsAuthorized(ctx, "owner", "owner", "reader", []string{"organization.profile.read"}); err != nil {
		t.Fatal(err)
	}
	if _, err := s.CreateRoleBindingAuthorized(ctx, "owner", "owner", CreateRoleBindingInput{ID: "recovery-reader", SubjectType: "user", SubjectID: "owner", SubjectOrganizationID: "owner", ProviderID: "grant", ResourceType: "organization", ResourceID: "owner", RoleID: "reader", State: "active"}); err != nil {
		t.Fatal(err)
	}
	before, err := s.GetOrganizationOwnerAuthorized(ctx, "owner", "owner")
	if err != nil {
		t.Fatal(err)
	}
	transferred, err := s.TransferOrganizationOwnerAuthorized(ctx, "owner", "owner", "next", before.ETag)
	if err != nil {
		t.Fatal(err)
	}
	check := func() {
		t.Helper()
		owner, err := s.GetOrganizationOwnerAuthorized(ctx, "next", "owner")
		if err != nil || owner.OwnerUserID != "next" || owner.ETag != transferred.ETag {
			t.Fatalf("ownership/revision changed: %+v, %v", owner, err)
		}
		for _, tc := range []struct {
			user, action string
			allowed      bool
		}{{"owner", "organization.profile.read", true}, {"owner", "organization.application.manage", false}, {"next", "organization.application.manage", true}} {
			decision, err := s.CheckAuthorization(ctx, AuthorizationSubject{SubjectType: "user", ID: tc.user, OrganizationID: "owner"}, tc.action, AuthorizationResource{ProviderID: "grant", ResourceType: "organization", ID: "owner"}, ReadStrong)
			if err != nil || decision.Allowed != tc.allowed {
				t.Fatalf("authorization %+v: %+v, %v", tc, decision, err)
			}
		}
		if _, err := s.CreateGroupAuthorized(ctx, "owner", "owner", UpsertGroupInput{ID: "forbidden", Name: "Forbidden"}); err == nil || ErrorStatus(err) != 403 {
			t.Fatalf("former owner regained write authority: %v", err)
		}
	}
	check()
	if err := store.Close(); err != nil {
		t.Fatal(err)
	}
	store = nil
	if err := RestoreLocalStorage(ctx, cfg); err != nil {
		t.Fatal(err)
	}
	s = open()
	check()
	for _, id := range []string{"after-first-loss", "after-second-loss"} {
		if err := store.Close(); err != nil {
			t.Fatal(err)
		}
		store = nil
		if err := os.RemoveAll(cfg.Database.DataDir); err != nil {
			t.Fatal(err)
		}
		if _, err := os.Stat(cfg.Database.DataDir); !os.IsNotExist(err) {
			t.Fatalf("data directory not fully lost: %v", err)
		}
		if err := RestoreLocalStorage(ctx, cfg); err != nil {
			t.Fatal(err)
		}
		s = open()
		check()
		if id == "after-second-loss" {
			if _, err := s.GetGroupAuthorized(ctx, "next", "owner", "after-first-loss", ReadStrong); err != nil {
				t.Fatalf("post-recovery write lost: %v", err)
			}
		}
		if _, err := s.CreateGroupAuthorized(ctx, "next", "owner", UpsertGroupInput{ID: id, Name: id}); err != nil {
			t.Fatalf("post-recovery write: %v", err)
		}
		if _, err := s.GetGroupAuthorized(ctx, "next", "owner", id, ReadStrong); err != nil {
			t.Fatalf("post-recovery read: %v", err)
		}
		t.Logf("PASS %s: automatic archive restore, ownership revision, allow/deny decisions, new read/write", id)
	}
}
