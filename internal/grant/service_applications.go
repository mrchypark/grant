package grant

import (
	"context"
	"fmt"
	whatwg "github.com/nlnwa/whatwg-url/url"
	"net"
	"strings"
)

func (s *Service) ListApplicationsAuthorized(ctx context.Context, user, org string, c ReadConsistency) ([]Application, error) {
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.application.manage", "manage organization applications", c); err != nil {
		return nil, err
	}
	return s.store.ListApplicationsForOrganizationWithConsistency(ctx, org, c)
}
func (s *Service) GetApplicationAuthorized(ctx context.Context, user, org, id string, c ReadConsistency) (Application, error) {
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.application.manage", "manage organization applications", c); err != nil {
		return Application{}, err
	}
	a, err := s.store.FindApplicationForOrganizationWithConsistency(ctx, id, org, c)
	if err != nil {
		return Application{}, err
	}
	if a == nil {
		return Application{}, serviceError(404, fmt.Sprintf("application not found: %s in organization %s", id, org))
	}
	return *a, nil
}
func (s *Service) GetApplicationDetailAuthorized(ctx context.Context, user, org, id string, c ReadConsistency) (ApplicationRead, error) {
	a, err := s.GetApplicationAuthorized(ctx, user, org, id, c)
	if err != nil {
		return ApplicationRead{}, err
	}
	uri, err := s.applicationRedirect(ctx, id, c)
	return ApplicationRead{Application: a, RedirectURI: uri}, err
}
func (s *Service) applicationRedirect(ctx context.Context, id string, c ReadConsistency) (*string, error) {
	uris, err := s.store.ListApplicationRedirectURIsWithConsistency(ctx, id, c)
	if err != nil {
		return nil, err
	}
	if len(uris) > 1 {
		return nil, serviceError(409, "local application redirect URI manifest has more than one entry")
	}
	if len(uris) == 1 {
		return &uris[0].RedirectURI, nil
	}
	return nil, nil
}
func CanonicalRedirectURI(value string) (string, error) {
	if strings.TrimSpace(value) == "" || len(value) > 2048 || strings.Contains(value, "*") {
		return "", serviceError(400, "redirectUri must be a non-empty URL of at most 2048 bytes")
	}
	u, err := whatwg.Parse(value)
	if err != nil {
		return "", serviceError(400, "redirectUri must be an absolute URL")
	}
	authority := ""
	if _, rest, ok := strings.Cut(value, "://"); ok {
		authority = rest
		if end := strings.IndexAny(rest, "/?#"); end >= 0 {
			authority = rest[:end]
		}
	}
	canonical := u.Href(false)
	if strings.Contains(authority, "@") || strings.ContainsAny(canonical, "?#") || u.Hostname() == "" {
		return "", serviceError(400, "redirectUri has unsupported authority, fragment, query, or host")
	}
	switch u.Scheme() {
	case "https":
	case "http":
		ip := net.ParseIP(strings.Trim(u.Hostname(), "[]"))
		if ip == nil || !ip.IsLoopback() || u.Port() == "" {
			return "", serviceError(400, "redirectUri HTTP must use a loopback IP with an explicit port")
		}
	default:
		return "", serviceError(400, "redirectUri scheme must be https or loopback http")
	}
	if len(canonical) > 2048 || strings.Contains(canonical, "*") {
		return "", serviceError(400, "redirectUri must be a non-empty URL of at most 2048 bytes")
	}
	return canonical, nil
}
func (s *Service) CreateApplication(ctx context.Context, user, org string, in CreateApplicationInput) (ApplicationRead, error) {
	if !enumValue(in.ApplicationType, "confidential", "public") || !enumValue(in.AppType, "web", "mobile", "server", "internal") {
		return ApplicationRead{}, serviceError(400, "invalid application type")
	}
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.application.manage", "manage organization applications", ReadStrong); err != nil {
		return ApplicationRead{}, err
	}
	vals := []string{in.ID, in.Slug, in.Name}
	labels := []string{"id", "slug", "name"}
	for i := range vals {
		x, err := required(labels[i], vals[i])
		if err != nil {
			return ApplicationRead{}, err
		}
		vals[i] = x
	}
	a := Application{ID: vals[0], OrganizationID: org, Slug: vals[1], Name: vals[2], Description: optionalText(in.Description), ApplicationType: in.ApplicationType, AppType: in.AppType, FirstParty: false, ConsentPolicy: "explicit", Active: in.Active, ProvisioningStatus: "unprovisioned", CreatedByUserID: user}
	if s.cfg.Auth.ProviderMode == "pocketbase" {
		a.ProvisioningStatus = "ready"
		a.RauthyClientID = nil
	}
	var uri *string
	if in.RedirectURI != nil {
		v, err := CanonicalRedirectURI(*in.RedirectURI)
		if err != nil {
			return ApplicationRead{}, err
		}
		uri = &v
	}
	err := s.store.InsertApplicationWithRedirectURI(ctx, a, uri)
	return ApplicationRead{Application: a, RedirectURI: uri}, err
}
func (s *Service) ListApplicationActionsAuthorized(ctx context.Context, user, org, id string, c ReadConsistency) ([]Action, error) {
	if _, err := s.GetApplicationAuthorized(ctx, user, org, id, c); err != nil {
		return nil, err
	}
	return s.store.ListApplicationActionsWithConsistency(ctx, id, c)
}
func (s *Service) ReplaceApplicationActionsAuthorized(ctx context.Context, user, org, id string, actions []string) ([]Action, error) {
	app, err := s.GetApplicationAuthorized(ctx, user, org, id, ReadStrong)
	if err != nil {
		return nil, err
	}
	if app.FirstParty && app.ConsentPolicy == "auto_approve" {
		return nil, serviceError(403, "forbidden: trusted application actions cannot be changed")
	}
	if err := s.applicationMutationAvailable(ctx, id); err != nil {
		return nil, err
	}
	target, err := serviceRow[applicationActionTarget](ctx, s, ReadStrong, "SELECT id,organization_id,created_at FROM applications WHERE id=? AND organization_id=?", id, org)
	if err != nil {
		return nil, err
	}
	if len(actions) > 999 {
		return nil, serviceError(400, "actionIds must contain at most 999 items")
	}
	ids := make([]string, 0, len(actions))
	for _, a := range actions {
		x, err := required("actionIds[]", a)
		if err != nil {
			return nil, err
		}
		ids = append(ids, x)
	}
	j, _ := jsonString(ids)
	invalid, err := s.store.Query(ctx, ReadStrong, "SELECT j.value AS id FROM json_each(?) j WHERE NOT EXISTS(SELECT 1 FROM actions a JOIN service_providers p ON p.id=a.provider_id AND p.active=1 WHERE a.id=j.value AND a.grantable_to_applications=1) LIMIT 1", j)
	if err != nil {
		return nil, err
	}
	if len(invalid) != 0 {
		return nil, serviceError(400, "action "+rowString(invalid[0], "id")+" is not grantable to applications")
	}
	out, err := s.store.replaceApplicationActions(ctx, target, user, ids)
	return out, applicationMutationError(err)
}
func (s *Service) SetApplicationTrustAdmin(ctx context.Context, org, id string, trusted bool) (Application, error) {
	if _, err := s.applicationInOrg(ctx, id, org); err != nil {
		return Application{}, err
	}
	if err := s.applicationMutationAvailable(ctx, id); err != nil {
		return Application{}, err
	}
	out, err := s.store.SetApplicationTrust(ctx, id, org, trusted)
	return out, applicationMutationError(err)
}

func (s *Service) applicationMutationAvailable(ctx context.Context, id string) error {
	rows, err := s.store.Query(ctx, ReadStrong, "SELECT id FROM applications WHERE id=? AND security_mutation_token IS NOT NULL AND security_mutation_expires_at>?", id, nowString())
	if err != nil {
		return err
	}
	if len(rows) > 0 {
		return serviceError(409, "application security mutation is already in progress")
	}
	return nil
}

func applicationMutationError(err error) error {
	if err != nil && strings.Contains(err.Error(), "transaction rejected") {
		return serviceError(409, "application security mutation preconditions changed")
	}
	return err
}

type ApplicationConsent struct {
	Grant   ApplicationGrant `json:"grant"`
	Actions []Action         `json:"actions"`
}

func (s *Service) applicationInOrg(ctx context.Context, id, org string) (Application, error) {
	a, err := s.store.GetApplicationWithConsistency(ctx, id, ReadStrong)
	if err != nil {
		return a, err
	}
	if a.OrganizationID != org {
		return Application{}, serviceError(404, fmt.Sprintf("application not found: %s in organization %s", id, org))
	}
	return a, nil
}
func (s *Service) grantActions(ctx context.Context, id string) ([]Action, error) {
	return serviceRows[Action](ctx, s, ReadStrong, "SELECT a.* FROM actions a JOIN application_grant_actions ga ON ga.action_id=a.id WHERE ga.application_grant_id=? ORDER BY a.code", id)
}
func (s *Service) GetApplicationConsent(ctx context.Context, user, org, id string) (*ApplicationConsent, error) {
	if _, err := s.applicationInOrg(ctx, id, org); err != nil {
		return nil, err
	}
	g, err := s.store.FindActiveApplicationConsentWithConsistency(ctx, user, id, org, ReadStrong)
	if err != nil || g == nil {
		return nil, err
	}
	actions, err := s.grantActions(ctx, g.ID)
	return &ApplicationConsent{Grant: *g, Actions: actions}, err
}
func (s *Service) ReplaceApplicationConsent(ctx context.Context, user, org, id string, ids []string) (ApplicationConsent, error) {
	if len(ids) == 0 || len(ids) > 999 {
		return ApplicationConsent{}, serviceError(400, "actionIds must contain between 1 and 999 items")
	}
	normalized := make([]string, 0, len(ids))
	seen := map[string]bool{}
	for _, v := range ids {
		x, err := required("actionIds[]", v)
		if err != nil {
			return ApplicationConsent{}, err
		}
		if !seen[x] {
			seen[x] = true
			normalized = append(normalized, x)
		}
	}
	a, err := s.applicationInOrg(ctx, id, org)
	if err != nil {
		return ApplicationConsent{}, err
	}
	if !a.Active {
		return ApplicationConsent{}, serviceError(403, "forbidden: application is inactive")
	}
	if a.ConsentPolicy != "explicit" {
		return ApplicationConsent{}, serviceError(400, "explicit consent cannot be written for an auto-approve application")
	}
	ok, err := s.store.HasActiveOrganizationMembershipWithConsistency(ctx, user, org, ReadStrong)
	if err != nil {
		return ApplicationConsent{}, err
	}
	if !ok {
		return ApplicationConsent{}, serviceError(403, "forbidden: user is not an active member of organization")
	}
	g := ApplicationGrant{ID: requestID(), UserID: user, ApplicationID: id, OrganizationID: org, Status: "active", ConsentedAt: nowString(), CreatedAt: nowString(), UpdatedAt: nowString()}
	g, err = s.store.ReplaceActiveApplicationConsent(ctx, g, normalized)
	if err != nil {
		return ApplicationConsent{}, err
	}
	actions, err := s.grantActions(ctx, g.ID)
	if err != nil {
		return ApplicationConsent{}, err
	}
	if len(actions) != len(normalized) {
		return ApplicationConsent{}, serviceError(409, "application actions changed while consent was being written")
	}
	return ApplicationConsent{Grant: g, Actions: actions}, nil
}
func (s *Service) RevokeApplicationConsent(ctx context.Context, user, org, id string) error {
	if _, err := s.applicationInOrg(ctx, id, org); err != nil {
		return err
	}
	return s.store.RevokeActiveApplicationConsent(ctx, user, id, org)
}
func (s *Service) ListApplicationGrantsAuthorized(ctx context.Context, user, org string, c ReadConsistency) ([]ApplicationGrant, error) {
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.application.manage", "manage organization application grants", c); err != nil {
		return nil, err
	}
	return serviceRows[ApplicationGrant](ctx, s, c, "SELECT * FROM application_grants WHERE organization_id=? ORDER BY created_at DESC,id", org)
}
