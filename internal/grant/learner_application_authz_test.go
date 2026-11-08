package grant

import (
	"context"
	"fmt"
	"github.com/mrchypark/rhiza"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func TestLearnerApplicationAuthorizationRealReplica(t *testing.T) {
	ctx := context.Background()
	var calls atomic.Int32
	var profiles atomic.Int32
	var idpState atomic.Int32
	idp := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/oidc/introspect":
			calls.Add(1)
			if idpState.Load() == 1 {
				fmt.Fprint(w, `{"active":false}`)
				return
			}
			if idpState.Load() == 2 {
				w.WriteHeader(503)
				return
			}
			fmt.Fprintf(w, `{"active":true,"sub":"subject","client_id":"login","exp":%d,"aud":["https://grant.example.test/"],"scope":"openid"}`, time.Now().Add(time.Hour).Unix())
		case "/oidc/userinfo":
			profiles.Add(1)
			fmt.Fprint(w, `{"sub":"subject"}`)
		default:
			w.WriteHeader(404)
		}
	}))
	defer idp.Close()
	udp, err := net.ListenPacket("udp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	addr := udp.LocalAddr().String()
	udp.Close()
	member := rhiza.Member{ID: "voter", PeerURL: "quic://" + addr, PublicKey: rhiza.PeerPublicKey("application-fence", "voter", "voter-only")}
	cfg := Config{Auth: AuthConfig{ProviderMode: "goauthy", Goauthy: &GoauthyConfig{Issuer: idp.URL, IntrospectionClientID: "introspector", IntrospectionClientSecret: "secret", ExpectedClientID: "login", Resource: "https://grant.example.test/", ApplicationID: "test-app", RequiredScopes: []string{"openid"}}}, Database: DatabaseConfig{DataDir: t.TempDir(), ReadFenceKey: strings.Repeat("f", 40), RemoteAPISecret: strings.Repeat("w", 40), Rhiza: rhiza.Config{ClusterID: "application-fence", NodeID: "voter", PeerAddr: addr, PeerToken: "voter-only", Members: []rhiza.Member{member}, AdminToken: "sync-only", ObjStoreProvider: "filesystem", ObjStoreDir: t.TempDir(), ObjStoreDurability: rhiza.ObjectStoreDurabilityBeforeAck}}}
	voter, err := OpenStore(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	defer voter.Close()
	if err = voter.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	user, err := voter.EnsureExternalIdentity(ctx, ExternalIdentityMaterialization{Provider: "goauthy", Issuer: idp.URL, ExternalUserID: "subject", DisplayName: "Test"})
	if err != nil {
		t.Fatal(err)
	}
	now := nowString()
	statements := []Statement{
		{SQL: "INSERT INTO roles(id,organization_id,code,name,created_at,updated_at) VALUES('test-workspace-role','conalog-grant','workspace','Workspace',?,?)", Args: []any{now, now}},
		{SQL: "INSERT INTO role_actions(role_id,action_id) VALUES('test-workspace-role','organization.group.manage')"},
		{SQL: "INSERT INTO organization_memberships(id,organization_id,user_id,membership_role,status,created_at,updated_at) VALUES('test-membership','conalog-grant',?,'member','active',?,?)", Args: []any{user.ID, now, now}},
		{SQL: "INSERT INTO applications(id,organization_id,slug,name,application_type,app_type,first_party,consent_policy,active,provisioning_status,created_by_user_id,created_at,updated_at) VALUES('test-app','conalog-grant','test-app','Test','confidential','server',1,'auto_approve',1,'ready','conalog-grant',?,?)", Args: []any{now, now}},
		{SQL: "INSERT INTO application_actions(application_id,action_id) VALUES('test-app','organization.group.manage')"},
		{SQL: "UPDATE applications SET first_party=0,consent_policy='explicit' WHERE id='test-app'"},
		{SQL: "INSERT INTO application_grants(id,user_id,application_id,organization_id,status,consented_at,created_at,updated_at) VALUES('test-consent',?,'test-app','conalog-grant','active',?,?,?)", Args: []any{user.ID, now, now, now}},
		{SQL: "INSERT INTO application_grant_actions(application_grant_id,action_id) VALUES('test-consent','organization.group.manage')"},
		{SQL: "INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,state,created_by_user_id,created_at,updated_at) VALUES('test-binding','user',?,'conalog-grant','grant','workspace','cypark-default','test-workspace-role','active','conalog-grant',?,?)", Args: []any{user.ID, now, now}},
	}
	if err = voter.Transaction(ctx, statements); err != nil {
		t.Fatal(err)
	}
	var blockFence atomic.Bool
	fenceEntered := make(chan struct{}, 1)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/read-fence" && blockFence.Load() {
			fenceEntered <- struct{}{}
			<-r.Context().Done()
			return
		}
		voter.RuntimeHandler().ServeHTTP(w, r)
	}))
	defer server.Close()
	public, err := rhiza.NewReplicaMember("application-fence", member)
	if err != nil {
		t.Fatal(err)
	}
	lc := cfg
	lc.Database.DataDir = t.TempDir()
	lc.Database.LocalNodeRole = "learner"
	lc.Database.RemoteAPISecret = ""
	lc.Database.RemoteNodes = []string{server.URL}
	lc.Database.Replica = rhiza.ReplicaConfig{ClusterID: "application-fence", ReplicaID: requestID(), AdminToken: "sync-only", Members: []rhiza.ReplicaMember{public}, SyncInterval: time.Hour}
	replica, err := OpenStore(ctx, lc)
	if err != nil {
		t.Fatal(err)
	}
	defer replica.Close()
	learner := NewService(lc, replica)
	learner.client = idp.Client()
	checks := []ApplicationAuthorizationCheck{{Action: "organization.group.manage", Resource: AuthorizationResource{ProviderID: "grant", ResourceType: "workspace", ID: "cypark-default"}}}
	check := func(ctx context.Context, wantAllowed bool, wantStatus int) {
		t.Helper()
		before := calls.Load()
		out, err := learner.CheckApplicationAuthorizations(ctx, "Bearer token", "conalog-grant", checks)
		if wantStatus != 0 {
			if ErrorStatus(err) != wantStatus {
				t.Fatalf("want status %d got %v out=%v", wantStatus, err, out)
			}
			return
		}
		if err != nil || len(out) != 1 || out[0].Allowed != wantAllowed {
			t.Fatalf("allowed=%v out=%v err=%v", wantAllowed, out, err)
		}
		if calls.Load()-before != 1 {
			t.Fatalf("external identity calls=%d", calls.Load()-before)
		}
	}
	// No manual Sync or request priming: first request must obtain its own fence.
	check(ctx, true, 0)
	t.Run("organization scope is independent of item count", func(t *testing.T) {
		inputs := [][]ApplicationAuthorizationCheck{nil, {}, checks}
		assertScope := func(t *testing.T, wantStatus int) {
			t.Helper()
			for _, input := range inputs {
				out, err := learner.CheckApplicationAuthorizations(ctx, "Bearer token", "conalog-grant", input)
				if wantStatus != 0 {
					if err == nil || ErrorStatus(err) != wantStatus || len(out) != 0 {
						t.Fatalf("items=%d out=%v err=%v, want status %d", len(input), out, err, wantStatus)
					}
				} else if err != nil || out == nil || len(out) != len(input) || (len(out) > 0 && !out[0].Allowed) {
					t.Fatalf("valid scope: items=%d out=%v err=%v", len(input), out, err)
				}
			}
		}
		assertScope(t, 0)
		for _, mutation := range []struct {
			name, sql, restore string
			restoreArgs        []any
		}{
			{"inactive membership", "UPDATE organization_memberships SET status='inactive' WHERE id='test-membership'", "UPDATE organization_memberships SET status='active' WHERE id='test-membership'", nil},
			{"missing membership", "DELETE FROM organization_memberships WHERE id='test-membership'", "INSERT INTO organization_memberships(id,organization_id,user_id,membership_role,status,created_at,updated_at) VALUES('test-membership','conalog-grant',?,'member','active',?,?)", []any{user.ID, now, now}},
			{"inactive organization", "UPDATE organizations SET inactivated_at=? WHERE id='conalog-grant'", "UPDATE organizations SET inactivated_at=NULL WHERE id='conalog-grant'", nil},
		} {
			t.Run(mutation.name, func(t *testing.T) {
				var args []any
				if mutation.name == "inactive organization" {
					args = []any{now}
				}
				if _, err := voter.Exec(ctx, mutation.sql, args...); err != nil {
					t.Fatal(err)
				}
				defer func() {
					if _, err := voter.Exec(ctx, mutation.restore, mutation.restoreArgs...); err != nil {
						t.Fatal(err)
					}
				}()
				assertScope(t, 403)
			})
			assertScope(t, 0)
		}
	})
	t.Run("empty request retries scope at the same snapshot", func(t *testing.T) {
		identityReads := 0
		before := calls.Load()
		replica.afterReplicaQuery = func(_ context.Context, sql string) {
			if !strings.Contains(sql, "JOIN user_identities") {
				return
			}
			identityReads++
			if identityReads == 1 {
				if _, err := voter.Exec(ctx, "UPDATE organization_memberships SET status='inactive' WHERE id='test-membership'"); err != nil {
					t.Fatal(err)
				}
				if err := replica.replica.Sync(ctx); err != nil {
					t.Fatal(err)
				}
			}
		}
		defer func() {
			replica.afterReplicaQuery = nil
			if _, err := voter.Exec(ctx, "UPDATE organization_memberships SET status='active' WHERE id='test-membership'"); err != nil {
				t.Fatal(err)
			}
		}()
		out, err := learner.CheckApplicationAuthorizations(ctx, "Bearer token", "conalog-grant", nil)
		if err == nil || ErrorStatus(err) != 403 || len(out) != 0 || identityReads != 2 || calls.Load()-before != 1 {
			t.Fatalf("empty scope retry: out=%v err=%v identity reads=%d external calls=%d", out, err, identityReads, calls.Load()-before)
		}
	})
	voterService := NewService(cfg, voter)
	voterService.client = idp.Client()
	batch := []ApplicationAuthorizationCheck{checks[0], checks[0]}
	batch[1].Resource.ID = "unbound-workspace"
	for _, service := range []*Service{voterService, learner} {
		out, err := service.CheckApplicationAuthorizations(ctx, "Bearer token", "conalog-grant", batch)
		if err != nil || len(out) != 2 || !out[0].Allowed || out[1].Allowed {
			t.Fatalf("voter/learner cardinality and order: %v %v", out, err)
		}
	}
	// Mutate after the real production resolver reads the application. A retry
	// must reconstruct that context and reject the now-inactive application.
	beforeRetry := calls.Load()
	appReads := 0
	replica.afterReplicaQuery = func(_ context.Context, sql string) {
		if strings.Contains(sql, "FROM applications WHERE") {
			appReads++
		}
		if appReads == 1 && strings.Contains(sql, "FROM applications WHERE") {
			if _, err := voter.Exec(ctx, "UPDATE applications SET active=0 WHERE id='test-app'"); err != nil {
				t.Fatal(err)
			}
			if err := replica.replica.Sync(ctx); err != nil {
				t.Fatal(err)
			}
		}
	}
	result, err := learner.CheckApplicationAuthorizations(ctx, "Bearer token", "conalog-grant", batch)
	replica.afterReplicaQuery = nil
	if appReads != 2 || ErrorStatus(err) != 403 || len(result) != 0 || calls.Load()-beforeRetry != 1 {
		t.Fatalf("snapshot retry: appReads=%d result=%v err=%v external calls=%d", appReads, result, err, calls.Load()-beforeRetry)
	}
	if _, err := voter.Exec(ctx, "UPDATE applications SET active=1 WHERE id='test-app'"); err != nil {
		t.Fatal(err)
	}
	check(ctx, true, 0)
	for _, mutation := range []struct {
		sql, restore string
		status       int
	}{
		{"DELETE FROM application_actions WHERE application_id='test-app'", "INSERT INTO application_actions(application_id,action_id) VALUES('test-app','organization.group.manage')", 0},
		{"UPDATE application_grants SET status='revoked' WHERE id='test-consent'", "UPDATE application_grants SET status='active' WHERE id='test-consent'", 0},
		{"UPDATE role_bindings SET state='disabled' WHERE id='test-binding'", "UPDATE role_bindings SET state='active' WHERE id='test-binding'", 0},
		{"UPDATE applications SET provisioning_status='unprovisioned' WHERE id='test-app'", "UPDATE applications SET provisioning_status='ready' WHERE id='test-app'", 403},
		{"UPDATE applications SET active=0 WHERE id='test-app'", "UPDATE applications SET active=1 WHERE id='test-app'", 403},
		{"UPDATE organization_memberships SET status='inactive' WHERE id='test-membership'", "UPDATE organization_memberships SET status='active' WHERE id='test-membership'", 403},
		{"UPDATE user_identities SET external_user_id='unlinked' WHERE provider='goauthy'", "UPDATE user_identities SET external_user_id='subject' WHERE provider='goauthy'", 403},
	} {
		if _, err = voter.Exec(ctx, mutation.sql); err != nil {
			t.Fatal(err)
		}
		check(ctx, false, mutation.status)
		if _, err = voter.Exec(ctx, mutation.restore); err != nil {
			t.Fatal(err)
		}
		check(ctx, true, 0)
	}
	if _, err = learner.CheckApplicationAuthorizations(ctx, "Bearer token", "wrong-org", checks); ErrorStatus(err) != 403 {
		t.Fatalf("wrong org: %v", err)
	}
	// Exercise actual production wiring under identity remapping, sustained
	// snapshot movement, and cancellation after a later batch query.
	identityReads := 0
	beforeRetry = calls.Load()
	replica.afterReplicaQuery = func(_ context.Context, sql string) {
		if !strings.Contains(sql, "JOIN user_identities") {
			return
		}
		identityReads++
		if identityReads == 1 {
			if _, err := voter.Exec(ctx, "UPDATE user_identities SET external_user_id='moved' WHERE provider='goauthy'"); err != nil {
				t.Fatal(err)
			}
			if err := replica.replica.Sync(ctx); err != nil {
				t.Fatal(err)
			}
		}
	}
	result, err = learner.CheckApplicationAuthorizations(ctx, "Bearer token", "conalog-grant", batch)
	replica.afterReplicaQuery = nil
	if identityReads != 2 || ErrorStatus(err) != 403 || len(result) != 0 || calls.Load()-beforeRetry != 1 {
		t.Fatalf("identity retry: reads=%d out=%v err=%v calls=%d", identityReads, result, err, calls.Load()-beforeRetry)
	}
	if _, err := voter.Exec(ctx, "UPDATE user_identities SET external_user_id='subject' WHERE provider='goauthy'"); err != nil {
		t.Fatal(err)
	}
	appReads = 0
	beforeRetry = calls.Load()
	replica.afterReplicaQuery = func(_ context.Context, sql string) {
		if !strings.Contains(sql, "FROM applications WHERE") {
			return
		}
		appReads++
		if _, err := voter.Exec(ctx, "UPDATE applications SET name=? WHERE id='test-app'", fmt.Sprintf("churn-%d", appReads)); err != nil {
			t.Fatal(err)
		}
		if err := replica.replica.Sync(ctx); err != nil {
			t.Fatal(err)
		}
	}
	result, err = learner.CheckApplicationAuthorizations(ctx, "Bearer token", "conalog-grant", batch)
	replica.afterReplicaQuery = nil
	if appReads != 3 || ErrorStatus(err) != 503 || len(result) != 0 || calls.Load()-beforeRetry != 1 {
		t.Fatalf("retry exhaustion: reads=%d out=%v err=%v calls=%d", appReads, result, err, calls.Load()-beforeRetry)
	}
	laterCtx, laterCancel := context.WithCancel(ctx)
	defer laterCancel()
	queryCount := 0
	replica.afterReplicaQuery = func(_ context.Context, sql string) {
		if strings.Contains(sql, "WITH requested") {
			queryCount++
			if queryCount == 2 {
				laterCancel()
			}
		}
	}
	result, err = learner.CheckApplicationAuthorizations(laterCtx, "Bearer token", "conalog-grant", []ApplicationAuthorizationCheck{checks[0], checks[0], checks[0]})
	replica.afterReplicaQuery = nil
	if queryCount != 2 || err == nil || len(result) != 0 {
		t.Fatalf("later batch cancellation: queries=%d out=%v err=%v", queryCount, result, err)
	}

	// Rauthy with positive TTL must neither read nor write either cache on the
	// learner. Use the same real replica and a separately qualified identity.
	if err := voter.Transaction(ctx, []Statement{
		{SQL: "INSERT INTO user_identities(id,user_id,provider,issuer,collection_id,external_user_id,last_seen_at,created_at,updated_at) VALUES('rauthy-test',?,'rauthy','','','subject',?,?,?)", Args: []any{user.ID, now, now, now}},
		{SQL: "UPDATE applications SET rauthy_client_id='login' WHERE id='test-app'"},
	}); err != nil {
		t.Fatal(err)
	}
	rc := lc
	rc.Auth = AuthConfig{ProviderMode: "rauthy", Rauthy: RauthyConfig{BaseURL: idp.URL, IntrospectPath: "/oidc/introspect", UserinfoPath: "/oidc/userinfo", ClientID: "introspector", ClientSecret: "secret", CacheTTLSeconds: 3600}}
	rauthyLearner := NewService(rc, replica)
	rauthyLearner.client = idp.Client()
	beforeCold, beforeProfile := calls.Load(), profiles.Load()
	result, err = rauthyLearner.CheckApplicationAuthorizations(ctx, "Bearer cold-token", "conalog-grant", checks)
	if err != nil || len(result) != 1 || !result[0].Allowed || calls.Load()-beforeCold != 1 || profiles.Load()-beforeProfile != 1 {
		t.Fatalf("cold Rauthy: out=%v err=%v introspections=%d profiles=%d", result, err, calls.Load()-beforeCold, profiles.Load()-beforeProfile)
	}
	if len(rauthyLearner.authCache) != 0 {
		t.Fatal("learner populated memory cache")
	}
	rows, err := voter.Query(ctx, ReadStrong, "SELECT * FROM auth_identity_cache")
	if err != nil || len(rows) != 0 {
		t.Fatalf("learner persisted cache: %v %v", rows, err)
	}
	vc := rc
	vc.Database = cfg.Database
	rauthyVoter := NewService(vc, voter)
	rauthyVoter.client = idp.Client()
	warmIdentity, err := rauthyVoter.resolveExternalIdentity(ctx, "Bearer warm-token", ReadStrong)
	if err != nil {
		t.Fatal(err)
	}
	rows, err = voter.Query(ctx, ReadStrong, "SELECT * FROM auth_identity_cache")
	if err != nil || len(rows) != 1 {
		t.Fatalf("voter cache prerequisite: %v %v", rows, err)
	}
	// A normal successful request catches up the replicated warm-cache row;
	// this is not manual priming of first-request availability.
	if out, err := rauthyLearner.CheckApplicationAuthorizations(ctx, "Bearer warm-token", "conalog-grant", checks); err != nil || len(out) != 1 || !out[0].Allowed {
		t.Fatalf("warm-cache catch-up: %v %v", out, err)
	}
	rauthyLearner.cacheIdentity(tokenCacheKey("warm-token"), cachedIdentity{Identity: warmIdentity, ExpiresAt: time.Now().Add(time.Hour)})
	for _, state := range []int32{1, 2} {
		idpState.Store(state)
		dbCachedLearner := NewService(rc, replica)
		dbCachedLearner.client = idp.Client()
		for _, service := range []*Service{rauthyLearner, dbCachedLearner} {
			beforeCold = calls.Load()
			beforeProfile = profiles.Load()
			result, err = service.CheckApplicationAuthorizations(ctx, "Bearer warm-token", "conalog-grant", checks)
			want := 401
			if state == 2 {
				want = 503
			}
			if ErrorStatus(err) != want || len(result) != 0 || calls.Load()-beforeCold != 1 || profiles.Load() != beforeProfile {
				t.Fatalf("cached Rauthy state=%d out=%v err=%v introspections=%d", state, result, err, calls.Load()-beforeCold)
			}
			// Voters retain the warm-cache behavior without another IdP call.
			if _, err := rauthyVoter.resolveExternalIdentity(ctx, "Bearer warm-token", ReadStrong); err != nil || calls.Load()-beforeCold != 1 {
				t.Fatalf("voter legacy cache changed: %v", err)
			}
		}
	}
	idpState.Store(0)
	canceled, cancel := context.WithCancel(ctx)
	cancel()
	if _, err = learner.CheckApplicationAuthorizations(canceled, "Bearer token", "conalog-grant", checks); err == nil {
		t.Fatal("canceled request allowed")
	}
	// Disconnect while the quorum-fence request is in flight, rather than only
	// exercising an already-canceled token verification request.
	blockFence.Store(true)
	inflight, disconnect := context.WithCancel(ctx)
	done := make(chan error, 1)
	go func() {
		_, err := learner.CheckApplicationAuthorizations(inflight, "Bearer token", "conalog-grant", checks)
		done <- err
	}()
	select {
	case <-fenceEntered:
	case <-time.After(5 * time.Second):
		disconnect()
		t.Fatal("request never reached quorum fence")
	}
	disconnect()
	select {
	case err := <-done:
		if ErrorStatus(err) != 503 {
			t.Fatalf("disconnect did not fail closed: %v", err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("disconnected request did not terminate")
	}
	blockFence.Store(false)
	t.Run("gRPC unary requires a fresh fence", func(t *testing.T) {
		const provider, org, token = "grant", "conalog-grant", "Bearer token"
		mode := "user"
		pep, err := voter.UpsertProviderPepClient(ctx, provider, "grpc-learner-pep", org, UpsertProviderPepClient{
			Lifecycle: "active", AuthorizationMode: &mode,
		})
		if err != nil {
			t.Fatal(err)
		}
		cap := GRPCCapability{
			CapabilityID: "workspace.get", Version: 1, Enabled: true, FullMethod: "/workspace.Workspaces/Get",
			DescriptorDigest: sha256Text("descriptor"), ActionID: checks[0].Action,
			ResourceType: checks[0].Resource.ResourceType, ResourceID: checks[0].Resource.ID, MaxMessageBytes: 1024,
		}
		cap.ContractDigest, err = GRPCCapabilityContractDigest(cap)
		if err != nil {
			t.Fatal(err)
		}
		stage := StageGRPCCapabilityGeneration{SchemaVersion: 1, Generation: 1, Capabilities: []GRPCCapability{cap}}
		stage.CatalogDigest, err = GRPCCapabilityCatalogDigest(provider, stage)
		if err != nil {
			t.Fatal(err)
		}
		if _, err = voter.StageGRPCCapabilityGeneration(ctx, provider, stage); err != nil {
			t.Fatal(err)
		}
		if _, err = voter.ActivateGRPCCapabilityGeneration(ctx, provider, ActivateGRPCCapabilityGeneration{
			TargetGeneration: stage.Generation, TargetCatalogDigest: stage.CatalogDigest,
		}); err != nil {
			t.Fatal(err)
		}
		grpcCfg := lc
		grpcCfg.Security.HTTPCapabilityPEPClientID = pep.PepClientID
		grpcLearner := NewService(grpcCfg, replica)
		grpcLearner.client = idp.Client()
		in := GRPCCapabilityDecisionInput{
			WireVersion: 1, ProviderID: provider, PEPClientID: pep.PepClientID, PEPPolicyRevision: pep.PolicyRevision,
			CatalogGeneration: stage.Generation, CatalogDigest: stage.CatalogDigest,
			CapabilityID: cap.CapabilityID, CapabilityVersion: cap.Version, ContractDigest: cap.ContractDigest,
			FullMethod: cap.FullMethod, DescriptorDigest: cap.DescriptorDigest, RPCKind: "unary",
			ResourceType: cap.ResourceType, ResourceID: cap.ResourceID, MessageDigest: sha256Text(""),
			ForwardedMetadataDigest: sha256Text(""), ProviderCredentialDigest: CredentialDigest(token),
			ProviderCredential: GRPCCredentialEvidence{Subject: goauthySubject(idp.URL, "subject"), Audience: "https://grant.example.test/"},
			RequestBinding:     "abcdefghijklmnopqrstuv",
		}
		checkGRPC := func(want bool) {
			t.Helper()
			out, err := grpcLearner.DecideGRPCCapability(ctx, token, org, in)
			if err != nil || out.Allowed != want || out.OrganizationID != org {
				t.Fatalf("gRPC decision=%+v err=%v; want allowed=%t", out, err, want)
			}
		}
		checkGRPC(true)
		if _, err = voter.Exec(ctx, "UPDATE role_bindings SET state='disabled' WHERE id='test-binding'"); err != nil {
			t.Fatal(err)
		}
		checkGRPC(false)
		if _, err = voter.Exec(ctx, "UPDATE role_bindings SET state='active' WHERE id='test-binding'"); err != nil {
			t.Fatal(err)
		}
		checkGRPC(true)
		server.Close()
		if out, err := grpcLearner.DecideGRPCCapability(ctx, token, org, in); err == nil || ErrorStatus(err) != 503 || out != (GRPCCapabilityDecision{}) {
			t.Fatalf("gRPC decision without fence: %+v %v", out, err)
		}
		if _, err := grpcLearner.ActiveGRPCCapabilityCatalog(ctx, provider); err == nil || ErrorStatus(err) != 503 {
			t.Fatalf("gRPC catalog without fence: %v", err)
		}
	})
	server.Close()
	check(ctx, false, 503)
}
