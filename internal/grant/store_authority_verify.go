package grant

import (
	"context"
	"fmt"
)

// Replay must verify the currently materialized authority, not only its receipt.
func (s *Store) verifyAuthorityImport(ctx context.Context, c PatchDataAuthorityImport) error {
	check := func(q string, args ...any) error {
		r, e := s.Query(ctx, ReadStrong, "SELECT EXISTS("+q+") AS ok", args...)
		if e != nil {
			return e
		}
		if len(r) != 1 || rowInt(r[0], "ok") != 1 {
			return fmt.Errorf("Patch Data authority import state drift")
		}
		return nil
	}
	expected := len(c.Users) + len(c.Organizations) + len(c.Memberships) + len(c.PlantBindings) + len(c.ApplicationAssignments)
	if e := check("SELECT 1 WHERE (SELECT COUNT(*) FROM patch_data_authority_import_entities WHERE source_generation=?)=?", c.Generation, expected); e != nil {
		return e
	}
	for _, u := range c.Users {
		if e := check("SELECT 1 FROM users u JOIN user_identities ui ON ui.user_id=u.id WHERE u.id=? AND u.email IS ? AND u.display_name=? AND u.active=? AND ui.provider='patch-data' AND ui.external_user_id=? AND EXISTS(SELECT 1 FROM patch_data_authority_import_entities e WHERE e.entity_type='user' AND e.entity_id=u.id AND e.source_generation=?)", u.UserID, u.Email, u.DisplayName, u.Active, u.ExternalUserID, c.Generation); e != nil {
			return e
		}
	}
	for _, o := range c.Organizations {
		if e := check("SELECT 1 FROM organizations o WHERE o.id=? AND o.business_no IS NULL AND o.name=? AND ((?=1 AND o.inactivated_at IS NULL) OR (?=0 AND o.inactivated_at IS NOT NULL)) AND EXISTS(SELECT 1 FROM patch_data_authority_import_entities e WHERE e.entity_type='organization' AND e.entity_id=o.id AND e.source_generation=?)", o.OrganizationID, o.Name, o.Active, o.Active, c.Generation); e != nil {
			return e
		}
	}
	for _, m := range c.Memberships {
		if e := check("SELECT 1 FROM organization_memberships m WHERE m.id=? AND m.organization_id=? AND m.user_id=? AND m.membership_role=? AND m.status=? AND EXISTS(SELECT 1 FROM patch_data_authority_import_entities e WHERE e.entity_type='membership' AND e.entity_id=m.id AND e.source_generation=?)", m.MembershipID, m.OrganizationID, m.UserID, m.Role, m.Status, c.Generation); e != nil {
			return e
		}
	}
	for _, b := range c.PlantBindings {
		if e := check("SELECT 1 FROM role_bindings b WHERE b.id=? AND b.subject_type=? AND b.subject_id=? AND b.subject_organization_id=? AND b.provider_id='patch-data' AND b.resource_type='plant' AND b.resource_id=? AND b.role_id='role-patch-data-plant-reader' AND b.agreement_id IS NULL AND b.state=? AND EXISTS(SELECT 1 FROM patch_data_authority_import_entities e WHERE e.entity_type='plant_binding' AND e.entity_id=b.id AND e.source_generation=?)", b.BindingID, b.SubjectType, b.SubjectID, b.SubjectOrganizationID, b.PlantID, b.State, c.Generation); e != nil {
			return e
		}
	}
	for _, a := range c.ApplicationAssignments {
		if e := check("SELECT 1 FROM applications a WHERE a.id=? AND a.organization_id=? AND a.active=1 AND a.provisioning_status='ready' AND EXISTS(SELECT 1 FROM application_actions aa WHERE aa.application_id=a.id AND aa.action_id='patch-data.plant.read') AND EXISTS(SELECT 1 FROM patch_data_authority_import_entities e WHERE e.entity_type='application_assignment' AND e.entity_id=a.id AND e.source_generation=?)", a.ApplicationID, a.OrganizationID, c.Generation); e != nil {
			return e
		}
	}
	if e := check("SELECT 1 WHERE (SELECT COUNT(*) FROM organization_hierarchy_edges WHERE source_provider='patch-data' AND source_generation=?)=?", c.Generation, len(c.HierarchyEdges)); e != nil {
		return e
	}
	for _, edge := range c.HierarchyEdges {
		if e := check("SELECT 1 FROM organization_hierarchy_edges WHERE source_provider='patch-data' AND source_generation=? AND parent_organization_id=? AND child_organization_id=?", c.Generation, edge.ParentOrganizationID, edge.ChildOrganizationID); e != nil {
			return e
		}
	}
	return check("SELECT 1 FROM service_providers sp JOIN actions a ON a.provider_id=sp.id JOIN role_actions ra ON ra.action_id=a.id JOIN roles r ON r.id=ra.role_id WHERE sp.id='patch-data' AND sp.active=1 AND a.id='patch-data.plant.read' AND a.code=a.id AND a.grantable_to_applications=1 AND r.id='role-patch-data-plant-reader' AND r.organization_id IS NULL")
}
