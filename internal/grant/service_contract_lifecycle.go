package grant

import (
	"bytes"
	"context"
	"encoding/json"
	"reflect"
	"strings"
)

// PatchText distinguishes an omitted field from an explicit JSON null (clear).
type PatchText struct {
	Present bool
	Value   *string
}

func (p *PatchText) UnmarshalJSON(data []byte) error {
	p.Present = true
	return json.Unmarshal(data, &p.Value)
}

type PatchServiceProviderInput struct {
	Name        *string   `json:"name"`
	Description PatchText `json:"description"`
	Active      *bool     `json:"active"`
}
type PatchGroupInput struct {
	Name        *string   `json:"name"`
	Description PatchText `json:"description"`
}
type PatchOrganizationInput struct {
	Name *string `json:"name"`
}
type CreateOrganizationInput struct {
	ID          string `json:"id"`
	Name        string `json:"name"`
	OwnerUserID string `json:"ownerUserId"`
}
type PatchAgreementInput struct {
	Title         *string   `json:"title"`
	AgreementType *string   `json:"agreementType"`
	Status        *string   `json:"status"`
	ReferenceNo   PatchText `json:"referenceNo"`
	StartsAt      PatchText `json:"startsAt"`
	EndsAt        PatchText `json:"endsAt"`
}

func (s *Service) CreateServiceProviderAdmin(ctx context.Context, p ServiceProvider) (ServiceProvider, error) {
	var err error
	if p.ID, err = required("id", p.ID); err != nil {
		return p, err
	}
	if p.Name, err = required("name", p.Name); err != nil {
		return p, err
	}
	p.Description = optionalText(p.Description)
	// A conditional insert, rather than an upsert, also handles concurrent creates.
	err = s.store.Transaction(ctx, []Statement{{SQL: "INSERT INTO service_providers(id,name,description,active,created_at,updated_at) VALUES(?,?,?,?,?,?) ON CONFLICT(id) DO NOTHING", Args: []any{p.ID, p.Name, p.Description, p.Active, nowString(), nowString()}, ExpectedRowsAffected: requireOne()}})
	if err != nil {
		if rows, e := s.store.Query(ctx, ReadStrong, "SELECT id FROM service_providers WHERE id=?", p.ID); e == nil && len(rows) > 0 {
			return p, serviceError(409, "service provider already exists")
		}
		return p, err
	}
	return p, nil
}

// patchColumns only writes supplied fields so independent metadata edits do not
// overwrite one another through a read/merge/full-row write.
func (s *Service) patchColumns(ctx context.Context, table, predicate string, keys []string, values, scope []any, guards ...Statement) error {
	if len(keys) == 0 {
		return serviceError(400, "patch must contain at least one field")
	}
	keys = append(keys, "updated_at=?")
	values = append(values, nowString())
	values = append(values, scope...)
	return lifecycleError(s.store.Transaction(ctx, append(guards, Statement{SQL: "UPDATE " + table + " SET " + strings.Join(keys, ",") + " WHERE " + predicate, Args: values, ExpectedRowsAffected: requireOne()})))
}
func patchName(name *string, keys *[]string, values *[]any) error {
	if name == nil {
		return nil
	}
	v, err := required("name", *name)
	if err != nil {
		return err
	}
	*keys = append(*keys, "name=?")
	*values = append(*values, v)
	return nil
}
func patchTextField(column string, p PatchText, keys *[]string, values *[]any) {
	if p.Present {
		*keys = append(*keys, column+"=?")
		*values = append(*values, optionalText(p.Value))
	}
}
func (s *Service) PatchServiceProviderAdmin(ctx context.Context, id string, in PatchServiceProviderInput) (ServiceProvider, error) {
	if _, err := serviceRow[ServiceProvider](ctx, s, ReadStrong, "SELECT * FROM service_providers WHERE id=?", id); err != nil {
		return ServiceProvider{}, err
	}
	var keys []string
	var values []any
	if err := patchName(in.Name, &keys, &values); err != nil {
		return ServiceProvider{}, err
	}
	patchTextField("description", in.Description, &keys, &values)
	if in.Active != nil {
		keys = append(keys, "active=?")
		values = append(values, *in.Active)
	}
	if err := s.patchColumns(ctx, "service_providers", "id=?", keys, values, []any{id}); err != nil {
		return ServiceProvider{}, err
	}
	return serviceRow[ServiceProvider](ctx, s, ReadStrong, "SELECT * FROM service_providers WHERE id=?", id)
}
func (s *Service) PatchGroupAuthorized(ctx context.Context, user, org, id string, in PatchGroupInput) (Group, error) {
	if _, err := s.GetGroupAuthorized(ctx, user, org, id, ReadStrong); err != nil {
		return Group{}, err
	}
	var keys []string
	var values []any
	if err := patchName(in.Name, &keys, &values); err != nil {
		return Group{}, err
	}
	patchTextField("description", in.Description, &keys, &values)
	if err := s.patchColumns(ctx, "groups", "id=? AND organization_id=?", keys, values, []any{id, org}, lifecycleActorGuard(user, org, "organization.group.manage")); err != nil {
		return Group{}, err
	}
	return serviceRow[Group](ctx, s, ReadStrong, "SELECT * FROM groups WHERE id=? AND organization_id=?", id, org)
}
func (s *Service) ListOrganizationsAuthorized(ctx context.Context, user string, c ReadConsistency) ([]Organization, error) {
	return serviceRows[Organization](ctx, s, c, "SELECT o.* FROM organizations o JOIN organization_memberships m ON m.organization_id=o.id JOIN users u ON u.id=m.user_id WHERE m.user_id=? AND m.status='active' AND u.active=1 ORDER BY o.name,o.id", user)
}
func (s *Service) GetOrganizationAuthorized(ctx context.Context, user, org string, c ReadConsistency) (Organization, error) {
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.profile.read", "read organization profile", c); err != nil {
		return Organization{}, err
	}
	return serviceRow[Organization](ctx, s, c, "SELECT * FROM organizations WHERE id=?", org)
}
func (s *Service) PatchOrganizationAuthorized(ctx context.Context, user, org string, in PatchOrganizationInput) (Organization, error) {
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.permission.manage", "manage organization profile", ReadStrong); err != nil {
		return Organization{}, err
	}
	var keys []string
	var values []any
	if err := patchName(in.Name, &keys, &values); err != nil {
		return Organization{}, err
	}
	if err := s.patchColumns(ctx, "organizations", "id=?", keys, values, []any{org}, lifecycleActorGuard(user, org, "organization.permission.manage")); err != nil {
		return Organization{}, err
	}
	return serviceRow[Organization](ctx, s, ReadStrong, "SELECT * FROM organizations WHERE id=?", org)
}

// CreateOrganizationAdmin is only for platform-authorized callers. OwnerUserID
// identifies an existing active Grant user; organization creation never creates
// an identity or promotes the caller implicitly.
func (s *Service) CreateOrganizationAdmin(ctx context.Context, in CreateOrganizationInput) (Organization, error) {
	return s.createOrganization(ctx, in, nil)
}
func (s *Service) createOrganization(ctx context.Context, in CreateOrganizationInput, extra []Statement) (Organization, error) {
	var err error
	if in.ID, err = required("id", in.ID); err != nil {
		return Organization{}, err
	}
	if in.Name, err = required("name", in.Name); err != nil {
		return Organization{}, err
	}
	if in.OwnerUserID, err = required("ownerUserId", in.OwnerUserID); err != nil {
		return Organization{}, err
	}
	users, err := s.store.Query(ctx, ReadStrong, "SELECT id FROM users WHERE id=? AND active=1", in.OwnerUserID)
	if err != nil {
		return Organization{}, err
	}
	if len(users) != 1 {
		return Organization{}, serviceError(400, "owner must be an existing active Grant user")
	}
	now := nowString()
	statements := []Statement{
		assertion("SELECT 1 FROM users WHERE id=? AND active=1", in.OwnerUserID),
		assertion("SELECT 1 FROM roles WHERE id='role-system-bootstrap-owner' AND organization_id IS NULL"),
		{SQL: "INSERT INTO organizations(id,name,created_at,updated_at) VALUES(?,?,?,?) ON CONFLICT(id) DO NOTHING", Args: []any{in.ID, in.Name, now, now}, ExpectedRowsAffected: requireOne()},
		{SQL: "INSERT INTO organization_memberships(id,organization_id,user_id,membership_role,status,created_at,updated_at) VALUES(?,?,?,'owner','active',?,?)", Args: []any{requestID(), in.ID, in.OwnerUserID, now, now}},
		// Retain the historical system-owner binding as immutable audit provenance.
		// Authorization treats membership as canonical and rejects this binding for non-owners.
		{SQL: "INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,state,created_by_user_id,created_at,updated_at) VALUES(?,'user',?,?,'grant','organization',?,'role-system-bootstrap-owner','active',?,?,?)", Args: []any{requestID(), in.OwnerUserID, in.ID, in.ID, ManagementActorFromContext(ctx), now, now}},
	}
	err = s.store.Transaction(ctx, append(statements, extra...))
	if err != nil {
		if rows, e := s.store.Query(ctx, ReadStrong, "SELECT id FROM organizations WHERE id=?", in.ID); e == nil && len(rows) > 0 {
			return Organization{}, serviceError(409, "organization already exists")
		}
		return Organization{}, err
	}
	return Organization{ID: in.ID, Name: in.Name}, nil
}
func (s *Service) PatchAgreementAuthorized(ctx context.Context, user, org, id string, in PatchAgreementInput) (Agreement, error) {
	a, err := s.GetAgreementAuthorized(ctx, user, org, id, ReadStrong)
	if err != nil {
		return Agreement{}, err
	}
	if org != a.GrantorOrganizationID && (in.Status != nil || in.StartsAt.Present || in.EndsAt.Present) {
		return Agreement{}, serviceError(403, "only the grantor organization can change agreement authorization terms")
	}
	var keys []string
	var values []any
	if in.Title != nil {
		v, e := required("title", *in.Title)
		if e != nil {
			return Agreement{}, e
		}
		keys = append(keys, "title=?")
		values = append(values, v)
	}
	if in.AgreementType != nil {
		typ, err := normalizeAgreementType(*in.AgreementType)
		if err != nil {
			return Agreement{}, err
		}
		keys = append(keys, "agreement_type=?")
		values = append(values, typ)
	}
	if in.Status != nil {
		if !enumValue(*in.Status, "draft", "active", "expired", "terminated") {
			return Agreement{}, serviceError(400, "invalid agreement status")
		}
		keys = append(keys, "status=?")
		values = append(values, *in.Status)
	}
	patchTextField("reference_no", in.ReferenceNo, &keys, &values)
	guards := []Statement{lifecycleActorGuard(user, org, "organization.permission.manage")}
	if in.StartsAt.Present || in.EndsAt.Present {
		start, end := a.StartsAt, a.EndsAt
		if in.StartsAt.Present {
			start = in.StartsAt.Value
		}
		if in.EndsAt.Present {
			end = in.EndsAt.Value
		}
		start, end, err = normalizeAgreementInterval(start, end)
		if err != nil {
			return Agreement{}, err
		}
		keys = append(keys, "starts_at=?", "ends_at=?")
		values = append(values, start, end)
		guards = append(guards, agreementBoundsGuard(a))
	}
	if err = s.patchColumns(ctx, "agreements", "id=? AND (grantor_organization_id=? OR grantee_organization_id=?)", keys, values, []any{id, org, org}, guards...); err != nil {
		return Agreement{}, err
	}
	return serviceRow[Agreement](ctx, s, ReadStrong, "SELECT * FROM agreements WHERE id=?", id)
}

func decodeContractPatch(data []byte, dst any, nonnullable ...string) error {
	var fields map[string]json.RawMessage
	if err := json.Unmarshal(data, &fields); err != nil {
		return err
	}
	if fields == nil {
		return serviceError(400, "patch must be a JSON object")
	}
	shape := reflect.TypeOf(dst).Elem()
	allowed := make(map[string]bool, shape.NumField())
	for i := 0; i < shape.NumField(); i++ {
		allowed[strings.Split(shape.Field(i).Tag.Get("json"), ",")[0]] = true
	}
	for field := range fields {
		if !allowed[field] {
			return serviceError(400, "unknown or immutable patch field: "+field)
		}
	}
	for _, field := range nonnullable {
		if raw, ok := fields[field]; ok && bytes.Equal(bytes.TrimSpace(raw), []byte("null")) {
			return serviceError(400, field+" cannot be null")
		}
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	return decoder.Decode(dst)
}
func (p *PatchServiceProviderInput) UnmarshalJSON(data []byte) error {
	type plain PatchServiceProviderInput
	return decodeContractPatch(data, (*plain)(p), "name", "active")
}
func (p *PatchGroupInput) UnmarshalJSON(data []byte) error {
	type plain PatchGroupInput
	return decodeContractPatch(data, (*plain)(p), "name")
}
func (p *PatchOrganizationInput) UnmarshalJSON(data []byte) error {
	type plain PatchOrganizationInput
	return decodeContractPatch(data, (*plain)(p), "name")
}
func (p *PatchAgreementInput) UnmarshalJSON(data []byte) error {
	type plain PatchAgreementInput
	return decodeContractPatch(data, (*plain)(p), "title", "agreementType", "status")
}
