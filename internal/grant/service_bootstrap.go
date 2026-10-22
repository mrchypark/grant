package grant

import "context"

// BootstrapOwner uses the configured identity adapter and creates only an
// identity, organization and owner membership. Applications are registered
// separately by their organization.
func (s *Service) BootstrapOwner(ctx context.Context, token, org string) (AuthenticatedPrincipal, error) {
	if org == "" {
		return AuthenticatedPrincipal{}, serviceError(400, "organization id is required")
	}
	i, err := s.externalIdentity(ctx, token)
	if err != nil {
		return AuthenticatedPrincipal{}, err
	}
	if !i.Active {
		return AuthenticatedPrincipal{}, serviceError(403, "forbidden: external user is inactive")
	}
	cmd := OwnerBootstrapCommand{
		Identity:       ExternalIdentityMaterialization{Provider: i.Provider, Issuer: i.Issuer, CollectionID: i.CollectionID, ExternalUserID: i.ExternalUserID, Email: i.Email, DisplayName: i.DisplayName},
		OrganizationID: org,
	}
	reserved := s.cfg.Security.BootstrapUserID
	if reserved == "" {
		reserved = "conalog-grant"
	}
	if org == reserved {
		cmd.ReservedUserID = reserved
	}
	v, err := s.store.BootstrapOwnerAtomic(ctx, cmd)
	return AuthenticatedPrincipal{UserID: v.UserID, OrganizationID: v.OrganizationID}, err
}

func (s *Service) TriggerBackup(ctx context.Context) error { return s.store.TriggerBackup(ctx) }
func (s *Service) ListBackups(ctx context.Context) ([]BackupInfo, error) {
	return s.store.ListBackups(ctx)
}
