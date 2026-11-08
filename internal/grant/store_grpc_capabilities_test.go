package grant

import (
	"context"
	"math"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

func grpcStoreTestStage(t *testing.T, provider string, generation uint64, expected *uint64, capabilities ...GRPCCapability) StageGRPCCapabilityGeneration {
	t.Helper()
	for i := range capabilities {
		var err error
		capabilities[i].ContractDigest, err = GRPCCapabilityContractDigest(capabilities[i])
		if err != nil {
			t.Fatal(err)
		}
	}
	in := StageGRPCCapabilityGeneration{SchemaVersion: 1, Generation: generation, ExpectedLatestGeneration: expected, Capabilities: capabilities}
	var err error
	in.CatalogDigest, err = GRPCCapabilityCatalogDigest(provider, in)
	if err != nil {
		t.Fatal(err)
	}
	return in
}

func TestGRPCStoreCanonicalCatalog(t *testing.T) {
	descriptor := "sha256:" + strings.Repeat("a", 64)
	c := GRPCCapability{
		CapabilityID: "read", Version: 1, Enabled: true, FullMethod: "/Service/Get",
		DescriptorDigest: descriptor, ActionID: "example.read", ResourceType: "file", ResourceID: "fixed", MaxMessageBytes: 2048,
	}
	in := grpcStoreTestStage(t, "example", 1, nil, c)
	const start = `{"actionId":"example.read","capabilityId":"read",`
	tail := `"descriptorDigest":"` + descriptor + `","enabled":true,"fullMethod":"/Service/Get","maxMessageBytes":2048,"resourceId":"fixed","resourceType":"file","version":1}`
	contract := digestBytes([]byte(start + tail))
	if in.Capabilities[0].ContractDigest != contract {
		t.Fatal("contract digest disagrees with canonical fixture")
	}
	payload := `{"capabilities":[` + start + `"contractDigest":"` + contract + `",` + tail + `],"generation":1,"providerId":"example","schemaVersion":1}`
	if in.CatalogDigest != digestBytes([]byte(payload)) {
		t.Fatal("catalog digest disagrees with canonical fixture")
	}
	if _, err := decodeGRPCCatalog("example", 1, in.CatalogDigest, payload); err != nil {
		t.Fatal(err)
	}
	empty := grpcStoreTestStage(t, "example", 1, nil)
	if empty.CatalogDigest != digestBytes([]byte(`{"capabilities":[],"generation":1,"providerId":"example","schemaVersion":1}`)) {
		t.Fatal("empty catalog digest does not use an empty array")
	}
	for _, malformed := range []string{
		strings.Replace(payload, `"enabled":true`, `"enabled":true,"enabled":true`, 1),
		strings.Replace(payload, `"resourceId":"fixed"`, `"resourceId":"fixed","extractor":"field"`, 1),
		strings.Replace(payload, `"version":1`, `"version":1.0`, 1),
		payload + "\n",
	} {
		if _, err := decodeGRPCCatalog("example", 1, digestBytes([]byte(malformed)), malformed); err == nil {
			t.Fatalf("accepted noncanonical payload: %s", malformed)
		}
	}
	other := c
	other.CapabilityID, other.Version = "read-v2", 2
	for _, tc := range []struct {
		name string
		caps []GRPCCapability
	}{
		{"same enabled method across versions and IDs", []GRPCCapability{c, other}},
		{"same capability identity", []GRPCCapability{c, c}},
		{"wildcard", []GRPCCapability{{CapabilityID: "read", Version: 1, Enabled: true, FullMethod: "/Service/*", DescriptorDigest: descriptor, ActionID: "example.read", ResourceType: "file", ResourceID: "fixed", MaxMessageBytes: 2048}}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			in := grpcStoreTestStage(t, "example", 1, nil, tc.caps...)
			out := GRPCCatalog{in.SchemaVersion, "example", in.Generation, in.CatalogDigest, in.Capabilities}
			if _, err := validateGRPCCatalog(out); err == nil {
				t.Fatal("accepted invalid gRPC catalog")
			}
		})
	}
	other.Enabled = false
	in = grpcStoreTestStage(t, "example", 1, nil, c, other)
	if _, err := validateGRPCCatalog(GRPCCatalog{in.SchemaVersion, "example", in.Generation, in.CatalogDigest, in.Capabilities}); err != nil {
		t.Fatalf("disabled version made an enabled method ambiguous: %v", err)
	}
}

func TestGRPCStoreGenerationCASAndOwnership(t *testing.T) {
	ctx := context.Background()
	s, err := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir()}})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = s.Close() })
	exec := func(query string, args ...any) {
		t.Helper()
		if _, err := s.Exec(ctx, query, args...); err != nil {
			t.Fatal(err)
		}
	}
	const provider = "grpc-provider"
	now := nowString()
	for _, id := range []string{provider, "other"} {
		exec("INSERT INTO service_providers(id,name,created_at,updated_at) VALUES(?,?,?,?)", id, id, now, now)
	}
	for _, row := range []struct{ id, owner string }{{"grpc.read", provider}, {"grpc.write", provider}, {"other.read", "other"}} {
		exec("INSERT INTO actions(id,provider_id,code,name,risk_level,created_at,updated_at) VALUES(?,?,?,?,'low',?,?)", row.id, row.owner, row.id, row.id, now, now)
	}
	capability := GRPCCapability{
		CapabilityID: "read", Version: 1, Enabled: true, FullMethod: "/example.Files/Read",
		DescriptorDigest: digestBytes([]byte("descriptor")), ActionID: "grpc.read", ResourceType: "file", ResourceID: "fixed", MaxMessageBytes: 4096,
	}
	first := grpcStoreTestStage(t, provider, 1, nil, capability)
	if _, err := s.StageGRPCCapabilityGeneration(ctx, provider, first); err != nil {
		t.Fatal(err)
	}
	if _, err := s.StageGRPCCapabilityGeneration(ctx, provider, first); err != nil {
		t.Fatalf("identical stage retry failed: %v", err)
	}
	conflicting := capability
	conflicting.ResourceID = "changed"
	if _, err := s.StageGRPCCapabilityGeneration(ctx, provider, grpcStoreTestStage(t, provider, 1, nil, conflicting)); err == nil || ErrorStatus(err) != 409 {
		t.Fatalf("immutable stage conflict: %v", err)
	}
	if _, err := s.ActiveGRPCCapabilityCatalog(ctx, provider); err == nil || ErrorStatus(err) != 404 {
		t.Fatalf("staging changed the active pointer: %v", err)
	}
	firstActivation := ActivateGRPCCapabilityGeneration{TargetGeneration: 1, TargetCatalogDigest: first.CatalogDigest}
	if _, err := s.ActivateGRPCCapabilityGeneration(ctx, provider, firstActivation); err != nil {
		t.Fatal(err)
	}
	svc := &Service{store: s}
	if err := svc.DeleteProviderAction(ctx, provider, "grpc.read"); err == nil || ErrorStatus(err) != 409 {
		t.Fatalf("provider delete did not protect active gRPC action: %v", err)
	}
	if _, err := svc.ReplaceProviderActionsAdmin(ctx, provider, nil); err == nil || ErrorStatus(err) != 409 {
		t.Fatalf("provider replacement did not protect active gRPC action: %v", err)
	}
	const changeOwner = "UPDATE actions SET provider_id='other' WHERE id=?"
	if _, err := s.Exec(ctx, changeOwner, "grpc.read"); err == nil {
		t.Fatal("rekey of an actively referenced action succeeded")
	}
	if _, err := s.Exec(ctx, "DELETE FROM actions WHERE id='grpc.read'"); err == nil {
		t.Fatal("deletion of an actively referenced action succeeded")
	}

	previous := uint64(1)
	if _, err := s.StageGRPCCapabilityGeneration(ctx, provider, grpcStoreTestStage(t, provider, 3, &previous, capability)); err == nil || ErrorStatus(err) != 409 {
		t.Fatalf("staging accepted a gap: %v", err)
	}
	foreign := capability
	foreign.ActionID = "other.read"
	if _, err := s.StageGRPCCapabilityGeneration(ctx, provider, grpcStoreTestStage(t, provider, 2, &previous, foreign)); err == nil || ErrorStatus(err) != 409 {
		t.Fatalf("cross-provider stage: %v", err)
	}
	next := capability
	next.ActionID = "grpc.write"
	second := grpcStoreTestStage(t, provider, 2, &previous, next)
	if _, err := s.StageGRPCCapabilityGeneration(ctx, provider, second); err != nil {
		t.Fatal(err)
	}
	activate := ActivateGRPCCapabilityGeneration{ExpectedGeneration: &previous, ExpectedCatalogDigest: &first.CatalogDigest, TargetGeneration: 2, TargetCatalogDigest: second.CatalogDigest}
	exec("UPDATE service_providers SET active=0 WHERE id=?", provider)
	if _, err := s.ActivateGRPCCapabilityGeneration(ctx, provider, activate); err == nil || ErrorStatus(err) != 409 {
		t.Fatalf("inactive provider activated a catalog: %v", err)
	}
	if _, err := s.ActiveGRPCCapabilityCatalog(ctx, provider); err == nil || ErrorStatus(err) != 404 {
		t.Fatalf("inactive provider catalog remained readable: %v", err)
	}
	exec("UPDATE service_providers SET active=1 WHERE id=?", provider)
	exec(changeOwner, "grpc.write")
	if _, err := s.ActivateGRPCCapabilityGeneration(ctx, provider, activate); err == nil || ErrorStatus(err) != 409 {
		t.Fatalf("activation accepted a staged action now owned by a different provider: %v", err)
	}
	exec("UPDATE actions SET provider_id=? WHERE id='grpc.write'", provider)
	want, err := s.ActivateGRPCCapabilityGeneration(ctx, provider, activate)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := s.ActivateGRPCCapabilityGeneration(ctx, provider, activate); err == nil || ErrorStatus(err) != 409 {
		t.Fatalf("stale activation CAS: %v", err)
	}
	if got, err := s.ActiveGRPCCapabilityCatalog(ctx, provider); err != nil || !reflect.DeepEqual(got, want) {
		t.Fatalf("active catalog: %+v, %v", got, err)
	}
	scope := &replicaRead{fence: math.MaxUint64}
	if _, err := s.ActiveGRPCCapabilityCatalog(context.WithValue(ctx, replicaReadKey{}, scope), provider); err == nil || ErrorStatus(err) != 503 || !scope.changed {
		t.Fatalf("catalog ignored the caller's fence: %v", err)
	}

	rollback := firstActivation
	rollback.ExpectedGeneration, rollback.ExpectedCatalogDigest = &want.Generation, &want.CatalogDigest
	if got, err := s.ActivateGRPCCapabilityGeneration(ctx, provider, rollback); err != nil || got.CatalogDigest != first.CatalogDigest {
		t.Fatalf("rollback with a registered action failed: %+v, %v", got, err)
	}
	if _, err := s.ActivateGRPCCapabilityGeneration(ctx, provider, activate); err != nil {
		t.Fatal(err)
	}
	if err := svc.DeleteProviderAction(ctx, provider, "grpc.read"); err != nil {
		t.Fatalf("provider delete blocked retirement of an inactive action: %v", err)
	}
	if _, err := s.ActivateGRPCCapabilityGeneration(ctx, provider, rollback); err == nil || ErrorStatus(err) != 409 {
		t.Fatalf("retired action could be reactivated: %v", err)
	}
	snapshot, err := s.ExportSnapshot(ctx)
	if err != nil {
		t.Fatal(err)
	}
	cfg := Config{Database: DatabaseConfig{DataDir: filepath.Join(t.TempDir(), "restore")}}
	if err = RestoreStoreSnapshot(ctx, cfg, snapshot); err != nil {
		t.Fatal(err)
	}
	restored, err := OpenStore(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = restored.Close() })
	if got, err := restored.ActiveGRPCCapabilityCatalog(ctx, provider); err != nil || !reflect.DeepEqual(got, want) {
		t.Fatalf("restored active catalog: %+v, %v", got, err)
	}
	restoredSvc := &Service{store: restored}
	if err := restoredSvc.DeleteProviderAction(ctx, provider, "grpc.write"); err == nil || ErrorStatus(err) != 409 {
		t.Fatalf("restored store allowed deletion of its active action: %v", err)
	}
	if _, err := restoredSvc.ReplaceProviderActionsAdmin(ctx, provider, nil); err == nil || ErrorStatus(err) != 409 {
		t.Fatalf("restored store allowed replacement to remove its active action: %v", err)
	}
	if _, err := restored.Exec(ctx, "DELETE FROM actions WHERE id='grpc.write'"); err == nil {
		t.Fatal("restored schema did not enforce the active action trigger")
	}
	if _, err := restored.ActivateGRPCCapabilityGeneration(ctx, provider, rollback); err == nil || ErrorStatus(err) != 409 {
		t.Fatalf("restored history reactivated a retired action: %v", err)
	}
	for i := range snapshot.Tables {
		table := &snapshot.Tables[i]
		if table.Name != "provider_grpc_capability_active" {
			continue
		}
		table.Rows[0][2] = first.CatalogDigest // Valid digest for the wrong generation.
		table.Digest, err = tableDigest(*table)
		if err != nil {
			t.Fatal(err)
		}
	}
	snapshot.Digest, err = snapshotDigest(snapshot)
	if err != nil {
		t.Fatal(err)
	}
	if err := ValidateStoreSnapshot(snapshot); err == nil || !strings.Contains(err.Error(), "active pointer") {
		t.Fatalf("snapshot admitted mismatched pointer: %v", err)
	}
	for i := range snapshot.Tables {
		table := &snapshot.Tables[i]
		switch table.Name {
		case "provider_grpc_capability_active":
			table.Rows[0][2] = want.CatalogDigest
		case "actions":
			for _, row := range table.Rows {
				if row[0] == "grpc.write" {
					row[1] = "other"
				}
			}
		default:
			continue
		}
		table.Digest, err = tableDigest(*table)
		if err != nil {
			t.Fatal(err)
		}
	}
	snapshot.Digest, err = snapshotDigest(snapshot)
	if err != nil {
		t.Fatal(err)
	}
	if err := ValidateStoreSnapshot(snapshot); err == nil || !strings.Contains(err.Error(), "active action owner") {
		t.Fatalf("snapshot admitted an active action owned by another provider: %v", err)
	}
}
