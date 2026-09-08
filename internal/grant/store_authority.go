package grant

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"strings"
)

func validatePatchImport(c PatchDataAuthorityImport) error {
	if c.ManifestVersion != "grant.patch-data-authority.v1" || strictToken(c.Generation, 255) != nil || len(c.LedgerDigest) != 64 || !validDigest("sha256:"+c.LedgerDigest) || !validDigest("sha256:"+c.ManifestDigest) {
		return fmt.Errorf("invalid authority manifest metadata")
	}
	if c.ExpectedPreviousGeneration != nil && (*c.ExpectedPreviousGeneration == c.Generation || strictToken(*c.ExpectedPreviousGeneration, 255) != nil) {
		return fmt.Errorf("invalid previous generation")
	}
	if len(c.Users)+len(c.Organizations)+len(c.Memberships)+len(c.HierarchyEdges)+len(c.PlantBindings)+len(c.ApplicationAssignments) > 10000 {
		return fmt.Errorf("authority manifest exceeds entity limit")
	}
	users, exts, orgs, pairs, active := map[string]bool{}, map[string]bool{}, map[string]bool{}, map[string]bool{}, map[string]bool{}
	prev := ""
	for _, u := range c.Users {
		key := u.UserID + "\x00" + u.ExternalUserID
		if strictToken(u.UserID, 255) != nil || strictToken(u.ExternalUserID, 255) != nil || strictToken(u.DisplayName, 255) != nil || (u.Email != nil && strictToken(*u.Email, 320) != nil) || key <= prev || users[u.UserID] || exts[u.ExternalUserID] {
			return fmt.Errorf("users must be valid sorted unique identities")
		}
		prev = key
		users[u.UserID] = true
		exts[u.ExternalUserID] = true
	}
	prev = ""
	for _, o := range c.Organizations {
		if strictToken(o.OrganizationID, 255) != nil || strictToken(o.Name, 255) != nil || o.OrganizationID <= prev {
			return fmt.Errorf("organizations must be valid sorted unique")
		}
		prev = o.OrganizationID
		orgs[o.OrganizationID] = true
	}
	prev = ""
	for _, m := range c.Memberships {
		key := m.OrganizationID + "\x00" + m.UserID
		if strictToken(m.MembershipID, 255) != nil || m.MembershipID <= prev || !users[m.UserID] || !orgs[m.OrganizationID] || pairs[key] || (m.Role != "owner" && m.Role != "admin" && m.Role != "member") || (m.Status != "active" && m.Status != "disabled") {
			return fmt.Errorf("invalid membership references/order")
		}
		prev = m.MembershipID
		pairs[key] = true
		if m.Status == "active" {
			active[key] = true
		}
	}
	parents := map[string]string{}
	prev = ""
	for _, e := range c.HierarchyEdges {
		key := e.ParentOrganizationID + "\x00" + e.ChildOrganizationID
		if !orgs[e.ParentOrganizationID] || !orgs[e.ChildOrganizationID] || e.ParentOrganizationID == e.ChildOrganizationID || key <= prev || parents[e.ChildOrganizationID] != "" {
			return fmt.Errorf("invalid hierarchy")
		}
		parents[e.ChildOrganizationID] = e.ParentOrganizationID
		prev = key
	}
	for child := range parents {
		seen := map[string]bool{}
		for child != "" {
			if seen[child] {
				return fmt.Errorf("hierarchy cycle")
			}
			seen[child] = true
			child = parents[child]
		}
	}
	prev = ""
	for _, b := range c.PlantBindings {
		if strictToken(b.BindingID, 255) != nil || strictToken(b.PlantID, 255) != nil || b.BindingID <= prev || (b.State != "active" && b.State != "disabled") {
			return fmt.Errorf("invalid plant binding")
		}
		prev = b.BindingID
		switch b.SubjectType {
		case "user":
			if !active[b.SubjectOrganizationID+"\x00"+b.SubjectID] {
				return fmt.Errorf("plant binding lacks active membership")
			}
		case "organization":
			if b.SubjectID != b.SubjectOrganizationID || !orgs[b.SubjectID] {
				return fmt.Errorf("invalid organization binding")
			}
		default:
			return fmt.Errorf("unsupported plant subject")
		}
	}
	prev = ""
	for _, a := range c.ApplicationAssignments {
		if strictToken(a.ApplicationID, 255) != nil || a.ApplicationID <= prev || !orgs[a.OrganizationID] {
			return fmt.Errorf("invalid application assignment")
		}
		prev = a.ApplicationID
	}
	if PatchDataManifestDigest(c) != c.ManifestDigest {
		return fmt.Errorf("manifestDigest does not match canonical manifest")
	}
	return nil
}
func (s *Store) ImportPatchDataAuthority(ctx context.Context, c PatchDataAuthorityImport) (PatchDataAuthorityImportResult, error) {
	var result PatchDataAuthorityImportResult
	if e := validatePatchImport(c); e != nil {
		return result, e
	}
	old, e := s.Query(ctx, ReadStrong, "SELECT * FROM patch_data_authority_imports WHERE provider_id='patch-data'")
	if e != nil {
		return result, e
	}
	if len(old) > 0 && rowString(old[0], "generation") == c.Generation && rowString(old[0], "foundation_digest") == c.ManifestDigest && rowString(old[0], "ledger_digest") == c.LedgerDigest {
		if e = s.verifyAuthorityImport(ctx, c); e != nil {
			return result, e
		}
		result = PatchDataAuthorityImportResult{Applied: false, ManifestVersion: c.ManifestVersion, Generation: c.Generation, LedgerDigest: c.LedgerDigest, ManifestDigest: c.ManifestDigest, AppliedAt: rowString(old[0], "applied_at")}
		return result, nil
	}
	// Each input collection is a bound JSON value. Statement count is independent of entity count.
	raw, _ := json.Marshal(c)
	j := string(raw)
	var identities []map[string]any
	for _, u := range c.Users {
		h := sha256.Sum256([]byte(u.ExternalUserID))
		identities = append(identities, map[string]any{"identityId": "identity-patch-data-" + hex.EncodeToString(h[:]), "userId": u.UserID, "externalUserId": u.ExternalUserID, "email": u.Email, "displayName": u.DisplayName})
	}
	ij, _ := jsonString(identities)
	now := nowString()
	ss := []Statement{assertion("SELECT 1 WHERE (? IS NULL AND NOT EXISTS(SELECT 1 FROM patch_data_authority_imports WHERE provider_id='patch-data')) OR EXISTS(SELECT 1 FROM patch_data_authority_imports WHERE provider_id='patch-data' AND generation=?)", c.ExpectedPreviousGeneration, c.ExpectedPreviousGeneration),
		assertion("SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM group_memberships gm JOIN patch_data_authority_import_entities e ON e.entity_type='membership' AND e.entity_id=gm.organization_membership_id) AND NOT EXISTS(SELECT 1 FROM actions WHERE (id='patch-data.plant.read' AND (provider_id<>'patch-data' OR code<>'patch-data.plant.read')) OR (provider_id='patch-data' AND code='patch-data.plant.read' AND id<>'patch-data.plant.read')) AND NOT EXISTS(SELECT 1 FROM roles WHERE (id='role-patch-data-plant-reader' AND (organization_id IS NOT NULL OR code<>'role-patch-data-plant-reader')) OR (organization_id IS NULL AND code='role-patch-data-plant-reader' AND id<>'role-patch-data-plant-reader'))"),
		assertion("SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM json_each(?, '$.users') j JOIN users u ON u.id=j.value->>'userId' WHERE NOT EXISTS(SELECT 1 FROM patch_data_authority_import_entities e WHERE e.entity_type='user' AND e.entity_id=u.id))", j),
		assertion("SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM json_each(?) j JOIN user_identities i ON (i.provider='patch-data' AND i.external_user_id=j.value->>'externalUserId') OR i.id=j.value->>'identityId' WHERE i.provider<>'patch-data' OR i.external_user_id<>j.value->>'externalUserId' OR i.user_id<>j.value->>'userId')", ij),
		assertion("SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM json_each(?, '$.organizations') j JOIN organizations o ON o.id=j.value->>'organizationId' WHERE o.business_no IS NOT NULL OR NOT EXISTS(SELECT 1 FROM patch_data_authority_import_entities e WHERE e.entity_type='organization' AND e.entity_id=o.id))", j),
		assertion("SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM json_each(?, '$.memberships') j JOIN organization_memberships m ON m.id=j.value->>'membershipId' OR (m.organization_id=j.value->>'organizationId' AND m.user_id=j.value->>'userId') WHERE NOT EXISTS(SELECT 1 FROM patch_data_authority_import_entities e WHERE e.entity_type='membership' AND e.entity_id=m.id))", j),
		assertion("SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM json_each(?, '$.plantBindings') j JOIN role_bindings b ON b.id=j.value->>'bindingId' WHERE NOT EXISTS(SELECT 1 FROM patch_data_authority_import_entities e WHERE e.entity_type='plant_binding' AND e.entity_id=b.id))", j),
		assertion("SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM json_each(?, '$.applicationAssignments') j WHERE NOT EXISTS(SELECT 1 FROM applications a WHERE a.id=j.value->>'applicationId' AND a.organization_id=j.value->>'organizationId' AND a.active=1 AND a.provisioning_status='ready') OR (EXISTS(SELECT 1 FROM application_actions aa WHERE aa.application_id=j.value->>'applicationId' AND aa.action_id='patch-data.plant.read') AND NOT EXISTS(SELECT 1 FROM patch_data_authority_import_entities e WHERE e.entity_type='application_assignment' AND e.entity_id=j.value->>'applicationId')))", j),
		{SQL: "DELETE FROM role_bindings WHERE id IN(SELECT entity_id FROM patch_data_authority_import_entities WHERE entity_type='plant_binding')"}, {SQL: "DELETE FROM organization_memberships WHERE id IN(SELECT entity_id FROM patch_data_authority_import_entities WHERE entity_type='membership')"}, {SQL: "DELETE FROM application_actions WHERE action_id='patch-data.plant.read' AND application_id IN(SELECT entity_id FROM patch_data_authority_import_entities WHERE entity_type='application_assignment')"}, {SQL: "DELETE FROM patch_data_authority_import_entities"}, {SQL: "DELETE FROM organization_hierarchy_edges WHERE source_provider='patch-data'"},
		{SQL: "INSERT INTO service_providers(id,name,active,created_at,updated_at) VALUES('patch-data','Patch Data',1,?,?) ON CONFLICT(id) DO UPDATE SET active=1,updated_at=excluded.updated_at", Args: []any{now, now}},
		{SQL: "INSERT INTO actions(id,provider_id,code,name,risk_level,visibility_granting,grantable_to_applications,created_at,updated_at) VALUES('patch-data.plant.read','patch-data','patch-data.plant.read','Read Patch Data plant','low',0,1,?,?) ON CONFLICT(id) DO UPDATE SET visibility_granting=0,grantable_to_applications=1,updated_at=excluded.updated_at", Args: []any{now, now}},
		{SQL: "INSERT INTO roles(id,organization_id,code,name,created_at,updated_at) VALUES('role-patch-data-plant-reader',NULL,'role-patch-data-plant-reader','Patch Data plant reader',?,?) ON CONFLICT(id) DO NOTHING", Args: []any{now, now}},
		{SQL: "INSERT INTO role_actions(role_id,action_id) VALUES('role-patch-data-plant-reader','patch-data.plant.read') ON CONFLICT DO NOTHING"},
		{SQL: "INSERT INTO users(id,email,display_name,active,created_at) SELECT value->>'userId',value->>'email',value->>'displayName',value->>'active',? FROM json_each(?,'$.users') WHERE 1 ON CONFLICT(id) DO UPDATE SET email=excluded.email,display_name=excluded.display_name,active=excluded.active", Args: []any{now, j}},
		{SQL: "INSERT INTO user_identities(id,user_id,provider,external_user_id,email_snapshot,display_name_snapshot,last_seen_at,created_at,updated_at) SELECT value->>'identityId',value->>'userId','patch-data',value->>'externalUserId',value->>'email',value->>'displayName',?,?,? FROM json_each(?) WHERE 1 ON CONFLICT(id) DO UPDATE SET email_snapshot=excluded.email_snapshot,display_name_snapshot=excluded.display_name_snapshot,last_seen_at=excluded.last_seen_at,updated_at=excluded.updated_at", Args: []any{now, now, now, ij}},
		{SQL: "INSERT INTO organizations(id,name,inactivated_at,created_at,updated_at) SELECT value->>'organizationId',value->>'name',CASE WHEN value->>'active'=1 THEN NULL ELSE ? END,?,? FROM json_each(?,'$.organizations') WHERE 1 ON CONFLICT(id) DO UPDATE SET name=excluded.name,inactivated_at=excluded.inactivated_at,updated_at=excluded.updated_at", Args: []any{now, now, now, j}},
		{SQL: "INSERT INTO organization_memberships(id,organization_id,user_id,membership_role,status,created_at,updated_at) SELECT value->>'membershipId',value->>'organizationId',value->>'userId',value->>'role',value->>'status',?,? FROM json_each(?,'$.memberships')", Args: []any{now, now, j}},
		{SQL: "INSERT INTO organization_hierarchy_edges(source_provider,parent_organization_id,child_organization_id,source_generation,created_at) SELECT 'patch-data',value->>'parentOrganizationId',value->>'childOrganizationId',?,? FROM json_each(?,'$.hierarchyEdges')", Args: []any{c.Generation, now, j}},
		{SQL: "INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,state,created_by_user_id,created_at,updated_at) SELECT value->>'bindingId',value->>'subjectType',value->>'subjectId',value->>'subjectOrganizationId','patch-data','plant',value->>'plantId','role-patch-data-plant-reader',value->>'state',?,?,? FROM json_each(?,'$.plantBindings')", Args: []any{s.cfg.Security.BootstrapUserID, now, now, j}},
		{SQL: "INSERT INTO application_actions(application_id,action_id) SELECT value->>'applicationId','patch-data.plant.read' FROM json_each(?,'$.applicationAssignments')", Args: []any{j}},
	}
	for _, kind := range []struct{ kind, collection, key string }{{"user", "users", "userId"}, {"organization", "organizations", "organizationId"}, {"membership", "memberships", "membershipId"}, {"plant_binding", "plantBindings", "bindingId"}, {"application_assignment", "applicationAssignments", "applicationId"}} {
		ss = append(ss, Statement{SQL: fmt.Sprintf("INSERT INTO patch_data_authority_import_entities(entity_type,entity_id,source_generation) SELECT ?,value->>'%s',? FROM json_each(?,'$.%s')", kind.key, kind.collection), Args: []any{kind.kind, c.Generation, j}})
	}
	ss = append(ss, Statement{SQL: "INSERT INTO patch_data_authority_imports(provider_id,generation,ledger_digest,foundation_digest,applied_at,manifest_version) VALUES('patch-data',?,?,?,?,?) ON CONFLICT(provider_id) DO UPDATE SET generation=excluded.generation,ledger_digest=excluded.ledger_digest,foundation_digest=excluded.foundation_digest,applied_at=excluded.applied_at,manifest_version=excluded.manifest_version", Args: []any{c.Generation, c.LedgerDigest, c.ManifestDigest, now, c.ManifestVersion}})
	stageID := strings.TrimPrefix(digestBytes(raw), "sha256:")
	if e = s.stageImport(ctx, stageID, c, identities); e != nil {
		return result, e
	}
	ss = stagedStatements(stageID, j, ij, ss)
	if e = s.Transaction(ctx, ss); e != nil {
		return result, e
	}
	return PatchDataAuthorityImportResult{Applied: true, ManifestVersion: c.ManifestVersion, Generation: c.Generation, LedgerDigest: c.LedgerDigest, ManifestDigest: c.ManifestDigest, AppliedAt: now}, nil
}

var _ = strings.TrimSpace
