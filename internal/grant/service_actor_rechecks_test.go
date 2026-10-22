package grant

import (
	"context"
	"fmt"
	"net/http"
	"net/http/httptest"
	"net/http/httputil"
	"net/url"
	"os"
	"strings"
	"sync"
	"testing"
	"time"
)

const (
	actorClusterOptInEnv        = "GRANT_EPHEMERAL_CLUSTER_TEST"
	actorClusterNamespace       = "GRANT_CLUSTER_TEST_NAMESPACE"
	actorClusterRuntimeEnv      = "GRANT_CLUSTER_TEST_RUNTIME_URL"
	actorClusterSecretEnv       = "GRANT_CLUSTER_TEST_RUNTIME_SECRET"
	actorClusterVotersEnv       = "GRANT_CLUSTER_TEST_VOTER_URLS"
	actorClusterNamespacePrefix = "grant-fixed9-"
)

var actorClusterResetLock sync.Mutex

type actorRecheckGate struct {
	mu      sync.Mutex
	paused  bool
	reached chan struct{}
	resume  chan struct{}
	once    sync.Once
	next    http.Handler
}

func (g *actorRecheckGate) arm() {
	g.mu.Lock()
	g.paused = true
	g.mu.Unlock()
}
func (g *actorRecheckGate) release() { g.once.Do(func() { close(g.resume) }) }

func (g *actorRecheckGate) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	g.mu.Lock()
	paused := g.paused
	g.mu.Unlock()
	if paused && r.URL.Path == "/sql/execute" {
		select {
		case g.reached <- struct{}{}:
		case <-r.Context().Done():
			return
		}
		select {
		case <-g.resume:
		case <-r.Context().Done():
			return
		}
	}
	g.next.ServeHTTP(w, r)
}

func actorRecheckFixture(t *testing.T) (context.Context, *Service, *Service, *actorRecheckGate) {
	t.Helper()
	clusterOptIn := os.Getenv(actorClusterOptInEnv)
	if clusterOptIn != "" && clusterOptIn != "1" {
		t.Fatalf("%s must be unset or exactly 1, got %q", actorClusterOptInEnv, clusterOptIn)
	}
	if clusterOptIn == "1" {
		return actorClusterRecheckFixture(t, localActorRecheckConfig(t))
	}
	localCfg := localActorRecheckConfig(t)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	t.Cleanup(cancel)
	local, err := OpenStore(ctx, localCfg)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = local.Close() })
	if err = local.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	gate := &actorRecheckGate{reached: make(chan struct{}, 1), resume: make(chan struct{})}
	gate.next = local.RuntimeHandler()
	server := httptest.NewServer(gate)
	t.Cleanup(func() { gate.release(); server.Close() })
	remoteCfg := localCfg
	remoteCfg.Database.RuntimeMode = "remote"
	remoteCfg.Database.RemoteNodes = []string{server.URL}
	remote := &Store{cfg: remoteCfg}
	return ctx, NewService(localCfg, local), NewService(remoteCfg, remote), gate
}

func localActorRecheckConfig(t *testing.T) Config {
	t.Helper()
	return Config{
		Security: SecurityConfig{BootstrapUserID: "owner", BootstrapUserName: "Owner"},
		Database: DatabaseConfig{
			DataDir:                t.TempDir(),
			DefaultReadConsistency: ReadStrong,
			RemoteAPISecret:        strings.Repeat("r", 40),
		},
	}
}

func actorClusterRecheckFixture(t *testing.T, localCfg Config) (context.Context, *Service, *Service, *actorRecheckGate) {
	t.Helper()
	namespace := strings.TrimSpace(os.Getenv(actorClusterNamespace))
	runtimeURL := strings.TrimRight(strings.TrimSpace(os.Getenv(actorClusterRuntimeEnv)), "/")
	voterURLs := clusterVoterURLs()
	clusterSecret := os.Getenv(actorClusterSecretEnv)
	if !validClusterNamespace(namespace) || runtimeURL == "" || len(clusterSecret) < 32 || clusterSecret == localCfg.Database.RemoteAPISecret || len(voterURLs) != 3 {
		t.Fatalf("cluster test requires explicit namespace, three voter URLs, private runtime URL, and >=32-byte secret; namespace=%q runtimeURL=%q voterCount=%d", namespace, runtimeURL, len(voterURLs))
	}
	if !validClusterHTTPBase(runtimeURL, namespace) {
		t.Fatalf("cluster runtime URL must be HTTP namespace-internal endpoint, got %q", redactURL(runtimeURL))
	}
	seenVoters := make(map[string]struct{}, len(voterURLs))
	for _, voter := range voterURLs {
		if !validClusterHTTPBase(voter, namespace) {
			t.Fatalf("cluster voter URL must be HTTP namespace-internal endpoint, got %q", redactURL(voter))
		}
		if _, exists := seenVoters[voter]; exists {
			t.Fatalf("cluster voter URLs must identify three distinct nodes: %q", voter)
		}
		seenVoters[voter] = struct{}{}
	}
	actorClusterResetLock.Lock()
	t.Cleanup(actorClusterResetLock.Unlock)
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	t.Cleanup(cancel)
	directCfg := localCfg
	directCfg.Database.RuntimeMode = "remote"
	directCfg.Database.RemoteNodes = []string{runtimeURL}
	directCfg.Database.RemoteAPISecret = clusterSecret
	direct := &Store{cfg: directCfg}
	cleanupClusterActorData(t, ctx, direct)
	t.Cleanup(func() {
		cleanupCtx, cleanupCancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cleanupCancel()
		cleanupClusterActorData(t, cleanupCtx, direct)
		assertStrongClusterRows(t, cleanupCtx, direct, "SELECT id FROM organizations WHERE id='owner' AND inactivated_at IS NULL", 1)
		assertStrongClusterRows(t, cleanupCtx, direct, "SELECT id FROM role_bindings WHERE id='binding-system-bootstrap-owner' AND subject_type='user' AND subject_id='owner' AND subject_organization_id='owner' AND provider_id='grant' AND resource_type='organization' AND resource_id='owner' AND state='active'", 1)
	})
	if err := direct.EnsureBootstrapData(ctx); err != nil {
		t.Fatalf("bootstrap cluster owner: %v", err)
	}
	assertStrongClusterRows(t, ctx, direct, "SELECT id FROM users WHERE id='owner' AND display_name='Owner' AND active=1", 1)
	for _, voter := range voterURLs {
		if err := assertStrongClusterRead(ctx, direct, voter); err != nil {
			t.Fatalf("strong read from cluster voter: %v", err)
		}
	}
	gate := &actorRecheckGate{reached: make(chan struct{}, 1), resume: make(chan struct{})}
	target, err := url.Parse(runtimeURL)
	if err != nil {
		t.Fatalf("parse cluster runtime URL: %v", err)
	}
	gate.next = httputil.NewSingleHostReverseProxy(target)
	proxy := httptest.NewServer(gate)
	t.Cleanup(func() { gate.release(); proxy.Close() })
	proxyCfg := directCfg
	proxyCfg.Database.RemoteNodes = []string{proxy.URL}
	proxyStore := &Store{cfg: proxyCfg}
	return ctx, NewService(directCfg, direct), NewService(proxyCfg, proxyStore), gate
}

func splitNonempty(raw string) []string {
	var out []string
	for _, value := range strings.Split(raw, ",") {
		if value = strings.TrimSpace(value); value != "" {
			out = append(out, value)
		}
	}
	return out
}

func clusterVoterURLs() []string {
	var out []string
	for _, voter := range splitNonempty(os.Getenv(actorClusterVotersEnv)) {
		out = append(out, strings.TrimRight(voter, "/"))
	}
	return out
}

func validClusterNamespace(value string) bool {
	if !strings.HasPrefix(value, actorClusterNamespacePrefix) || len(value) == len(actorClusterNamespacePrefix) || len(value) > 63 || strings.HasPrefix(value, "-") || strings.HasSuffix(value, "-") {
		return false
	}
	for i, r := range value {
		if (r < 'a' || r > 'z') && (r < '0' || r > '9') && r != '-' || r == '-' && (i == 0 || i == len(value)-1) {
			return false
		}
	}
	return true
}

func validClusterHTTPBase(value, namespace string) bool {
	u, err := url.Parse(value)
	if err != nil || u.Scheme != "http" || u.User != nil || u.RawQuery != "" || u.Fragment != "" || u.Path != "" || u.Port() != "8200" {
		return false
	}
	hostname := strings.ToLower(u.Hostname())
	return hostname != "" && (strings.HasSuffix(hostname, "."+namespace+".svc") || strings.HasSuffix(hostname, "."+namespace+".svc.cluster.local"))
}

func redactURL(value string) string {
	u, err := url.Parse(value)
	if err != nil {
		return "<invalid>"
	}
	return u.Scheme + "://" + u.Hostname()
}

func cleanupClusterActorData(t *testing.T, ctx context.Context, store *Store) {
	t.Helper()
	now := nowString()
	statements := []Statement{
		{SQL: "DELETE FROM group_memberships WHERE group_id LIKE 'race-%'"},
		{SQL: "DELETE FROM role_bindings WHERE id LIKE 'race-%' OR id LIKE 'cross-%' OR subject_id='delegate' OR subject_organization_id='receiver' OR resource_id='receiver'"},
		{SQL: "DELETE FROM role_actions WHERE role_id LIKE 'race-%' OR role_id LIKE 'cross-%'"},
		{SQL: "DELETE FROM roles WHERE id LIKE 'race-%' OR id LIKE 'cross-%'"},
		{SQL: "DELETE FROM agreements WHERE id='cross-deactivate-agreement'"},
		{SQL: "DELETE FROM groups WHERE id LIKE 'race-%'"},
		{SQL: "DELETE FROM organization_memberships WHERE user_id LIKE 'race-%' OR user_id='delegate' OR organization_id='receiver'"},
		{SQL: "DELETE FROM users WHERE id LIKE 'race-%' OR id='delegate'"},
		{SQL: "DELETE FROM organizations WHERE id='receiver'"},
		{SQL: "UPDATE organizations SET inactivated_at=NULL,updated_at=? WHERE id='owner'", Args: []any{now}},
		{SQL: "UPDATE role_bindings SET state='active',updated_at=? WHERE id='binding-system-bootstrap-owner'", Args: []any{now}},
		{SQL: "INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,state,created_by_user_id,created_at,updated_at) VALUES('binding-system-bootstrap-owner','user','owner','owner','grant','organization','owner','role-system-bootstrap-owner','active','owner',?,?) ON CONFLICT(id) DO NOTHING", Args: []any{now, now}},
	}
	if err := store.Transaction(ctx, statements); err != nil {
		t.Errorf("cleanup owned cluster rows: %v", err)
	}
}

func assertStrongClusterRead(ctx context.Context, store *Store, voter string) error {
	cfg := store.cfg
	cfg.Database.RemoteNodes = []string{voter}
	rows, err := (&Store{cfg: cfg}).Query(ctx, ReadStrong, "SELECT 1")
	if err != nil {
		return err
	}
	if len(rows) != 1 {
		return fmt.Errorf("strong read from %s returned %d rows", voter, len(rows))
	}
	return nil
}

func assertStrongClusterRows(t *testing.T, ctx context.Context, store *Store, query string, want int) {
	t.Helper()
	voters := clusterVoterURLs()
	if len(voters) != 3 {
		t.Errorf("cluster strong-state check requires three voter URLs, got %d", len(voters))
		return
	}
	for _, voter := range voters {
		cfg := store.cfg
		cfg.Database.RemoteNodes = []string{voter}
		rows, err := (&Store{cfg: cfg}).Query(ctx, ReadStrong, query)
		if err != nil {
			t.Errorf("strong state read from %s: %v", voter, err)
		} else if len(rows) != want {
			t.Errorf("strong state read from %s returned %d rows, want %d", voter, len(rows), want)
		}
	}
}

func waitActorRecheck(t *testing.T, ctx context.Context, ch <-chan struct{}, what string) {
	t.Helper()
	select {
	case <-ch:
	case <-ctx.Done():
		t.Fatalf("waiting for %s: %v", what, ctx.Err())
	}
}

func TestLifecycleActorRechecksRejectRevokedActors(t *testing.T) {
	tests := []struct {
		name  string
		setup func(*testing.T, context.Context, *Service)
		call  func(context.Context, *Service) error
		check func(*testing.T, context.Context, *Service)
	}{
		{
			name: "delete role binding",
			setup: func(t *testing.T, ctx context.Context, s *Service) {
				if _, err := s.CreateRoleAuthorized(ctx, "owner", "owner", CreateRoleInput{ID: "race-binding-role", Code: "race-binding", Name: "Race Binding"}); err != nil {
					t.Fatal(err)
				}
				_, err := s.CreateRoleBindingAuthorized(ctx, "owner", "owner", CreateRoleBindingInput{ID: "race-binding", SubjectType: "user", SubjectID: "owner", SubjectOrganizationID: "owner", ProviderID: "grant", ResourceType: "organization", ResourceID: "owner", RoleID: "race-binding-role", State: "disabled"})
				if err != nil {
					t.Fatal(err)
				}
			},
			call: func(ctx context.Context, s *Service) error {
				return s.DeleteRoleBindingAuthorized(ctx, "owner", "owner", "race-binding")
			},
			check: func(t *testing.T, ctx context.Context, s *Service) {
				if lifecycleCount(t, ctx, s, "SELECT id FROM role_bindings WHERE id='race-binding'") != 1 {
					t.Fatal("binding was deleted")
				}
			},
		},
		{
			name:  "create group",
			setup: func(*testing.T, context.Context, *Service) {},
			call: func(ctx context.Context, s *Service) error {
				_, err := s.CreateGroupAuthorized(ctx, "owner", "owner", UpsertGroupInput{ID: "race-create", Name: "Create"})
				return err
			},
			check: func(t *testing.T, ctx context.Context, s *Service) {
				if lifecycleCount(t, ctx, s, "SELECT id FROM groups WHERE id='race-create'") != 0 {
					t.Fatal("group was created")
				}
			},
		},
		{
			name: "update group",
			setup: func(t *testing.T, ctx context.Context, s *Service) {
				if _, err := s.CreateGroupAuthorized(ctx, "owner", "owner", UpsertGroupInput{ID: "race-update", Name: "Before"}); err != nil {
					t.Fatal(err)
				}
			},
			call: func(ctx context.Context, s *Service) error {
				_, err := s.UpdateGroupAuthorized(ctx, "owner", "owner", "race-update", UpsertGroupInput{Name: "After"})
				return err
			},
			check: func(t *testing.T, ctx context.Context, s *Service) {
				if lifecycleCount(t, ctx, s, "SELECT id FROM groups WHERE id='race-update' AND name='Before'") != 1 {
					t.Fatal("group changed")
				}
			},
		},
		{
			name: "delete group",
			setup: func(t *testing.T, ctx context.Context, s *Service) {
				if _, err := s.CreateGroupAuthorized(ctx, "owner", "owner", UpsertGroupInput{ID: "race-delete", Name: "Delete"}); err != nil {
					t.Fatal(err)
				}
				lifecycleExec(t, ctx, s, "INSERT INTO users(id,display_name,active,created_at) VALUES('race-member','Member',1,?)", nowString())
				if _, err := s.EnrollOrganizationMemberAuthorized(ctx, "owner", "owner", "race-member"); err != nil {
					t.Fatal(err)
				}
				if _, err := s.AddGroupMemberAuthorized(ctx, "owner", "owner", "race-delete", "race-member"); err != nil {
					t.Fatal(err)
				}
			},
			call: func(ctx context.Context, s *Service) error {
				return s.DeleteGroupAuthorized(ctx, "owner", "owner", "race-delete")
			},
			check: func(t *testing.T, ctx context.Context, s *Service) {
				if lifecycleCount(t, ctx, s, "SELECT id FROM groups WHERE id='race-delete'") != 1 || lifecycleCount(t, ctx, s, "SELECT * FROM group_memberships WHERE group_id='race-delete'") != 1 {
					t.Fatal("group deletion partially committed")
				}
			},
		},
		{
			name: "create role binding",
			setup: func(t *testing.T, ctx context.Context, s *Service) {
				lifecycleExec(t, ctx, s, "INSERT INTO users(id,display_name,active,created_at) VALUES('race-bind-target','Target',1,?)", nowString())
				if _, err := s.EnrollOrganizationMemberAuthorized(ctx, "owner", "owner", "race-bind-target"); err != nil {
					t.Fatal(err)
				}
			},
			call: func(ctx context.Context, s *Service) error {
				_, err := s.CreateRoleBindingAuthorized(ctx, "owner", "owner", CreateRoleBindingInput{ID: "race-create-binding", SubjectType: "user", SubjectID: "race-bind-target", SubjectOrganizationID: "owner", ProviderID: "grant", ResourceType: "organization", ResourceID: "owner", RoleID: "role-system-bootstrap-owner", State: "disabled"})
				return err
			},
			check: func(t *testing.T, ctx context.Context, s *Service) {
				if lifecycleCount(t, ctx, s, "SELECT id FROM role_bindings WHERE id='race-create-binding'") != 0 {
					t.Fatal("binding was created")
				}
			},
		},
		{
			name: "add group member",
			setup: func(t *testing.T, ctx context.Context, s *Service) {
				if _, err := s.CreateGroupAuthorized(ctx, "owner", "owner", UpsertGroupInput{ID: "race-add-member", Name: "Add"}); err != nil {
					t.Fatal(err)
				}
				lifecycleExec(t, ctx, s, "INSERT INTO users(id,display_name,active,created_at) VALUES('race-add-target','Target',1,?)", nowString())
				if _, err := s.EnrollOrganizationMemberAuthorized(ctx, "owner", "owner", "race-add-target"); err != nil {
					t.Fatal(err)
				}
			},
			call: func(ctx context.Context, s *Service) error {
				_, err := s.AddGroupMemberAuthorized(ctx, "owner", "owner", "race-add-member", "race-add-target")
				return err
			},
			check: func(t *testing.T, ctx context.Context, s *Service) {
				if lifecycleCount(t, ctx, s, "SELECT * FROM group_memberships WHERE group_id='race-add-member'") != 0 {
					t.Fatal("member was added")
				}
			},
		},
		{
			name: "remove group member",
			setup: func(t *testing.T, ctx context.Context, s *Service) {
				if _, err := s.CreateGroupAuthorized(ctx, "owner", "owner", UpsertGroupInput{ID: "race-remove-member", Name: "Remove"}); err != nil {
					t.Fatal(err)
				}
				lifecycleExec(t, ctx, s, "INSERT INTO users(id,display_name,active,created_at) VALUES('race-remove-target','Target',1,?)", nowString())
				if _, err := s.EnrollOrganizationMemberAuthorized(ctx, "owner", "owner", "race-remove-target"); err != nil {
					t.Fatal(err)
				}
				if _, err := s.AddGroupMemberAuthorized(ctx, "owner", "owner", "race-remove-member", "race-remove-target"); err != nil {
					t.Fatal(err)
				}
			},
			call: func(ctx context.Context, s *Service) error {
				return s.RemoveGroupMemberAuthorized(ctx, "owner", "owner", "race-remove-member", "race-remove-target")
			},
			check: func(t *testing.T, ctx context.Context, s *Service) {
				if lifecycleCount(t, ctx, s, "SELECT * FROM group_memberships WHERE group_id='race-remove-member'") != 1 {
					t.Fatal("member was removed")
				}
			},
		},
		{
			name:  "deactivate organization",
			setup: func(*testing.T, context.Context, *Service) {},
			call: func(ctx context.Context, s *Service) error {
				_, err := s.DeactivateOrganization(ctx, "owner", "owner", "owner")
				return err
			},
			check: func(t *testing.T, ctx context.Context, s *Service) {
				if lifecycleCount(t, ctx, s, "SELECT id FROM organizations WHERE id='owner' AND inactivated_at IS NULL") != 1 {
					t.Fatal("organization was deactivated")
				}
			},
		},
		{
			name: "reactivate organization",
			setup: func(t *testing.T, ctx context.Context, s *Service) {
				lifecycleExec(t, ctx, s, "UPDATE organizations SET inactivated_at=? WHERE id='owner'", nowString())
			},
			call: func(ctx context.Context, s *Service) error {
				_, err := s.ReactivateOrganization(ctx, "owner", "owner", "owner")
				return err
			},
			check: func(t *testing.T, ctx context.Context, s *Service) {
				if lifecycleCount(t, ctx, s, "SELECT id FROM organizations WHERE id='owner' AND inactivated_at IS NOT NULL") != 1 {
					t.Fatal("organization was reactivated")
				}
			},
		},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			ctx, local, remote, gate := actorRecheckFixture(t)
			tc.setup(t, ctx, local)
			gate.arm()
			result := make(chan error, 1)
			go func() { result <- tc.call(ctx, remote) }()
			waitActorRecheck(t, ctx, gate.reached, "final transaction")
			lifecycleExec(t, ctx, local, "UPDATE role_bindings SET state='disabled' WHERE subject_type='user' AND subject_id='owner' AND subject_organization_id='owner'")
			if os.Getenv(actorClusterOptInEnv) == "1" {
				assertStrongClusterRows(t, ctx, local.store, "SELECT id FROM role_bindings WHERE id='binding-system-bootstrap-owner' AND state='disabled'", 1)
			}
			gate.release()
			select {
			case err := <-result:
				if err == nil {
					t.Fatal("stale actor authority committed")
				}
				if ErrorStatus(err) != 409 {
					t.Fatalf("status=%d err=%v", ErrorStatus(err), err)
				}
			case <-ctx.Done():
				t.Fatalf("waiting for rejected transaction: %v", ctx.Err())
			}
			if os.Getenv(actorClusterOptInEnv) == "1" {
				assertStrongClusterRows(t, ctx, local.store, "SELECT id FROM role_bindings WHERE id='binding-system-bootstrap-owner' AND state='disabled'", 1)
			}
			tc.check(t, ctx, local)
			if os.Getenv(actorClusterOptInEnv) == "1" {
				for _, voter := range clusterVoterURLs() {
					cfg := local.store.cfg
					cfg.Database.RemoteNodes = []string{voter}
					tc.check(t, ctx, NewService(cfg, &Store{cfg: cfg}))
				}
			}
		})
	}
}

func TestDeleteGroupRechecksRoleBindingReferences(t *testing.T) {
	ctx, local, remote, gate := actorRecheckFixture(t)
	if _, err := local.CreateGroupAuthorized(ctx, "owner", "owner", UpsertGroupInput{ID: "race-reference", Name: "Reference"}); err != nil {
		t.Fatal(err)
	}
	lifecycleExec(t, ctx, local, "INSERT INTO users(id,display_name,active,created_at) VALUES('race-reference-member','Member',1,?)", nowString())
	if _, err := local.EnrollOrganizationMemberAuthorized(ctx, "owner", "owner", "race-reference-member"); err != nil {
		t.Fatal(err)
	}
	if _, err := local.AddGroupMemberAuthorized(ctx, "owner", "owner", "race-reference", "race-reference-member"); err != nil {
		t.Fatal(err)
	}
	gate.arm()
	result := make(chan error, 1)
	go func() { result <- remote.DeleteGroupAuthorized(ctx, "owner", "owner", "race-reference") }()
	waitActorRecheck(t, ctx, gate.reached, "final transaction")
	lifecycleExec(t, ctx, local, "INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,state,created_by_user_id,created_at,updated_at) VALUES('race-reference-binding','group','race-reference','owner','grant','organization','owner','role-system-bootstrap-owner','active','owner',?,?)", nowString(), nowString())
	gate.release()
	select {
	case err := <-result:
		if err == nil {
			t.Fatal("new role binding did not reject deletion")
		}
		if ErrorStatus(err) != 409 {
			t.Fatalf("status=%d err=%v", ErrorStatus(err), err)
		}
	case <-ctx.Done():
		t.Fatalf("waiting for rejected transaction: %v", ctx.Err())
	}
	checkReferenceState := func(t *testing.T, s *Service) {
		t.Helper()
		if lifecycleCount(t, ctx, s, "SELECT id FROM groups WHERE id='race-reference'") != 1 || lifecycleCount(t, ctx, s, "SELECT * FROM group_memberships WHERE group_id='race-reference'") != 1 {
			t.Fatal("group deletion partially committed")
		}
	}
	checkReferenceState(t, local)
	if os.Getenv(actorClusterOptInEnv) == "1" {
		for _, voter := range clusterVoterURLs() {
			cfg := local.store.cfg
			cfg.Database.RemoteNodes = []string{voter}
			checkReferenceState(t, NewService(cfg, &Store{cfg: cfg}))
		}
	}
}

func TestLifecycleActorRechecksRequiredActionOnly(t *testing.T) {
	tests := []struct {
		name        string
		action      string
		otherAction string
		setup       func(*testing.T, context.Context, *Service)
		call        func(context.Context, *Service) error
		check       func(*testing.T, context.Context, *Service)
	}{
		{
			name:        "group action",
			action:      "organization.group.manage",
			otherAction: "organization.permission.manage",
			setup:       func(*testing.T, context.Context, *Service) {},
			call: func(ctx context.Context, s *Service) error {
				_, err := s.CreateGroupAuthorized(ctx, "owner", "owner", UpsertGroupInput{ID: "race-action-group", Name: "Group"})
				return err
			},
			check: func(t *testing.T, ctx context.Context, s *Service) {
				if lifecycleCount(t, ctx, s, "SELECT id FROM groups WHERE id='race-action-group'") != 0 {
					t.Fatal("group was created")
				}
			},
		},
		{
			name:        "permission action",
			action:      "organization.permission.manage",
			otherAction: "organization.group.manage",
			setup: func(t *testing.T, ctx context.Context, s *Service) {
				lifecycleExec(t, ctx, s, "INSERT INTO users(id,display_name,active,created_at) VALUES('race-action-target','Target',1,?)", nowString())
				if _, err := s.EnrollOrganizationMemberAuthorized(ctx, "owner", "owner", "race-action-target"); err != nil {
					t.Fatal(err)
				}
				_, err := s.CreateRoleBindingAuthorized(ctx, "owner", "owner", CreateRoleBindingInput{ID: "race-action-target-binding", SubjectType: "user", SubjectID: "race-action-target", SubjectOrganizationID: "owner", ProviderID: "grant", ResourceType: "organization", ResourceID: "owner", RoleID: "race-action-group-role", State: "disabled"})
				if err != nil {
					t.Fatal(err)
				}
			},
			call: func(ctx context.Context, s *Service) error {
				return s.DeleteRoleBindingAuthorized(ctx, "owner", "owner", "race-action-target-binding")
			},
			check: func(t *testing.T, ctx context.Context, s *Service) {
				if lifecycleCount(t, ctx, s, "SELECT id FROM role_bindings WHERE id='race-action-target-binding'") != 1 {
					t.Fatal("binding was deleted")
				}
			},
		},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			ctx, local, remote, gate := actorRecheckFixture(t)
			for _, role := range []struct{ id, action string }{{"race-action-group-role", "organization.group.manage"}, {"race-action-permission-role", "organization.permission.manage"}} {
				if _, err := local.CreateRoleAuthorized(ctx, "owner", "owner", CreateRoleInput{ID: role.id, Code: role.id, Name: role.id}); err != nil {
					t.Fatal(err)
				}
				if _, err := local.ReplaceRoleActionsAuthorized(ctx, "owner", "owner", role.id, []string{role.action}); err != nil {
					t.Fatal(err)
				}
				if _, err := local.CreateRoleBindingAuthorized(ctx, "owner", "owner", CreateRoleBindingInput{ID: role.id + "-binding", SubjectType: "user", SubjectID: "owner", SubjectOrganizationID: "owner", ProviderID: "grant", ResourceType: "organization", ResourceID: "owner", RoleID: role.id, State: "active"}); err != nil {
					t.Fatal(err)
				}
			}
			tc.setup(t, ctx, local)
			lifecycleExec(t, ctx, local, "DELETE FROM role_bindings WHERE subject_type='user' AND subject_id='owner' AND subject_organization_id='owner' AND role_id='role-system-bootstrap-owner'")
			gate.arm()
			result := make(chan error, 1)
			go func() { result <- tc.call(ctx, remote) }()
			waitActorRecheck(t, ctx, gate.reached, "final transaction")
			if os.Getenv(actorClusterOptInEnv) == "1" {
				assertStrongClusterRows(t, ctx, local.store, "SELECT id FROM role_bindings WHERE id='binding-system-bootstrap-owner'", 0)
			}
			bindingID := "race-action-group-role-binding"
			if tc.action == "organization.permission.manage" {
				bindingID = "race-action-permission-role-binding"
			}
			lifecycleExec(t, ctx, local, "UPDATE role_bindings SET state='disabled' WHERE id=?", bindingID)
			for _, check := range []struct {
				action string
				want   bool
			}{{tc.action, false}, {tc.otherAction, true}} {
				d, err := local.CheckAuthorization(ctx, AuthorizationSubject{SubjectType: "user", ID: "owner", OrganizationID: "owner"}, check.action, AuthorizationResource{ProviderID: "grant", ResourceType: "organization", ID: "owner"}, ReadStrong)
				if err != nil || d.Allowed != check.want {
					t.Fatalf("action %s allowed=%v want=%v err=%v", check.action, d.Allowed, check.want, err)
				}
			}
			gate.release()
			select {
			case err := <-result:
				if err == nil {
					t.Fatal("stale action authority committed")
				}
				if ErrorStatus(err) != 409 {
					t.Fatalf("status=%d err=%v", ErrorStatus(err), err)
				}
			case <-ctx.Done():
				t.Fatalf("waiting for rejected transaction: %v", ctx.Err())
			}
			if os.Getenv(actorClusterOptInEnv) == "1" {
				assertStrongClusterRows(t, ctx, local.store, "SELECT id FROM role_bindings WHERE id='binding-system-bootstrap-owner'", 0)
			}
			tc.check(t, ctx, local)
			if os.Getenv(actorClusterOptInEnv) == "1" {
				for _, voter := range clusterVoterURLs() {
					cfg := local.store.cfg
					cfg.Database.RemoteNodes = []string{voter}
					tc.check(t, ctx, NewService(cfg, &Store{cfg: cfg}))
				}
			}
			if err := tc.call(ctx, remote); err == nil {
				t.Fatal("revoked action remained authorized")
			} else if ErrorStatus(err) != 403 {
				t.Fatalf("revoked action status=%d err=%v", ErrorStatus(err), err)
			}
		})
	}
}

func TestOrganizationLifecycleKeepsAuthorizedCrossOrganizationDeactivate(t *testing.T) {
	var ctx context.Context
	var s *Service
	if os.Getenv(actorClusterOptInEnv) != "" {
		ctx, s, _, _ = actorRecheckFixture(t)
	} else {
		ctx, s = serviceFixture(t)
	}
	if _, err := s.CreateOrganizationAdmin(ctx, CreateOrganizationInput{ID: "receiver", Name: "Receiver", OwnerUserID: "owner"}); err != nil {
		t.Fatal(err)
	}
	lifecycleExec(t, ctx, s, "INSERT INTO users(id,display_name,active,created_at) VALUES('delegate','Delegate',1,?)", nowString())
	if _, err := s.EnrollOrganizationMemberAuthorized(ctx, "owner", "owner", "delegate"); err != nil {
		t.Fatal(err)
	}
	if _, err := s.CreateRoleAuthorized(ctx, "owner", "owner", CreateRoleInput{ID: "cross-deactivate", Code: "cross-deactivate", Name: "Cross Deactivate"}); err != nil {
		t.Fatal(err)
	}
	if _, err := s.ReplaceRoleActionsAuthorized(ctx, "owner", "owner", "cross-deactivate", []string{"organization.permission.manage"}); err != nil {
		t.Fatal(err)
	}
	agreement := "cross-deactivate-agreement"
	if _, err := s.CreateAgreementAuthorized(ctx, "owner", "receiver", UpsertAgreementInput{ID: agreement, Title: "Cross Deactivate", AgreementType: "delegation", Status: "active", GrantorOrganizationID: "receiver", GranteeOrganizationID: "owner"}); err != nil {
		t.Fatal(err)
	}
	if _, err := s.CreateRoleBindingAuthorized(ctx, "owner", "owner", CreateRoleBindingInput{ID: "cross-deactivate-binding", SubjectType: "organization", SubjectID: "owner", SubjectOrganizationID: "owner", ProviderID: "grant", ResourceType: "organization", ResourceID: "receiver", RoleID: "cross-deactivate", AgreementID: &agreement, State: "active"}); err != nil {
		t.Fatal(err)
	}
	if d, err := s.CheckAuthorization(ctx, AuthorizationSubject{SubjectType: "user", ID: "delegate", OrganizationID: "owner"}, "organization.permission.manage", AuthorizationResource{ProviderID: "grant", ResourceType: "organization", ID: "owner"}, ReadStrong); err != nil || d.Allowed {
		t.Fatalf("delegate retained actor-org permission: %+v %v", d, err)
	}
	if _, err := s.DeactivateOrganization(ctx, "delegate", "owner", "receiver"); err != nil {
		t.Fatal(err)
	}
	if lifecycleCount(t, ctx, s, "SELECT id FROM organizations WHERE id='receiver' AND inactivated_at IS NOT NULL") != 1 {
		t.Fatal("authorized cross-organization deactivation did not apply")
	}
	if os.Getenv(actorClusterOptInEnv) == "1" {
		for _, voter := range clusterVoterURLs() {
			cfg := s.store.cfg
			cfg.Database.RemoteNodes = []string{voter}
			voterService := NewService(cfg, &Store{cfg: cfg})
			if lifecycleCount(t, ctx, voterService, "SELECT id FROM organizations WHERE id='receiver' AND inactivated_at IS NOT NULL") != 1 {
				t.Fatalf("receiver deactivation missing on %s", voter)
			}
		}
	}
	if _, err := s.ReactivateOrganization(ctx, "owner", "receiver", "receiver"); err != nil {
		t.Fatal(err)
	}
	if lifecycleCount(t, ctx, s, "SELECT id FROM organizations WHERE id='receiver' AND inactivated_at IS NULL") != 1 {
		t.Fatal("authorized same-organization reactivation did not apply")
	}
	if os.Getenv(actorClusterOptInEnv) == "1" {
		for _, voter := range clusterVoterURLs() {
			cfg := s.store.cfg
			cfg.Database.RemoteNodes = []string{voter}
			voterService := NewService(cfg, &Store{cfg: cfg})
			if lifecycleCount(t, ctx, voterService, "SELECT id FROM organizations WHERE id='receiver' AND inactivated_at IS NULL") != 1 {
				t.Fatalf("receiver reactivation missing on %s", voter)
			}
		}
	}
}
