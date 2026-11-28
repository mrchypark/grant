package api

import (
	"context"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/json"
	"errors"
	"strings"
	"time"

	pdppb "github.com/mrchypark/grant/api/pdp/v1"
	"github.com/mrchypark/grant/internal/grant"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
)

const (
	// PDPGRPCMaxWireBytes bounds the gRPC message, including protobuf overhead.
	PDPGRPCMaxWireBytes = 2*1024*1024 + 1024
	pdpMaxDecisionJSON  = 64 * 1024
	pdpRPCTimeout       = 10 * time.Second
)

type pdpGRPC struct {
	pdppb.UnimplementedPolicyDecisionPointServer
	service *grant.Service
}

func RegisterPDPGRPC(reg grpc.ServiceRegistrar, service *grant.Service) {
	pdppb.RegisterPolicyDecisionPointServer(reg, &pdpGRPC{service: service})
}

func (s *pdpGRPC) GetHttpCatalog(ctx context.Context, r *pdppb.CatalogRequest) (*pdppb.CatalogResponse, error) {
	return pdpCatalog(ctx, s, r, s.service.ActiveHttpCapabilityCatalog)
}

func (s *pdpGRPC) GetGrpcCatalog(ctx context.Context, r *pdppb.CatalogRequest) (*pdppb.CatalogResponse, error) {
	return pdpCatalog(ctx, s, r, s.service.ActiveGRPCCapabilityCatalog)
}

func (s *pdpGRPC) DecideHttp(ctx context.Context, r *pdppb.DecisionRequest) (*pdppb.DecisionResponse, error) {
	return pdpDecision(ctx, s, r, false, func(ctx context.Context, bearer, org string, in grant.HttpCapabilityDecisionInput) (grant.HttpCapabilityDecision, error) {
		if in.PEPPolicyRevision < 0 {
			return grant.HttpCapabilityDecision{}, failure(422, "pepPolicyRevision must not be negative")
		}
		return s.service.DecideHttpCapability(ctx, bearer, org, in)
	})
}

func (s *pdpGRPC) DecideGrpc(ctx context.Context, r *pdppb.DecisionRequest) (*pdppb.DecisionResponse, error) {
	return pdpDecision(ctx, s, r, true, s.service.DecideGRPCCapability)
}

func pdpCatalog[T any](ctx context.Context, s *pdpGRPC, r *pdppb.CatalogRequest, lookup func(context.Context, string) (T, error)) (*pdppb.CatalogResponse, error) {
	ctx, cancel := context.WithTimeout(ctx, pdpRPCTimeout)
	defer cancel()
	if err := s.authenticatePEP(ctx); err != nil {
		return nil, err
	}
	if r == nil || len(r.ProtoReflect().GetUnknown()) != 0 || strings.TrimSpace(r.GetProviderId()) == "" {
		return nil, status.Error(codes.InvalidArgument, "invalid catalog request")
	}
	result, err := lookup(ctx, r.GetProviderId())
	raw, err := pdpEncode(ctx, result, err, maxJSONBodyBytes)
	if err != nil {
		return nil, err
	}
	return &pdppb.CatalogResponse{CatalogJson: raw}, nil
}

func pdpDecision[Input, Output any](ctx context.Context, s *pdpGRPC, r *pdppb.DecisionRequest, requireOrg bool, decide func(context.Context, string, string, Input) (Output, error)) (*pdppb.DecisionResponse, error) {
	ctx, cancel := context.WithTimeout(ctx, pdpRPCTimeout)
	defer cancel()
	if err := s.authenticatePEP(ctx); err != nil {
		return nil, err
	}
	bearer, org, err := pdpDecisionMetadata(ctx, requireOrg)
	if err != nil {
		return nil, err
	}
	if r == nil || len(r.ProtoReflect().GetUnknown()) != 0 {
		return nil, status.Error(codes.InvalidArgument, "invalid decision request")
	}
	in, err := decodeJSON[Input](r.GetEvidenceJson(), true)
	if err != nil {
		return nil, pdpStatus(err)
	}
	if err := ctx.Err(); err != nil {
		return nil, pdpStatus(err)
	}
	result, err := decide(ctx, bearer, org, in)
	raw, err := pdpEncode(ctx, result, err, pdpMaxDecisionJSON)
	if err != nil {
		return nil, err
	}
	return &pdppb.DecisionResponse{DecisionJson: raw}, nil
}

func (s *pdpGRPC) authenticatePEP(ctx context.Context) error {
	if err := ctx.Err(); err != nil {
		return pdpStatus(err)
	}
	keys := metadata.ValueFromIncomingContext(ctx, "x-grant-app-auth-key")
	if len(keys) > 1 {
		return status.Error(codes.InvalidArgument, "duplicate app key metadata")
	}
	expected := s.service.Config().Security.AppAuthKey
	if len(keys) != 1 || keys[0] == "" || expected == "" {
		return status.Error(codes.Unauthenticated, "app key required")
	}
	got, want := sha256.Sum256([]byte(keys[0])), sha256.Sum256([]byte(expected))
	if subtle.ConstantTimeCompare(got[:], want[:]) != 1 {
		return status.Error(codes.Unauthenticated, "invalid app key")
	}
	return nil
}

func pdpDecisionMetadata(ctx context.Context, requireOrg bool) (string, string, error) {
	auth := metadata.ValueFromIncomingContext(ctx, "authorization")
	orgs := metadata.ValueFromIncomingContext(ctx, "x-organization-id")
	if len(auth) > 1 || len(orgs) > 1 {
		return "", "", status.Error(codes.InvalidArgument, "duplicate decision metadata")
	}
	if len(auth) == 0 {
		return "", "", status.Error(codes.Unauthenticated, "Bearer credential required")
	}
	// Preserve the whole credential, including scheme case and internal spaces,
	// because the existing digest binds that exact string.
	bearer := strings.TrimSpace(auth[0])
	token, ok := strings.CutPrefix(bearer, "Bearer ")
	if !ok {
		token, ok = strings.CutPrefix(bearer, "bearer ")
	}
	if !ok || strings.TrimSpace(token) == "" {
		return "", "", status.Error(codes.InvalidArgument, "invalid Bearer credential")
	}
	var org string
	if len(orgs) == 1 {
		org = strings.TrimSpace(orgs[0])
	}
	if org == "" && (requireOrg || len(orgs) != 0) {
		return "", "", status.Error(codes.InvalidArgument, "organization required")
	}
	return bearer, org, nil
}

func pdpEncode(ctx context.Context, result any, err error, maxBytes int) ([]byte, error) {
	if ctx.Err() != nil {
		return nil, pdpStatus(ctx.Err())
	}
	if err != nil {
		return nil, pdpStatus(err)
	}
	raw, err := json.Marshal(result)
	if err != nil {
		return nil, status.Error(codes.Internal, "PDP response encoding failed")
	}
	if len(raw) > maxBytes {
		return nil, status.Error(codes.ResourceExhausted, "PDP response exceeds size limit")
	}
	if err := ctx.Err(); err != nil {
		return nil, pdpStatus(err)
	}
	return raw, nil
}

func pdpStatus(err error) error {
	switch {
	case errors.Is(err, context.Canceled):
		return status.Error(codes.Canceled, "PDP request canceled")
	case errors.Is(err, context.DeadlineExceeded):
		return status.Error(codes.DeadlineExceeded, "PDP request deadline exceeded")
	}
	code := codes.Internal
	switch grant.ErrorStatus(err) {
	case 400, 422:
		code = codes.InvalidArgument
	case 401:
		code = codes.Unauthenticated
	case 403:
		code = codes.PermissionDenied
	case 404:
		code = codes.NotFound
	case 409, 412, 428:
		code = codes.FailedPrecondition
	case 413:
		code = codes.ResourceExhausted
	case 503:
		code = codes.Unavailable
	}
	// Domain errors can include input, identity or storage details.
	return status.Error(code, code.String())
}
