package grant

import (
	"context"
	"fmt"
)

func (s *Service) ListAgreementsAuthorized(ctx context.Context, user, org string, c ReadConsistency) ([]Agreement, error) {
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.permission.manage", "manage organization agreements", c); err != nil {
		return nil, err
	}
	return serviceRows[Agreement](ctx, s, c, "SELECT * FROM agreements WHERE grantor_organization_id=? OR grantee_organization_id=? ORDER BY updated_at DESC,id", org, org)
}
func (s *Service) GetAgreementAuthorized(ctx context.Context, user, org, id string, c ReadConsistency) (Agreement, error) {
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.permission.manage", "manage organization agreements", c); err != nil {
		return Agreement{}, err
	}
	a, err := serviceRow[Agreement](ctx, s, c, "SELECT * FROM agreements WHERE id=?", id)
	if err != nil {
		return a, err
	}
	if a.GrantorOrganizationID != org && a.GranteeOrganizationID != org {
		return Agreement{}, serviceError(403, "forbidden: agreement is outside organization scope")
	}
	return a, nil
}
func (s *Service) agreement(ctx context.Context, user, org string, in UpsertAgreementInput, create bool) (Agreement, error) {
	if !enumValue(in.AgreementType, "delegation", "epc", "om", "distributor", "dealer") || !enumValue(in.Status, "draft", "active", "expired", "terminated") {
		return Agreement{}, serviceError(400, "invalid agreement type or status")
	}
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.permission.manage", "manage organization agreements", ReadStrong); err != nil {
		return Agreement{}, err
	}
	v := []string{in.ID, in.GrantorOrganizationID, in.GranteeOrganizationID, in.Title}
	labels := []string{"id", "grantorOrganizationId", "granteeOrganizationId", "title"}
	for i := range v {
		x, err := required(labels[i], v[i])
		if err != nil {
			return Agreement{}, err
		}
		v[i] = x
	}
	a := Agreement{ID: v[0], AgreementType: in.AgreementType, GrantorOrganizationID: v[1], GranteeOrganizationID: v[2], Status: in.Status, Title: v[3], ReferenceNo: optionalText(in.ReferenceNo), StartsAt: optionalText(in.StartsAt), EndsAt: optionalText(in.EndsAt)}
	if a.GrantorOrganizationID != org && a.GranteeOrganizationID != org {
		return a, serviceError(403, "forbidden: agreement is outside organization scope")
	}
	query := "INSERT INTO agreements(id,agreement_type,grantor_organization_id,grantee_organization_id,status,title,reference_no,starts_at,ends_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)"
	if !create {
		query += " ON CONFLICT(id) DO UPDATE SET agreement_type=excluded.agreement_type,status=excluded.status,title=excluded.title,reference_no=excluded.reference_no,starts_at=excluded.starts_at,ends_at=excluded.ends_at,updated_at=excluded.updated_at"
	}
	stmts := []Statement{assertion("SELECT 1 FROM organizations WHERE id=?", a.GrantorOrganizationID), assertion("SELECT 1 FROM organizations WHERE id=?", a.GranteeOrganizationID), {SQL: query, Args: []any{a.ID, a.AgreementType, a.GrantorOrganizationID, a.GranteeOrganizationID, a.Status, a.Title, a.ReferenceNo, a.StartsAt, a.EndsAt, nowString(), nowString()}}}
	return a, s.store.Transaction(ctx, stmts)
}
func (s *Service) CreateAgreementAuthorized(ctx context.Context, user, org string, in UpsertAgreementInput) (Agreement, error) {
	if in.GrantorOrganizationID != org {
		return Agreement{}, serviceError(403, fmt.Sprintf("forbidden: only grantor organization %s can create agreement %s", in.GrantorOrganizationID, in.ID))
	}
	return s.agreement(ctx, user, org, in, true)
}
func (s *Service) UpdateAgreementAuthorized(ctx context.Context, user, org, id string, in UpsertAgreementInput) (Agreement, error) {
	a, err := s.GetAgreementAuthorized(ctx, user, org, id, ReadStrong)
	if err != nil {
		return a, err
	}
	if in.ID != id {
		return a, serviceError(400, "agreement id must match path id")
	}
	if in.GrantorOrganizationID != a.GrantorOrganizationID || in.GranteeOrganizationID != a.GranteeOrganizationID {
		return a, serviceError(400, "agreement grantorOrganizationId and granteeOrganizationId cannot be changed")
	}
	if org != a.GrantorOrganizationID && (in.Status != a.Status || ptrValue(in.StartsAt) != ptrValue(a.StartsAt) || ptrValue(in.EndsAt) != ptrValue(a.EndsAt)) {
		return a, serviceError(403, fmt.Sprintf("forbidden: only grantor organization %s can change agreement authorization terms", a.GrantorOrganizationID))
	}
	return s.agreement(ctx, user, org, in, false)
}
