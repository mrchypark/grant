package grant

import "context"

// IdentityBindingInput names an existing client managed by the configured IdP.
// It never asks Grant to create a client or issue credentials.
type IdentityBindingInput struct {
	ClientID string `json:"clientId"`
}

func (s *Service) BindApplicationIdentityAdmin(ctx context.Context, org, id string, in IdentityBindingInput) (Application, error) {
	clientID, err := required("clientId", in.ClientID)
	if err != nil {
		return Application{}, err
	}
	a, err := s.store.FindApplicationForOrganizationWithConsistency(ctx, id, org, ReadStrong)
	if err != nil {
		return Application{}, err
	}
	if a == nil {
		return Application{}, serviceError(404, "application not found")
	}
	if !a.Active {
		return Application{}, serviceError(409, "application is inactive")
	}
	if a.ProvisioningStatus == "ready" && ptrValue(a.RauthyClientID) != clientID {
		return Application{}, serviceError(409, "ready application identity cannot be rebound")
	}
	if s.cfg.Auth.ProviderMode != "rauthy" {
		return Application{}, serviceError(409, "configured identity provider does not support client binding verification")
	}
	if s.cfg.Auth.Rauthy.ClientReadAPIKey == "" {
		return Application{}, serviceError(503, "identity client verification is not configured")
	}
	if err := s.applicationMutationAvailable(ctx, id); err != nil {
		return Application{}, err
	}
	uri, err := s.applicationRedirect(ctx, id, ReadStrong)
	if err != nil {
		return Application{}, err
	}
	if err := s.verifyRauthyIdentityBinding(ctx, *a, clientID, uri); err != nil {
		return Application{}, err
	}
	out, err := s.store.BindApplicationIdentity(ctx, *a, clientID, uri)
	return out, applicationMutationError(err)
}

// VerifyApplicationProvisioningAdmin supports existing internal callers with a
// persisted reservation. New clients use the explicit, verified identity binding.
func (s *Service) VerifyApplicationProvisioningAdmin(ctx context.Context, org, id string) (Application, error) {
	a, err := s.store.FindApplicationForOrganizationWithConsistency(ctx, id, org, ReadStrong)
	if err != nil {
		return Application{}, err
	}
	if a == nil {
		return Application{}, serviceError(404, "application not found")
	}
	return s.BindApplicationIdentityAdmin(ctx, org, id, IdentityBindingInput{ClientID: ptrValue(a.RauthyClientID)})
}
