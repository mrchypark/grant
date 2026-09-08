package grant

import (
	"context"
	"fmt"
)

type ExternalIdentityMaterialization struct {
	Provider, ExternalUserID string
	Issuer, CollectionID     string
	Email                    *string
	DisplayName              string
}
type ExternalOwnerBootstrapCommand struct {
	Identity                      ExternalIdentityMaterialization
	OrganizationID, ApplicationID string
}
type ExternalOwnerBootstrapResult struct {
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
func (s *Store) MaterializeExternalIdentity(ctx context.Context, in ExternalIdentityMaterialization) (User, error) {
	if in.Provider == "" || in.ExternalUserID == "" {
		return User{}, fmt.Errorf("external identity requires provider and subject")
	}
	if in.Provider == "pocketbase" {
		origin, e := canonicalPocketBaseOrigin(in.Issuer)
		if e != nil || origin != in.Issuer || !pocketBaseID(in.CollectionID) || !pocketBaseID(in.ExternalUserID) {
			return User{}, fmt.Errorf("invalid scoped PocketBase identity")
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
		return User{}, fmt.Errorf("materialized identity missing")
	}
	return *u, nil
}
func (s *Store) ResolveOrCreateUserIdentityWithConsistency(ctx context.Context, p, id string, email *string, name string, c ReadConsistency) (User, error) {
	if p != "rauthy" {
		return User{}, fmt.Errorf("unqualified identity resolution is only supported for Rauthy")
	}
	mapped, e := s.ResolveExistingUserIdentityWithConsistency(ctx, p, id, ReadStrong)
	if e != nil {
		return User{}, e
	}
	userID := p + ":" + id
	if mapped != nil {
		userID = mapped.ID
	} else {
		if email != nil {
			rows, e := s.Query(ctx, ReadStrong, "SELECT u.id FROM users u LEFT JOIN user_identities i ON i.user_id=u.id AND i.provider=? WHERE lower(u.email)=lower(?) ORDER BY u.id LIMIT 2", p, *email)
			if e != nil {
				return User{}, e
			}
			if len(rows) == 1 {
				userID = rowString(rows[0], "id")
			}
		}
		if userID == p+":"+id {
			rows, e := s.Query(ctx, ReadStrong, "SELECT id FROM users WHERE id=?", id)
			if e != nil {
				return User{}, e
			}
			if len(rows) > 0 {
				userID = id
			}
		}
	}
	now := nowString()
	err := s.Transaction(ctx, []Statement{{SQL: "INSERT INTO users(id,email,display_name,active,created_at) VALUES(?,?,?,1,?) ON CONFLICT(id) DO UPDATE SET email=excluded.email,display_name=excluded.display_name", Args: []any{userID, email, name, now}}, {SQL: "INSERT INTO user_identities(id,user_id,provider,external_user_id,email_snapshot,display_name_snapshot,last_seen_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?) ON CONFLICT(provider,issuer,collection_id,external_user_id) DO UPDATE SET email_snapshot=excluded.email_snapshot,display_name_snapshot=excluded.display_name_snapshot,last_seen_at=excluded.last_seen_at,updated_at=excluded.updated_at", Args: []any{requestID(), userID, p, id, email, name, now, now, now}}})
	if err != nil {
		return User{}, err
	}
	return s.GetUserWithConsistency(ctx, userID, c)
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
func (s *Store) BootstrapExternalOwnerAtomic(ctx context.Context, c ExternalOwnerBootstrapCommand) (ExternalOwnerBootstrapResult, error) {
	var out ExternalOwnerBootstrapResult
	u, e := s.resolveScopedIdentity(ctx, c.Identity, ReadStrong)
	if e != nil {
		return out, e
	}
	id := requestID()
	if u != nil {
		id = u.ID
	}
	now := nowString()
	in := c.Identity
	org, app := c.OrganizationID, c.ApplicationID
	// Admission and all owner/application mutations are one replicated transaction.
	ss := []Statement{assertion(`SELECT 1 WHERE
 (NOT EXISTS(SELECT 1 FROM applications WHERE id=$2 OR slug=$2) OR EXISTS(SELECT 1 FROM applications WHERE id=$2 AND organization_id=$3 AND slug=$2 AND name='PocketBase Application' AND description IS NULL AND application_type='confidential' AND app_type='server' AND rauthy_client_id IS NULL AND created_by_user_id=$4))
 AND (NOT EXISTS(SELECT 1 FROM organizations WHERE id=$3) OR EXISTS(SELECT 1 FROM organizations WHERE id=$3 AND inactivated_at IS NULL))
 AND EXISTS(SELECT 1 FROM service_providers WHERE id='grant' AND active=1) AND EXISTS(SELECT 1 FROM roles WHERE id='role-system-bootstrap-owner')
 AND NOT EXISTS(SELECT 1 FROM user_identities WHERE provider=$5 AND issuer=$7 AND collection_id=$8 AND external_user_id=$6 AND user_id<>$4)
 AND (NOT EXISTS(SELECT 1 FROM users WHERE id=$4) OR EXISTS(SELECT 1 FROM users WHERE id=$4 AND active=1))
 AND NOT EXISTS(SELECT 1 FROM organization_memberships WHERE organization_id=$3 AND user_id=$4 AND (membership_role<>'owner' OR status<>'active'))
 AND NOT EXISTS(SELECT 1 FROM organization_memberships WHERE organization_id=$3 AND user_id<>$4 AND membership_role='owner')
 AND NOT EXISTS(SELECT 1 FROM role_bindings WHERE subject_type='user' AND subject_id=$4 AND subject_organization_id=$3 AND provider_id='grant' AND resource_type='organization' AND resource_id=$3 AND role_id='role-system-bootstrap-owner' AND state<>'active')`, app, org, id, in.Provider, in.ExternalUserID, in.Issuer, in.CollectionID)}
	ss = append(ss,
		Statement{SQL: "INSERT INTO users(id,email,display_name,active,created_at) VALUES(?,?,?,1,?) ON CONFLICT(id) DO NOTHING", Args: []any{id, in.Email, in.DisplayName, now}},
		Statement{SQL: "INSERT INTO user_identities(id,user_id,provider,issuer,collection_id,external_user_id,email_snapshot,display_name_snapshot,last_seen_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(provider,issuer,collection_id,external_user_id) DO NOTHING", Args: []any{requestID(), id, in.Provider, in.Issuer, in.CollectionID, in.ExternalUserID, in.Email, in.DisplayName, now, now, now}},
		Statement{SQL: "INSERT INTO organizations(id,name,created_at,updated_at) VALUES(?,?,?,?) ON CONFLICT(id) DO NOTHING", Args: []any{org, org, now, now}},
		Statement{SQL: "INSERT INTO applications(id,organization_id,slug,name,application_type,app_type,first_party,consent_policy,active,provisioning_status,created_by_user_id,created_at,updated_at) VALUES(?,?,?,'PocketBase Application','confidential','server',1,'auto_approve',1,'ready',?,?,?) ON CONFLICT(id) DO NOTHING", Args: []any{app, org, app, id, now, now}},
		Statement{SQL: "INSERT INTO organization_memberships(id,organization_id,user_id,membership_role,status,created_at,updated_at) VALUES(?,?,?,'owner','active',?,?) ON CONFLICT(organization_id,user_id) DO NOTHING", Args: []any{requestID(), org, id, now, now}},
		Statement{SQL: "INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,state,created_by_user_id,created_at,updated_at) VALUES(?,'user',?,?,'grant','organization',?,'role-system-bootstrap-owner','active',?,?,?) ON CONFLICT(subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id) DO NOTHING", Args: []any{requestID(), id, org, org, id, now, now}},
		assertion("SELECT 1 FROM role_bindings WHERE subject_type='user' AND subject_id=? AND subject_organization_id=? AND provider_id='grant' AND resource_type='organization' AND resource_id=? AND role_id='role-system-bootstrap-owner' AND state='active'", id, org, org),
		assertion("SELECT 1 FROM applications WHERE id=? AND organization_id=? AND active=1 AND provisioning_status='ready' AND created_by_user_id=?", app, org, id))
	if e = s.Transaction(ctx, ss); e != nil {
		return out, e
	}
	return ExternalOwnerBootstrapResult{UserID: id, OrganizationID: org}, nil
}
