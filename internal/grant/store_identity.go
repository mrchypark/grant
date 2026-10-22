package grant

import (
	"context"
	"fmt"
	"strings"
)

type ExternalIdentityMaterialization struct {
	Provider, ExternalUserID string
	Issuer, CollectionID     string
	Email                    *string
	DisplayName              string
}
type OwnerBootstrapCommand struct {
	Identity       ExternalIdentityMaterialization
	OrganizationID string
	// ReservedUserID is server-selected for the preseeded bootstrap organization.
	ReservedUserID string
}
type OwnerBootstrapResult struct {
	UserID         string `json:"userId"`
	OrganizationID string `json:"organizationId"`
}

func (s *Store) GetUserWithConsistency(ctx context.Context, id string, c ReadConsistency) (User, error) {
	rows, e := s.Query(ctx, c, "SELECT * FROM users WHERE id=?", id)
	if e != nil {
		return User{}, e
	}
	if len(rows) == 0 {
		return User{}, fmt.Errorf("user not found: %s", id)
	}
	return decodeRow[User](rows[0])
}
func (s *Store) SetUserActive(ctx context.Context, id string, active bool) (User, error) {
	n, e := s.Exec(ctx, "UPDATE users SET active=? WHERE id=?", active, id)
	if e != nil {
		return User{}, e
	}
	if n == 0 {
		return User{}, fmt.Errorf("user not found: %s", id)
	}
	return s.GetUserWithConsistency(ctx, id, ReadStrong)
}
func (s *Store) ResolveExistingUserIdentityWithConsistency(ctx context.Context, p, id string, c ReadConsistency) (*User, error) {
	return s.resolveScopedIdentity(ctx, ExternalIdentityMaterialization{Provider: p, ExternalUserID: id}, c)
}
func (s *Store) resolveScopedIdentity(ctx context.Context, in ExternalIdentityMaterialization, c ReadConsistency) (*User, error) {
	rows, e := s.Query(ctx, c, "SELECT u.* FROM users u JOIN user_identities i ON i.user_id=u.id WHERE i.provider=? AND i.issuer=? AND i.collection_id=? AND i.external_user_id=?", in.Provider, in.Issuer, in.CollectionID, in.ExternalUserID)
	if e != nil {
		return nil, e
	}
	if len(rows) == 0 {
		return nil, nil
	}
	u, e := decodeRow[User](rows[0])
	return &u, e
}
func (s *Store) HasActiveOrganizationMembershipWithConsistency(ctx context.Context, user, org string, c ReadConsistency) (bool, error) {
	rows, e := s.Query(ctx, c, "SELECT 1 FROM organization_memberships m JOIN users u ON u.id=m.user_id JOIN organizations o ON o.id=m.organization_id WHERE m.user_id=? AND m.organization_id=? AND m.status='active' AND u.active=1 AND o.inactivated_at IS NULL", user, org)
	return len(rows) > 0, e
}
func (s *Store) ListUserOrganizationsWithConsistency(ctx context.Context, user string, c ReadConsistency) ([]Organization, error) {
	rows, e := s.Query(ctx, c, "SELECT o.*,p.party_id FROM organizations o JOIN organization_memberships m ON m.organization_id=o.id LEFT JOIN organization_party_links p ON p.organization_id=o.id AND p.relation_type='legal' WHERE m.user_id=? AND m.status='active' AND o.inactivated_at IS NULL ORDER BY o.name,o.id", user)
	if e != nil {
		return nil, e
	}
	return decodeRows[Organization](rows)
}
func (s *Store) EnsureExternalIdentity(ctx context.Context, in ExternalIdentityMaterialization) (User, error) {
	if in.Provider == "" || in.ExternalUserID == "" {
		return User{}, fmt.Errorf("external identity requires provider and subject")
	}
	if in.Provider == "pocketbase" {
		origin, e := canonicalPocketBaseOrigin(in.Issuer)
		if e != nil || origin != in.Issuer || !pocketBaseID(in.CollectionID) || !pocketBaseID(in.ExternalUserID) {
			return User{}, fmt.Errorf("invalid scoped PocketBase identity")
		}
	}
	if in.Provider == "goauthy" {
		origin, e := canonicalGoauthyOrigin(in.Issuer)
		if e != nil || origin != in.Issuer || in.CollectionID != "" {
			return User{}, fmt.Errorf("invalid scoped Goauthy identity")
		}
	}
	if u, e := s.resolveScopedIdentity(ctx, in, ReadStrong); e != nil {
		return User{}, e
	} else if u != nil {
		return *u, nil
	}
	id := requestID()
	now := nowString()
	err := s.Transaction(ctx, []Statement{
		{SQL: "INSERT INTO users(id,email,display_name,active,created_at) SELECT ?,?,?,1,? WHERE NOT EXISTS(SELECT 1 FROM user_identities WHERE provider=? AND issuer=? AND collection_id=? AND external_user_id=?)", Args: []any{id, in.Email, in.DisplayName, now, in.Provider, in.Issuer, in.CollectionID, in.ExternalUserID}},
		{SQL: "INSERT INTO user_identities(id,user_id,provider,issuer,collection_id,external_user_id,email_snapshot,display_name_snapshot,last_seen_at,created_at,updated_at) SELECT ?,?,?,?,?,?,?,?,?,?,? WHERE EXISTS(SELECT 1 FROM users WHERE id=?) AND NOT EXISTS(SELECT 1 FROM user_identities WHERE provider=? AND issuer=? AND collection_id=? AND external_user_id=?)", Args: []any{requestID(), id, in.Provider, in.Issuer, in.CollectionID, in.ExternalUserID, in.Email, in.DisplayName, now, now, now, id, in.Provider, in.Issuer, in.CollectionID, in.ExternalUserID}},
	})
	if err != nil {
		return User{}, err
	}
	u, e := s.resolveScopedIdentity(ctx, in, ReadStrong)
	if e != nil {
		return User{}, e
	}
	if u == nil {
		return User{}, fmt.Errorf("ensured identity missing")
	}
	return *u, nil
}

type GoauthyIdentityLinkInput struct {
	ExistingUserID, Issuer, Subject, ExpectedEmail string
}

// LinkGoauthyIdentityToExistingUser records an operator-attested subject mapping.
// It deliberately does not infer identity ownership from email or alter the user.
func (s *Store) LinkGoauthyIdentityToExistingUser(ctx context.Context, in GoauthyIdentityLinkInput) error {
	if in.ExistingUserID == "" || in.Subject == "" || in.Subject != strings.TrimSpace(in.Subject) || in.ExpectedEmail == "" {
		return serviceError(422, "invalid Goauthy identity link")
	}
	now := nowString()
	statements := []Statement{
		assertion("SELECT 1 FROM users WHERE id=? AND active=1 AND email=?", in.ExistingUserID, in.ExpectedEmail),
		assertion("SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM user_identities WHERE provider='goauthy' AND issuer=? AND collection_id='' AND external_user_id=? AND user_id<>?)", in.Issuer, in.Subject, in.ExistingUserID),
		{SQL: "INSERT INTO user_identities(id,user_id,provider,issuer,collection_id,external_user_id,email_snapshot,display_name_snapshot,last_seen_at,created_at,updated_at) SELECT ?,id,'goauthy',?,'',?,email,display_name,NULL,?,? FROM users WHERE id=? AND active=1 AND email=? ON CONFLICT(provider,issuer,collection_id,external_user_id) DO NOTHING", Args: []any{requestID(), in.Issuer, in.Subject, now, now, in.ExistingUserID, in.ExpectedEmail}},
		assertion("SELECT 1 FROM user_identities WHERE provider='goauthy' AND issuer=? AND collection_id='' AND external_user_id=? AND user_id=?", in.Issuer, in.Subject, in.ExistingUserID),
	}
	if err := s.Transaction(ctx, statements); err != nil {
		if strings.Contains(err.Error(), "transaction rejected") {
			return serviceError(409, "Goauthy identity link conflict")
		}
		return err
	}
	return nil
}
func (s *Store) EnrollActiveMember(ctx context.Context, org, user string) (OrganizationMembership, error) {
	now := nowString()
	q := Statement{SQL: "INSERT INTO organization_memberships(id,organization_id,user_id,membership_role,status,created_at,updated_at) SELECT ?,?,?,'member','active',?,? FROM users u JOIN organizations o ON o.id=? WHERE u.id=? AND u.active=1 AND o.inactivated_at IS NULL ON CONFLICT(organization_id,user_id) DO UPDATE SET status='active',updated_at=excluded.updated_at WHERE organization_memberships.membership_role='member'", Args: []any{requestID(), org, user, now, now, org, user}, ExpectedRowsAffected: requireOne()}
	if e := s.Transaction(ctx, []Statement{q}); e != nil {
		return OrganizationMembership{}, e
	}
	r, e := s.Query(ctx, ReadStrong, "SELECT * FROM organization_memberships WHERE organization_id=? AND user_id=?", org, user)
	if e != nil {
		return OrganizationMembership{}, e
	}
	return decodeRow[OrganizationMembership](r[0])
}

type BindIdentityInput struct {
	UserID, Provider, ExternalUserID string
	Email                            *string
	DisplayName                      string
	Active, AllowRebind              bool
}

func (s *Store) BindIdentityToUser(ctx context.Context, in BindIdentityInput) (User, error) {
	if in.Provider != "rauthy" {
		return User{}, fmt.Errorf("unqualified binding is only supported for Rauthy")
	}
	now := nowString()
	ss := []Statement{assertion("SELECT 1 FROM users WHERE id=?", in.UserID)}
	if !in.AllowRebind {
		ss = append(ss, assertion("SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM user_identities WHERE provider=? AND external_user_id=? AND user_id<>?)", in.Provider, in.ExternalUserID, in.UserID))
	}
	ss = append(ss, Statement{SQL: "UPDATE users SET email=COALESCE(?,email),display_name=? WHERE id=?", Args: []any{in.Email, in.DisplayName, in.UserID}, ExpectedRowsAffected: requireOne()}, Statement{SQL: "INSERT INTO user_identities(id,user_id,provider,external_user_id,email_snapshot,display_name_snapshot,last_seen_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?) ON CONFLICT(provider,issuer,collection_id,external_user_id) DO UPDATE SET user_id=excluded.user_id,email_snapshot=excluded.email_snapshot,display_name_snapshot=excluded.display_name_snapshot,last_seen_at=excluded.last_seen_at,updated_at=excluded.updated_at", Args: []any{requestID(), in.UserID, in.Provider, in.ExternalUserID, in.Email, in.DisplayName, now, now, now}})
	if e := s.Transaction(ctx, ss); e != nil {
		return User{}, e
	}
	return s.GetUserWithConsistency(ctx, in.UserID, ReadStrong)
}
func (s *Store) BootstrapOwnerAtomic(ctx context.Context, c OwnerBootstrapCommand) (OwnerBootstrapResult, error) {
	var out OwnerBootstrapResult
	u, e := s.resolveScopedIdentity(ctx, c.Identity, ReadStrong)
	if e != nil {
		return out, e
	}
	id := requestID()
	if u != nil {
		id = u.ID
	}
	if c.ReservedUserID != "" {
		if u != nil && u.ID != c.ReservedUserID {
			return out, serviceError(409, "identity is already bound to another user")
		}
		id = c.ReservedUserID
	}
	now := nowString()
	in := c.Identity
	org := c.OrganizationID
	ss := []Statement{
		assertion("SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM organizations WHERE id=?) OR EXISTS(SELECT 1 FROM organizations WHERE id=? AND inactivated_at IS NULL)", org, org),
		assertion("SELECT 1 WHERE EXISTS(SELECT 1 FROM service_providers WHERE id='grant' AND active=1) AND EXISTS(SELECT 1 FROM roles WHERE id='role-system-bootstrap-owner')"),
		assertion("SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM user_identities WHERE provider=? AND issuer=? AND collection_id=? AND external_user_id=? AND user_id<>?)", in.Provider, in.Issuer, in.CollectionID, in.ExternalUserID, id),
		assertion("SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM users WHERE id=?) OR EXISTS(SELECT 1 FROM users WHERE id=? AND active=1)", id, id),
		assertion("SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM organization_memberships WHERE organization_id=? AND user_id=? AND (membership_role<>'owner' OR status<>'active'))", org, id),
		assertion("SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM organization_memberships WHERE organization_id=? AND user_id<>? AND membership_role='owner')", org, id),
		assertion("SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM role_bindings WHERE subject_type='user' AND subject_id=? AND subject_organization_id=? AND provider_id='grant' AND resource_type='organization' AND resource_id=? AND role_id='role-system-bootstrap-owner' AND state<>'active')", id, org, org),
	}
	if c.ReservedUserID != "" {
		ss = append(ss, assertion("SELECT 1 FROM users WHERE id=? AND active=1", id), assertion("SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM user_identities WHERE user_id=? AND provider=? AND issuer=? AND collection_id=? AND external_user_id<>?)", id, in.Provider, in.Issuer, in.CollectionID, in.ExternalUserID))
	}
	ss = append(ss,
		Statement{SQL: "INSERT INTO users(id,email,display_name,active,created_at) VALUES(?,?,?,1,?) ON CONFLICT(id) DO NOTHING", Args: []any{id, in.Email, in.DisplayName, now}},
		Statement{SQL: "INSERT INTO user_identities(id,user_id,provider,issuer,collection_id,external_user_id,email_snapshot,display_name_snapshot,last_seen_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(provider,issuer,collection_id,external_user_id) DO NOTHING", Args: []any{requestID(), id, in.Provider, in.Issuer, in.CollectionID, in.ExternalUserID, in.Email, in.DisplayName, now, now, now}},
		Statement{SQL: "INSERT INTO organizations(id,name,created_at,updated_at) VALUES(?,?,?,?) ON CONFLICT(id) DO NOTHING", Args: []any{org, org, now, now}},
		Statement{SQL: "INSERT INTO organization_memberships(id,organization_id,user_id,membership_role,status,created_at,updated_at) VALUES(?,?,?,'owner','active',?,?) ON CONFLICT(organization_id,user_id) DO NOTHING", Args: []any{requestID(), org, id, now, now}},
		Statement{SQL: "INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,state,created_by_user_id,created_at,updated_at) VALUES(?,'user',?,?,'grant','organization',?,'role-system-bootstrap-owner','active',?,?,?) ON CONFLICT(subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id) DO NOTHING", Args: []any{requestID(), id, org, org, id, now, now}},
		assertion("SELECT 1 FROM role_bindings WHERE subject_type='user' AND subject_id=? AND subject_organization_id=? AND provider_id='grant' AND resource_type='organization' AND resource_id=? AND role_id='role-system-bootstrap-owner' AND state='active'", id, org, org))
	if e = s.Transaction(ctx, ss); e != nil {
		return out, capabilityWriteError(e)
	}
	return OwnerBootstrapResult{UserID: id, OrganizationID: org}, nil
}
