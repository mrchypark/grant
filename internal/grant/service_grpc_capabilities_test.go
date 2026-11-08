package grant

import (
	"context"
	"strings"
	"testing"
	"time"
)

func TestGRPCPEPLeaseExpiryAtIssuance(t *testing.T) {
	now := time.Date(2026, 11, 8, 12, 0, 0, 0, time.UTC)
	for _, tc := range []struct {
		name, lifecycle string
		validUntil      string
		ttl, want       time.Duration
	}{
		{"no PEP expiry", "active", "", 5 * time.Second, 5 * time.Second},
		{"PEP outlasts receipt", "active", now.Add(6 * time.Second).Format(time.RFC3339Nano), 5 * time.Second, 5 * time.Second},
		{"receipt clipped", "active", now.Add(time.Nanosecond).Format(time.RFC3339Nano), 5 * time.Second, time.Nanosecond},
		{"catalog clipped", "active", now.Add(20 * time.Second).Format(time.RFC3339Nano), 30 * time.Second, 20 * time.Second},
		{"expires exactly at issuance", "active", now.Format(time.RFC3339Nano), 5 * time.Second, 0},
		{"expired before issuance", "active", now.Add(-time.Nanosecond).Format(time.RFC3339Nano), 30 * time.Second, 0},
		{"malformed expiry", "active", "invalid", 5 * time.Second, 0},
		{"inactive PEP", "disabled", "", 5 * time.Second, 0},
	} {
		t.Run(tc.name, func(t *testing.T) {
			pep := ProviderPepClient{Lifecycle: tc.lifecycle}
			if tc.validUntil != "" {
				pep.ValidUntil = &tc.validUntil
			}
			expires, err := grpcPEPLeaseExpiry(pep, now, tc.ttl)
			if tc.want == 0 {
				if err == nil || ErrorStatus(err) != 403 || !expires.IsZero() {
					t.Fatalf("issued an invalid lease: %s %v", expires, err)
				}
			} else if err != nil || !expires.Equal(now.Add(tc.want)) {
				t.Fatalf("expiry=%s err=%v, want %s", expires, err, now.Add(tc.want))
			}
		})
	}
}

func TestGRPCCapabilityDecisionAndRevocation(t *testing.T) {
	ctx := context.Background()
	f := newPrincipalHTTPFixture(t)
	s := f.svc
	const provider, org, token = "patch-data", "conalog-grant", "Bearer token"
	cap := GRPCCapability{
		CapabilityID: "files.update", Version: 1, Enabled: true, FullMethod: "/patch.Files/Update",
		DescriptorDigest: sha256Text("descriptor"), ActionID: *f.caps["files.update.0"].ActionID,
		ResourceType: "files", ResourceID: "aaaaaaaaaaaaaaa", MaxMessageBytes: 16,
	}
	var err error
	cap.ContractDigest, err = GRPCCapabilityContractDigest(cap)
	if err != nil {
		t.Fatal(err)
	}
	stage := StageGRPCCapabilityGeneration{SchemaVersion: 1, Generation: 1, Capabilities: []GRPCCapability{cap}}
	stage.CatalogDigest, err = GRPCCapabilityCatalogDigest(provider, stage)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := s.StageGRPCCapabilityGenerationAdmin(ctx, provider, stage); err != nil {
		t.Fatal(err)
	}
	if _, err := s.ActivateGRPCCapabilityGenerationAdmin(ctx, provider, ActivateGRPCCapabilityGeneration{
		TargetGeneration: 1, TargetCatalogDigest: stage.CatalogDigest,
	}); err != nil {
		t.Fatal(err)
	}
	catalog, err := s.ActiveGRPCCapabilityCatalog(ctx, provider)
	if err != nil || catalog.SchemaVersion != 1 || catalog.ProviderID != provider || catalog.Generation != 1 ||
		catalog.CatalogDigest != stage.CatalogDigest || catalog.AuthorizationMode != "user" ||
		catalog.PEPClientID != "batch-pep" || catalog.PEPPolicyRevision != 1 ||
		len(catalog.Capabilities) != 1 || catalog.Capabilities[0] != cap {
		t.Fatalf("bad catalog: %+v %v", catalog, err)
	}
	checkGRPCLifetime(t, catalog.IssuedAt, catalog.ExpiresAt, 30*time.Second)
	in := GRPCCapabilityDecisionInput{
		WireVersion: 1, ProviderID: provider, PEPClientID: catalog.PEPClientID, PEPPolicyRevision: catalog.PEPPolicyRevision,
		CatalogGeneration: catalog.Generation, CatalogDigest: catalog.CatalogDigest,
		CapabilityID: cap.CapabilityID, CapabilityVersion: cap.Version, ContractDigest: cap.ContractDigest,
		FullMethod: cap.FullMethod, DescriptorDigest: cap.DescriptorDigest, RPCKind: "unary",
		ResourceType: cap.ResourceType, ResourceID: cap.ResourceID, MessageDigest: sha256Text(""), MessageLength: 0,
		ForwardedMetadataDigest: sha256Text(""), ProviderCredentialDigest: CredentialDigest(token),
		ProviderCredential: GRPCCredentialEvidence{Subject: goauthySubject(f.issuer, "principal-user"), Audience: "https://grant.example.test/"},
		RequestBinding:     "abcdefghijklmnopqrstuv",
	}
	decide := func(input GRPCCapabilityDecisionInput, allowed bool, reason string) {
		t.Helper()
		before := f.calls.Load()
		out, err := s.DecideGRPCCapability(ctx, token, org, input)
		if err != nil || out.Allowed != allowed || out.Reason != reason ||
			out.GRPCCapabilityDecisionInput != input || out.OrganizationID != org || out.ActionID != cap.ActionID ||
			out.PrincipalDigest != ExposurePrincipalDigest(org, org, "") {
			t.Fatalf("bad decision: %+v %v", out, err)
		}
		checkGRPCLifetime(t, out.IssuedAt, out.ExpiresAt, 5*time.Second)
		if f.calls.Load() != before+1 {
			t.Fatal("expected one fresh identity verification per decision")
		}
	}
	decide(in, false, "no_matching_binding")
	f.bind(t, "grpc-file", in.ResourceType, in.ResourceID)
	decide(in, true, "role_binding")
	nonempty := in
	nonempty.MessageDigest, nonempty.MessageLength = sha256Text("\x0a\x01x"), 3
	decide(nonempty, true, "role_binding")

	for _, tc := range []struct {
		name   string
		change func(*GRPCCapabilityDecisionInput)
		status int
	}{
		{"HTTP wire version", func(v *GRPCCapabilityDecisionInput) { v.WireVersion = 2 }, 400},
		{"streaming", func(v *GRPCCapabilityDecisionInput) { v.RPCKind = "server-streaming" }, 400},
		{"zero revision", func(v *GRPCCapabilityDecisionInput) { v.PEPPolicyRevision = 0 }, 400},
		{"different provider", func(v *GRPCCapabilityDecisionInput) { v.ProviderID = "grant" }, 403},
		{"wrong PEP", func(v *GRPCCapabilityDecisionInput) { v.PEPClientID = "other" }, 403},
		{"stale PEP revision", func(v *GRPCCapabilityDecisionInput) { v.PEPPolicyRevision++ }, 403},
		{"stale generation", func(v *GRPCCapabilityDecisionInput) { v.CatalogGeneration++ }, 403},
		{"wrong catalog digest", func(v *GRPCCapabilityDecisionInput) { v.CatalogDigest = sha256Text("old") }, 403},
		{"wrong capability version", func(v *GRPCCapabilityDecisionInput) { v.CapabilityVersion++ }, 403},
		{"wrong contract digest", func(v *GRPCCapabilityDecisionInput) { v.ContractDigest = sha256Text("other") }, 403},
		{"wrong method", func(v *GRPCCapabilityDecisionInput) { v.FullMethod = "/patch.Files/Get" }, 403},
		{"wrong descriptor", func(v *GRPCCapabilityDecisionInput) { v.DescriptorDigest = sha256Text("other") }, 403},
		{"wrong constant resource", func(v *GRPCCapabilityDecisionInput) { v.ResourceID = "other" }, 403},
		{"oversized message", func(v *GRPCCapabilityDecisionInput) { v.MessageLength = cap.MaxMessageBytes + 1 }, 403},
		{"malformed message digest", func(v *GRPCCapabilityDecisionInput) { v.MessageDigest = "invalid" }, 400},
		{"application metadata", func(v *GRPCCapabilityDecisionInput) { v.ForwardedMetadataDigest = sha256Text("metadata") }, 403},
		{"token-only digest", func(v *GRPCCapabilityDecisionInput) { v.ProviderCredentialDigest = CredentialDigest("token") }, 403},
		{"forged subject", func(v *GRPCCapabilityDecisionInput) { v.ProviderCredential.Subject = "other" }, 403},
		{"forged audience", func(v *GRPCCapabilityDecisionInput) { v.ProviderCredential.Audience = "other" }, 403},
		{"invalid request binding", func(v *GRPCCapabilityDecisionInput) { v.RequestBinding = "invalid" }, 400},
	} {
		t.Run(tc.name, func(t *testing.T) {
			v := nonempty
			tc.change(&v)
			out, err := s.DecideGRPCCapability(ctx, token, org, v)
			if err == nil || ErrorStatus(err) != tc.status || out != (GRPCCapabilityDecision{}) {
				t.Fatalf("invalid request produced %+v %v; want status %d", out, err, tc.status)
			}
		})
	}
	if _, err := s.DecideGRPCCapability(ctx, token, "wrong-org", in); err == nil || ErrorStatus(err) != 403 {
		t.Fatalf("wrong organization: %v", err)
	}

	// Revoke after the catalog read. The decision must retry against one snapshot,
	// without re-verifying the external credential on the local retry.
	catalogReads := 0
	s.store.afterReplicaQuery = func(_ context.Context, sql string) {
		if strings.Contains(sql, "provider_grpc_capability_active") {
			catalogReads++
			if catalogReads == 1 {
				f.exec(t, "UPDATE role_bindings SET state='revoked' WHERE id='grpc-file'")
			}
		}
	}
	decide(in, false, "no_matching_binding")
	s.store.afterReplicaQuery = nil
	if catalogReads < 2 {
		t.Fatalf("expected the changed snapshot to be retried, got %d catalog reads", catalogReads)
	}

	f.exec(t, "UPDATE provider_pep_clients SET authorization_mode='application',policy_revision=2 WHERE pep_client_id='batch-pep'")
	in.PEPPolicyRevision = 2
	if _, err := s.DecideGRPCCapability(ctx, token, org, in); err == nil || ErrorStatus(err) != 403 {
		t.Fatalf("application mode decision: %v", err)
	}
	if _, err := s.ActiveGRPCCapabilityCatalog(ctx, provider); err == nil || ErrorStatus(err) != 403 {
		t.Fatalf("application mode catalog: %v", err)
	}
	// A service without local authority must never fall back to a remote read.
	unfenced := NewService(s.cfg, &Store{})
	unfenced.client = s.client
	if _, err := unfenced.DecideGRPCCapability(ctx, token, org, in); err == nil || ErrorStatus(err) != 503 {
		t.Fatalf("unfenced decision: %v", err)
	}
}

func checkGRPCLifetime(t *testing.T, issuedAt, expiresAt string, want time.Duration) {
	t.Helper()
	issued, first := time.Parse(time.RFC3339Nano, issuedAt)
	expires, second := time.Parse(time.RFC3339Nano, expiresAt)
	if first != nil || second != nil || expires.Sub(issued) != want {
		t.Fatalf("bad lifetime %q to %q, want %v", issuedAt, expiresAt, want)
	}
}
