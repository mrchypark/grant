package api

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"strings"
	"testing"
	"time"

	pdppb "github.com/mrchypark/grant/api/pdp/v1"
	"github.com/mrchypark/grant/internal/grant"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
	"google.golang.org/grpc/test/bufconn"
	"google.golang.org/protobuf/proto"
)

func newPDPTestClient(t *testing.T, service *grant.Service) pdppb.PolicyDecisionPointClient {
	t.Helper()
	listener := bufconn.Listen(64 * 1024)
	server := grpc.NewServer(grpc.MaxRecvMsgSize(PDPGRPCMaxWireBytes), grpc.MaxSendMsgSize(PDPGRPCMaxWireBytes))
	RegisterPDPGRPC(server, service)
	done := make(chan struct{})
	go func() {
		defer close(done)
		_ = server.Serve(listener)
	}()
	t.Cleanup(func() {
		server.Stop()
		_ = listener.Close()
		<-done
	})
	conn, err := grpc.NewClient("passthrough:///grant-pdp-test",
		grpc.WithTransportCredentials(insecure.NewCredentials()),
		grpc.WithContextDialer(func(ctx context.Context, _ string) (net.Conn, error) {
			return listener.DialContext(ctx)
		}))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = conn.Close() })
	return pdppb.NewPolicyDecisionPointClient(conn)
}

func TestPDPGRPCMetadataAndStrictPayload(t *testing.T) {
	client := newPDPTestClient(t, testServer().service)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	callContext := func(values ...string) context.Context {
		return metadata.NewOutgoingContext(ctx, metadata.Pairs(values...))
	}
	app := []string{"x-grant-app-auth-key", "app-secret"}
	bearer := []string{"authorization", "Bearer tok"}
	org := []string{"x-organization-id", "org"}
	decisionCtx := callContext("x-grant-app-auth-key", "app-secret", "authorization", "Bearer tok", "x-organization-id", "org")
	catalog := &pdppb.CatalogRequest{ProviderId: "provider"}
	for _, call := range []func(context.Context, *pdppb.CatalogRequest, ...grpc.CallOption) (*pdppb.CatalogResponse, error){
		client.GetHttpCatalog, client.GetGrpcCatalog,
	} {
		if _, err := call(ctx, catalog); status.Code(err) != codes.Unauthenticated {
			t.Fatalf("missing app key: %v", err)
		}
		if _, err := call(callContext("x-grant-app-auth-key", "wrong"), catalog); status.Code(err) != codes.Unauthenticated {
			t.Fatalf("wrong app key: %v", err)
		}
		if _, err := call(callContext(app...), &pdppb.CatalogRequest{}); status.Code(err) != codes.InvalidArgument {
			t.Fatalf("missing provider: %v", err)
		}
	}
	unknownCatalog := &pdppb.CatalogRequest{ProviderId: "provider"}
	unknownCatalog.ProtoReflect().SetUnknown([]byte{0x10, 1}) // Unknown field 2.
	if _, err := client.GetGrpcCatalog(callContext(app...), unknownCatalog); status.Code(err) != codes.InvalidArgument {
		t.Fatalf("unknown protobuf field: %v", err)
	}
	negativeRevision := pdpTestJSON(t, grant.HttpCapabilityDecisionInput{WireVersion: 2, PEPPolicyRevision: -1})
	grpcEvidence := pdpTestJSON(t, grant.GRPCCapabilityDecisionInput{})
	for _, tc := range []struct {
		name string
		ctx  context.Context
		raw  []byte
		code codes.Code
	}{
		{"key before parsing", ctx, []byte("{"), codes.Unauthenticated},
		{"duplicate key", callContext(append(append([]string{}, app...), app...)...), grpcEvidence, codes.InvalidArgument},
		{"missing bearer", callContext(append(append([]string{}, app...), org...)...), grpcEvidence, codes.Unauthenticated},
		{"duplicate bearer", metadata.AppendToOutgoingContext(decisionCtx, bearer...), grpcEvidence, codes.InvalidArgument},
		{"malformed bearer", callContext("x-grant-app-auth-key", "app-secret", "authorization", "Basic credentials"), grpcEvidence, codes.InvalidArgument},
		{"missing org", callContext(append(append([]string{}, app...), bearer...)...), grpcEvidence, codes.InvalidArgument},
		{"duplicate org", metadata.AppendToOutgoingContext(decisionCtx, org...), grpcEvidence, codes.InvalidArgument},
		{"missing JSON fields", decisionCtx, []byte("{}"), codes.InvalidArgument},
		{"unknown JSON field", decisionCtx, []byte(`{"notAllowed":"do-not-leak",` + string(grpcEvidence[1:])), codes.InvalidArgument},
		{"duplicate JSON field", decisionCtx, []byte(`{"wireVersion":1,` + string(grpcEvidence[1:])), codes.InvalidArgument},
		{"evidence over limit", decisionCtx, make([]byte, maxJSONBodyBytes+1), codes.ResourceExhausted},
	} {
		t.Run(tc.name, func(t *testing.T) {
			_, err := client.DecideGrpc(tc.ctx, &pdppb.DecisionRequest{EvidenceJson: tc.raw})
			if status.Code(err) != tc.code || strings.Contains(fmt.Sprint(err), "do-not-leak") {
				t.Fatalf("status=%v, want %s", err, tc.code)
			}
		})
	}
	if _, err := client.DecideHttp(callContext("x-grant-app-auth-key", "app-secret", "authorization", "Bearer tok"),
		&pdppb.DecisionRequest{EvidenceJson: negativeRevision}); status.Code(err) != codes.InvalidArgument {
		t.Fatalf("HTTP negative revision: %v", err)
	}
	unknownDecision := &pdppb.DecisionRequest{EvidenceJson: grpcEvidence}
	unknownDecision.ProtoReflect().SetUnknown([]byte{0x10, 1})
	if _, err := client.DecideGrpc(decisionCtx, unknownDecision); status.Code(err) != codes.InvalidArgument {
		t.Fatalf("unknown decision protobuf field: %v", err)
	}
	t.Run("malformed protobuf fails closed", func(t *testing.T) {
		out, err := client.GetGrpcCatalog(callContext(app...), catalog, grpc.ForceCodec(truncatedPDPProto{}))
		if err == nil || out != nil {
			t.Fatalf("malformed protobuf yielded a successful RPC: response=%v err=%v", out, err)
		}
	})
}

// Use the normal protobuf decoder on the server and for any client response.
type truncatedPDPProto struct{}

func (truncatedPDPProto) Name() string { return "proto" }
func (truncatedPDPProto) Marshal(any) ([]byte, error) {
	return []byte{0x0a, 0x02, 'x'}, nil // Field 1 declares two bytes but contains one.
}
func (truncatedPDPProto) Unmarshal(data []byte, out any) error {
	return proto.Unmarshal(data, out.(proto.Message))
}

func TestPDPGRPCUsesGrantCatalogAndRBAC(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 40*time.Second)
	defer cancel()
	fixture := newPrincipalExposureSystem(t, ctx)
	httpDigest := fixture.publish(t, "user")
	svc := grant.NewService(fixture.config, fixture.store)
	client := newPDPTestClient(t, svc)
	app := metadata.AppendToOutgoingContext(ctx, "x-grant-app-auth-key", fixture.config.Security.AppAuthKey)
	const org = "fixture-owner"
	const subject = "resource-owner"
	const audience = "https://grant.example.test/"
	const binding = "abcdefghijklmnopqrstuv"
	bearer := "Bearer " + fixture.tokens["user"]
	decisionCtx := metadata.AppendToOutgoingContext(app, "authorization", bearer, "x-organization-id", org)
	provider := fixture.providerID
	catalogReq := &pdppb.CatalogRequest{ProviderId: provider}
	httpCatalogResponse, err := client.GetHttpCatalog(app, catalogReq)
	if err != nil {
		t.Fatal(err)
	}
	var httpCatalog grant.ActiveHttpCapabilityCatalog
	if err := json.Unmarshal(httpCatalogResponse.GetCatalogJson(), &httpCatalog); err != nil ||
		httpCatalog.CatalogDigest != httpDigest || len(httpCatalog.Capabilities) != 1 || httpCatalog.AuthorizationMode != "user" {
		t.Fatalf("HTTP catalog: %+v %v", httpCatalog, err)
	}

	cap := grant.GRPCCapability{
		CapabilityID: "item-read-grpc", Version: 1, Enabled: true, FullMethod: "/inventory.Items/Get",
		DescriptorDigest: pdpTestDigest("descriptor"), ActionID: "fixture-provider.item.read",
		ResourceType: "item", ResourceID: "allowed", MaxMessageBytes: 64,
	}
	cap.ContractDigest, err = grant.GRPCCapabilityContractDigest(cap)
	if err != nil {
		t.Fatal(err)
	}
	stage := grant.StageGRPCCapabilityGeneration{SchemaVersion: 1, Generation: 1, Capabilities: []grant.GRPCCapability{cap}}
	stage.CatalogDigest, err = grant.GRPCCapabilityCatalogDigest(provider, stage)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := svc.StageGRPCCapabilityGenerationAdmin(ctx, provider, stage); err != nil {
		t.Fatal(err)
	}
	if _, err := svc.ActivateGRPCCapabilityGenerationAdmin(ctx, provider, grant.ActivateGRPCCapabilityGeneration{
		TargetGeneration: stage.Generation, TargetCatalogDigest: stage.CatalogDigest,
	}); err != nil {
		t.Fatal(err)
	}
	grpcCatalogResponse, err := client.GetGrpcCatalog(app, catalogReq)
	if err != nil {
		t.Fatal(err)
	}
	var grpcCatalog grant.ActiveGRPCCatalog
	if err := json.Unmarshal(grpcCatalogResponse.GetCatalogJson(), &grpcCatalog); err != nil ||
		grpcCatalog.CatalogDigest != stage.CatalogDigest || len(grpcCatalog.Capabilities) != 1 || grpcCatalog.Capabilities[0] != cap {
		t.Fatalf("gRPC catalog: %+v %v", grpcCatalog, err)
	}

	verifiedSubject := "goauthy:" + fixture.issuer.URL + "/" + subject
	emptyDigest := pdpTestDigest("")
	pathParam := "itemId"
	httpEvidence := grant.HttpCapabilityDecisionInput{
		WireVersion: 2, PEPPolicyRevision: httpCatalog.PEPPolicyRevision,
		ProviderID: provider, PEPClientID: httpCatalog.PepClientID,
		CatalogGeneration: httpCatalog.Generation, CatalogDigest: httpDigest,
		CapabilityID: "item-read", CapabilityVersion: 1, ContractDigest: httpCatalog.Capabilities[0].ContractDigest,
		Method: "GET", CanonicalPath: "/items/allowed", CanonicalQueryDigest: emptyDigest,
		ForwardedHeaderDigest: emptyDigest, BodyDigest: emptyDigest,
		ProviderCredentialDigest: grant.CredentialDigest(bearer),
		ProviderCredential:       grant.HttpCapabilityCredentialEvidence{Subject: verifiedSubject, Audience: audience},
		ResourceEvidence:         grant.HttpCapabilityResourceEvidence{Kind: "pathSegment", Name: &pathParam, CanonicalValue: "allowed"},
		ResourceType:             "item", ResourceID: "allowed", RequestBinding: binding,
	}
	grpcEvidence := grant.GRPCCapabilityDecisionInput{
		WireVersion: 1, PEPPolicyRevision: grpcCatalog.PEPPolicyRevision,
		ProviderID: provider, PEPClientID: grpcCatalog.PEPClientID,
		CatalogGeneration: grpcCatalog.Generation, CatalogDigest: grpcCatalog.CatalogDigest,
		CapabilityID: cap.CapabilityID, CapabilityVersion: cap.Version, ContractDigest: cap.ContractDigest,
		FullMethod: cap.FullMethod, DescriptorDigest: cap.DescriptorDigest, RPCKind: "unary",
		ResourceType: cap.ResourceType, ResourceID: cap.ResourceID, MessageDigest: emptyDigest,
		ForwardedMetadataDigest: emptyDigest, ProviderCredentialDigest: grant.CredentialDigest(bearer),
		ProviderCredential: grant.GRPCCredentialEvidence{Subject: verifiedSubject, Audience: audience},
		RequestBinding:     binding,
	}
	assertDecision := func(wantAllowed bool) {
		t.Helper()
		h, err := client.DecideHttp(decisionCtx, &pdppb.DecisionRequest{EvidenceJson: pdpTestJSON(t, httpEvidence)})
		if err != nil {
			t.Fatal(err)
		}
		var httpDecision grant.HttpCapabilityDecision
		if err := json.Unmarshal(h.GetDecisionJson(), &httpDecision); err != nil || httpDecision.Allowed != wantAllowed ||
			httpDecision.RequestBinding != binding || httpDecision.ProviderCredentialDigest != httpEvidence.ProviderCredentialDigest {
			t.Fatalf("HTTP decision: %+v %v", httpDecision, err)
		}
		g, err := client.DecideGrpc(decisionCtx, &pdppb.DecisionRequest{EvidenceJson: pdpTestJSON(t, grpcEvidence)})
		if err != nil {
			t.Fatal(err)
		}
		var grpcDecision grant.GRPCCapabilityDecision
		if err := json.Unmarshal(g.GetDecisionJson(), &grpcDecision); err != nil || grpcDecision.Allowed != wantAllowed ||
			grpcDecision.OrganizationID != org || grpcDecision.GRPCCapabilityDecisionInput != grpcEvidence {
			t.Fatalf("gRPC decision: %+v %v", grpcDecision, err)
		}
	}
	assertDecision(true)
	fixture.revokeResourceRole(t)
	assertDecision(false)
}

func TestPDPGRPCResponseBoundsAndContext(t *testing.T) {
	for _, limit := range []int{pdpMaxDecisionJSON, maxJSONBodyBytes} {
		if raw, err := pdpEncode(context.Background(), strings.Repeat("x", limit-2), nil, limit); err != nil || len(raw) != limit {
			t.Fatalf("exact JSON limit %d: len=%d err=%v", limit, len(raw), err)
		}
		if _, err := pdpEncode(context.Background(), strings.Repeat("x", limit-1), nil, limit); status.Code(err) != codes.ResourceExhausted {
			t.Fatalf("over JSON limit %d: %v", limit, err)
		}
	}
	s := &pdpGRPC{service: testServer().service}
	md := metadata.Pairs("x-grant-app-auth-key", "app-secret")
	for _, budget := range []time.Duration{time.Minute, time.Second} {
		parent, cancel := context.WithTimeout(metadata.NewIncomingContext(context.Background(), md), budget)
		parentDeadline, _ := parent.Deadline()
		_, err := pdpCatalog(parent, s, &pdppb.CatalogRequest{ProviderId: "provider"}, func(ctx context.Context, _ string) (int, error) {
			deadline, ok := ctx.Deadline()
			if !ok || (budget < pdpRPCTimeout && !deadline.Equal(parentDeadline)) ||
				(budget > pdpRPCTimeout && (deadline.After(time.Now().Add(pdpRPCTimeout)) || !deadline.Before(parentDeadline))) {
				t.Fatalf("unexpected RPC deadline: %v (parent %v)", deadline, parentDeadline)
			}
			cancel()
			if !errors.Is(ctx.Err(), context.Canceled) {
				t.Fatal("domain context did not receive cancellation")
			}
			return 1, nil
		})
		cancel()
		if status.Code(err) != codes.Canceled {
			t.Fatalf("canceled RPC returned result: %v", err)
		}
	}
}

func pdpTestJSON(t *testing.T, value any) []byte {
	t.Helper()
	raw, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	return raw
}

func pdpTestDigest(value string) string {
	return fmt.Sprintf("sha256:%x", sha256.Sum256([]byte(value)))
}
