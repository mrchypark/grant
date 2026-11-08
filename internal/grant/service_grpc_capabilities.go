package grant

import (
	"context"
	"strings"
	"time"
)

type ActiveGRPCCatalog struct {
	GRPCCatalog
	PEPClientID       string `json:"pepClientId"`
	AuthorizationMode string `json:"authorizationMode"`
	PEPPolicyRevision int64  `json:"pepPolicyRevision"`
	IssuedAt          string `json:"issuedAt"`
	ExpiresAt         string `json:"expiresAt"`
}

type GRPCCredentialEvidence struct {
	Subject  string `json:"subject"`
	Audience string `json:"audience"`
}

type GRPCCapabilityDecisionInput struct {
	WireVersion              uint64                 `json:"wireVersion"`
	ProviderID               string                 `json:"providerId"`
	PEPClientID              string                 `json:"pepClientId"`
	PEPPolicyRevision        int64                  `json:"pepPolicyRevision"`
	CatalogGeneration        uint64                 `json:"catalogGeneration"`
	CatalogDigest            string                 `json:"catalogDigest"`
	CapabilityID             string                 `json:"capabilityId"`
	CapabilityVersion        uint64                 `json:"capabilityVersion"`
	ContractDigest           string                 `json:"contractDigest"`
	FullMethod               string                 `json:"fullMethod"`
	DescriptorDigest         string                 `json:"descriptorDigest"`
	RPCKind                  string                 `json:"rpcKind"`
	ResourceType             string                 `json:"resourceType"`
	ResourceID               string                 `json:"resourceId"`
	MessageDigest            string                 `json:"messageDigest"`
	MessageLength            uint64                 `json:"messageLength"`
	ForwardedMetadataDigest  string                 `json:"forwardedMetadataDigest"`
	ProviderCredentialDigest string                 `json:"providerCredentialDigest"`
	ProviderCredential       GRPCCredentialEvidence `json:"providerCredential"`
	RequestBinding           string                 `json:"requestBinding"`
}

type GRPCCapabilityDecision struct {
	GRPCCapabilityDecisionInput
	Allowed         bool   `json:"allowed"`
	Reason          string `json:"reason"`
	ActionID        string `json:"actionId"`
	OrganizationID  string `json:"organizationId"`
	PrincipalDigest string `json:"principalDigest"`
	IssuedAt        string `json:"issuedAt"`
	ExpiresAt       string `json:"expiresAt"`
}

func (s *Service) StageGRPCCapabilityGenerationAdmin(ctx context.Context, provider string, in StageGRPCCapabilityGeneration) (GRPCCatalog, error) {
	return s.store.StageGRPCCapabilityGeneration(ctx, provider, in)
}

func (s *Service) ActivateGRPCCapabilityGenerationAdmin(ctx context.Context, provider string, in ActivateGRPCCapabilityGeneration) (GRPCCatalog, error) {
	return s.store.ActivateGRPCCapabilityGeneration(ctx, provider, in)
}

func (s *Service) ActiveGRPCCapabilityCatalog(ctx context.Context, provider string) (ActiveGRPCCatalog, error) {
	return capabilityAuthorizationRead(ctx, s, func(ctx context.Context) (ActiveGRPCCatalog, error) {
		pep, catalog, err := s.grpcCapabilityPolicy(ctx, provider)
		if err != nil {
			return ActiveGRPCCatalog{}, err
		}
		now := time.Now().UTC()
		expires, err := grpcPEPLeaseExpiry(pep, now, 30*time.Second)
		if err != nil {
			return ActiveGRPCCatalog{}, err
		}
		return ActiveGRPCCatalog{
			GRPCCatalog: catalog, PEPClientID: pep.PepClientID, AuthorizationMode: pep.AuthorizationMode,
			PEPPolicyRevision: pep.PolicyRevision, IssuedAt: now.Format(time.RFC3339Nano),
			ExpiresAt: expires.Format(time.RFC3339Nano),
		}, nil
	})
}

func grpcPEPLeaseExpiry(pep ProviderPepClient, now time.Time, ttl time.Duration) (time.Time, error) {
	if err := requireActivePEP(pep, now); err != nil {
		return time.Time{}, err
	}
	expires := now.Add(ttl)
	if pep.ValidUntil != nil {
		// requireActivePEP already validated this timestamp.
		validUntil, _ := time.Parse(time.RFC3339Nano, *pep.ValidUntil)
		if validUntil.Before(expires) {
			expires = validUntil
		}
	}
	return expires.UTC(), nil
}

func (s *Service) grpcCapabilityPolicy(ctx context.Context, provider string) (ProviderPepClient, GRPCCatalog, error) {
	pep, err := s.configuredPEP(ctx, provider)
	if err != nil {
		return ProviderPepClient{}, GRPCCatalog{}, err
	}
	if pep.ProviderID != provider || pep.AuthorizationMode != "user" || pep.PolicyRevision <= 0 {
		return ProviderPepClient{}, GRPCCatalog{}, serviceError(403, "forbidden: gRPC requires an active user PEP policy")
	}
	catalog, err := s.store.ActiveGRPCCapabilityCatalog(ctx, provider)
	if err != nil {
		return ProviderPepClient{}, GRPCCatalog{}, err
	}
	if catalog.SchemaVersion != 1 || catalog.ProviderID != provider || catalog.Generation == 0 {
		return ProviderPepClient{}, GRPCCatalog{}, serviceError(503, "unavailable: invalid gRPC capability catalog")
	}
	return pep, catalog, nil
}

func validateGRPCRequestEvidence(in GRPCCapabilityDecisionInput) error {
	if in.WireVersion != 1 || in.RPCKind != "unary" {
		return serviceError(400, "gRPC capability wire version 1 requires a unary RPC")
	}
	if in.PEPPolicyRevision <= 0 || in.CatalogGeneration == 0 || in.CapabilityVersion == 0 {
		return serviceError(400, "gRPC capability policy, catalog and capability versions must be positive")
	}
	for _, field := range []struct{ name, value string }{
		{"catalogDigest", in.CatalogDigest}, {"contractDigest", in.ContractDigest},
		{"descriptorDigest", in.DescriptorDigest}, {"messageDigest", in.MessageDigest},
		{"forwardedMetadataDigest", in.ForwardedMetadataDigest}, {"providerCredentialDigest", in.ProviderCredentialDigest},
	} {
		if err := validateSHA256(field.name, field.value); err != nil {
			return err
		}
	}
	if in.ForwardedMetadataDigest != sha256Text("") {
		return serviceError(403, "forbidden: gRPC application metadata is not supported")
	}
	if (in.MessageLength == 0) != (in.MessageDigest == sha256Text("")) {
		return serviceError(400, "gRPC message digest and length disagree")
	}
	if len(in.RequestBinding) < 22 || len(in.RequestBinding) > 256 ||
		strings.Trim(in.RequestBinding, "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_") != "" {
		return serviceError(400, "invalid gRPC request binding")
	}
	return nil
}

func (s *Service) DecideGRPCCapability(ctx context.Context, token, org string, in GRPCCapabilityDecisionInput) (GRPCCapabilityDecision, error) {
	if strings.TrimSpace(token) == "" {
		return GRPCCapabilityDecision{}, serviceError(401, "unauthorized: missing bearer token")
	}
	if err := exactValue("organizationId", org, 256); err != nil {
		return GRPCCapabilityDecision{}, err
	}
	if err := validateGRPCRequestEvidence(in); err != nil {
		return GRPCCapabilityDecision{}, err
	}
	if CredentialDigest(token) != in.ProviderCredentialDigest {
		return GRPCCapabilityDecision{}, serviceError(403, "forbidden: provider credential digest mismatch")
	}
	identity, err := s.externalIdentity(ctx, token)
	if err != nil {
		return GRPCCapabilityDecision{}, err
	}
	return capabilityAuthorizationRead(ctx, s, func(ctx context.Context) (GRPCCapabilityDecision, error) {
		return s.decideGRPCCapability(ctx, identity, org, in)
	})
}

func (s *Service) decideGRPCCapability(ctx context.Context, identity ExternalIdentity, org string, in GRPCCapabilityDecisionInput) (GRPCCapabilityDecision, error) {
	pep, catalog, err := s.grpcCapabilityPolicy(ctx, in.ProviderID)
	if err != nil {
		return GRPCCapabilityDecision{}, err
	}
	if pep.PepClientID != in.PEPClientID || pep.PolicyRevision != in.PEPPolicyRevision {
		return GRPCCapabilityDecision{}, serviceError(403, "forbidden: gRPC PEP policy binding mismatch")
	}
	if catalog.Generation != in.CatalogGeneration || catalog.CatalogDigest != in.CatalogDigest {
		return GRPCCapabilityDecision{}, serviceError(403, "forbidden: stale gRPC capability catalog")
	}
	for _, cap := range catalog.Capabilities {
		if cap.Enabled && cap.CapabilityID == in.CapabilityID && cap.Version == in.CapabilityVersion {
			return s.evaluateGRPCRequest(ctx, identity, org, in, cap, pep)
		}
	}
	return GRPCCapabilityDecision{}, serviceError(403, "forbidden: gRPC capability is not active")
}

func (s *Service) evaluateGRPCRequest(ctx context.Context, identity ExternalIdentity, org string, in GRPCCapabilityDecisionInput, cap GRPCCapability, pep ProviderPepClient) (GRPCCapabilityDecision, error) {
	// Grant binds trusted PEP evidence to the catalog. Exposure checks the pinned
	// descriptor's unary flags and hashes the exact protobuf bytes before framing.
	if in.ContractDigest != cap.ContractDigest || in.FullMethod != cap.FullMethod ||
		in.DescriptorDigest != cap.DescriptorDigest || in.ResourceType != cap.ResourceType ||
		in.ResourceID != cap.ResourceID || cap.MaxMessageBytes == 0 || in.MessageLength > cap.MaxMessageBytes {
		return GRPCCapabilityDecision{}, serviceError(403, "forbidden: gRPC capability evidence mismatch")
	}
	p, err := s.resolveLocalUserAuthorizationContext(ctx, identity, org)
	if err != nil {
		return GRPCCapabilityDecision{}, err
	}
	if p.ExternalSubject != in.ProviderCredential.Subject || p.ProviderAudience != in.ProviderCredential.Audience {
		return GRPCCapabilityDecision{}, serviceError(403, "forbidden: provider credential principal mismatch")
	}
	// The active catalog read verified that the action still belongs to this provider.
	decision, err := s.CheckAuthorization(ctx,
		AuthorizationSubject{SubjectType: "user", ID: p.UserID, OrganizationID: p.OrganizationID},
		cap.ActionID, AuthorizationResource{ProviderID: in.ProviderID, ResourceType: cap.ResourceType, ID: cap.ResourceID}, ReadStrong)
	if err != nil {
		return GRPCCapabilityDecision{}, err
	}
	now := time.Now().UTC()
	expires, err := grpcPEPLeaseExpiry(pep, now, 5*time.Second)
	if err != nil {
		return GRPCCapabilityDecision{}, err
	}
	return GRPCCapabilityDecision{
		GRPCCapabilityDecisionInput: in, Allowed: decision.Allowed, Reason: decision.Reason,
		ActionID: cap.ActionID, OrganizationID: p.OrganizationID,
		PrincipalDigest: ExposurePrincipalDigest(p.UserID, p.OrganizationID, ""),
		IssuedAt:        now.Format(time.RFC3339Nano), ExpiresAt: expires.Format(time.RFC3339Nano),
	}, nil
}
