package grant

import "context"

func (s *Service) BootstrapRauthyOwner(ctx context.Context, token string) (AuthenticatedPrincipal, error) {
	if s.AuthProviderMode() != "rauthy" {
		return AuthenticatedPrincipal{}, serviceError(403, "forbidden: Rauthy owner bootstrap is unavailable for the configured auth provider")
	}
	i, err := s.externalIdentity(ctx, token, "")
	if err != nil {
		return AuthenticatedPrincipal{}, err
	}
	u, err := s.store.BindIdentityToUser(ctx, BindIdentityInput{UserID: s.cfg.Security.BootstrapUserID, Provider: i.Provider, ExternalUserID: i.ExternalUserID, Email: i.Email, DisplayName: i.DisplayName, Active: i.Active, AllowRebind: true})
	if err != nil {
		return AuthenticatedPrincipal{}, err
	}
	if !u.Active {
		return AuthenticatedPrincipal{}, serviceError(403, "forbidden: local user is inactive")
	}
	return AuthenticatedPrincipal{UserID: u.ID, OrganizationID: s.cfg.Security.BootstrapUserID}, nil
}
func (s *Service) BootstrapExternalOwner(ctx context.Context, token, org string) (AuthenticatedPrincipal, error) {
	if s.AuthProviderMode() != "patch-data" {
		return AuthenticatedPrincipal{}, serviceError(403, "forbidden: external owner bootstrap is unavailable for Rauthy")
	}
	if org == "" {
		return AuthenticatedPrincipal{}, serviceError(403, "forbidden: Patch Data organization selection is required")
	}
	i, err := s.externalIdentity(ctx, token, org)
	if err != nil {
		return AuthenticatedPrincipal{}, err
	}
	if !i.Active {
		return AuthenticatedPrincipal{}, serviceError(403, "forbidden: external user is inactive")
	}
	v, err := s.store.BootstrapExternalOwnerAtomic(ctx, ExternalOwnerBootstrapCommand{Identity: ExternalIdentityMaterialization{Provider: i.Provider, ExternalUserID: i.ExternalUserID, Email: i.Email, DisplayName: i.DisplayName}, OrganizationID: org, ApplicationID: s.cfg.Auth.PatchData.ExposureApplicationID})
	return AuthenticatedPrincipal{UserID: v.UserID, OrganizationID: v.OrganizationID}, err
}
func (s *Service) TriggerBackup(ctx context.Context) error { return s.store.TriggerBackup(ctx) }
func (s *Service) ListBackupsLocal(ctx context.Context) ([]BackupInfo, error) {
	return s.store.ListBackupsLocal(ctx)
}
func (s *Service) ListBackupsS3(ctx context.Context) ([]BackupInfo, error) {
	return s.store.ListBackupsS3(ctx)
}
