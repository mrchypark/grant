package grant

import (
	"context"
	"fmt"
	"math"
	"strings"
	"time"
)

func validAuthorityID(v string) bool {
	if v == "" {
		return false
	}
	for i, r := range v {
		if !((r >= 'a' && r <= 'z') || (r >= '0' && r <= '9') || (i > 0 && strings.ContainsRune("._:-", r))) {
			return false
		}
	}
	return true
}
func sortedStrings(v []string) bool {
	for i, s := range v {
		if s == "" || (i > 0 && v[i-1] >= s) {
			return false
		}
	}
	return true
}
func validateSnapshot(v AuthoritySnapshotV2) error {
	if v.SchemaVersion != 2 || v.Generation == 0 || v.Generation > math.MaxInt64 || !validAuthorityID(v.SourceID) || !validDigest(v.SourceLedgerDigest) || !validDigest(v.SourceWatermark) || !validDigest(v.SnapshotDigest) || strictToken(v.SourceRevision, 10000) != nil {
		return fmt.Errorf("invalid snapshot metadata")
	}
	at, e := time.Parse(time.RFC3339Nano, v.ProducedAt)
	if e != nil || !strings.HasSuffix(v.ProducedAt, "Z") {
		return fmt.Errorf("producedAt must be UTC timestamp")
	}
	p := map[string]AuthorityPrincipalV2{}
	o := map[string]AuthorityOrganizationV2{}
	a := map[string]AuthorityApplicationV2{}
	ext := map[string]bool{}
	prev := ""
	for _, x := range v.Principals {
		if !validAuthorityID(x.ID) || x.ID <= prev || strictToken(x.Kind, 1000) != nil || x.ExternalSubject == "" || ext[x.ExternalSubject] {
			return fmt.Errorf("invalid principal")
		}
		prev = x.ID
		ext[x.ExternalSubject] = true
		p[x.ID] = x
	}
	prev = ""
	for _, x := range v.Organizations {
		if !validAuthorityID(x.ID) || x.ID <= prev {
			return fmt.Errorf("invalid organization")
		}
		prev = x.ID
		o[x.ID] = x
	}
	prev = ""
	sem := map[string]bool{}
	for _, x := range v.Memberships {
		pr, ok := p[x.PrincipalID]
		org, found := o[x.OrganizationID]
		key := x.PrincipalID + "\x00" + x.OrganizationID + "\x00" + x.Role + "\x00" + x.PrincipalType
		if !validAuthorityID(x.ID) || x.ID <= prev || !ok || !found || pr.Kind != x.PrincipalType || strictToken(x.Role, 1000) != nil || len(x.SourceRefs) == 0 || !sortedStrings(x.SourceRefs) || sem[key] {
			return fmt.Errorf("invalid membership")
		}
		prev = x.ID
		sem[key] = true
		switch x.Lifecycle {
		case "active", "disabled", "deleted", "expired":
		default:
			return fmt.Errorf("invalid membership lifecycle")
		}
		if x.Lifecycle == "active" && (!pr.Active || !org.Active) {
			return fmt.Errorf("active membership references inactive source")
		}
		if x.ValidUntil != nil {
			t, e := time.Parse(time.RFC3339Nano, *x.ValidUntil)
			if e != nil || !strings.HasSuffix(*x.ValidUntil, "Z") || (x.Lifecycle == "active" && !t.After(at)) {
				return fmt.Errorf("invalid membership expiry")
			}
		}
	}
	prev = ""
	for _, x := range v.Applications {
		_, ok := o[x.OrganizationID]
		if !validAuthorityID(x.ID) || x.ID <= prev || !ok || (x.Lifecycle != "active" && x.Lifecycle != "disabled" && x.Lifecycle != "deleted") {
			return fmt.Errorf("invalid application")
		}
		prev = x.ID
		a[x.ID] = x
	}
	prev = ""
	for _, x := range v.EffectiveBindings {
		appID := ""
		if x.ApplicationScope.ApplicationID != nil {
			appID = *x.ApplicationScope.ApplicationID
		}
		key := strings.Join([]string{x.PrincipalID, x.ApplicationScope.OrganizationID, x.ApplicationScope.Kind, appID, x.ProviderID, x.ActionID, x.ResourceType, x.ResourceID}, "\x00")
		if key <= prev || !p[x.PrincipalID].Active || !o[x.ApplicationScope.OrganizationID].Active || x.Effect != "allow" || strictToken(x.ProviderID, 10000) != nil || strictToken(x.ActionID, 10000) != nil || strictToken(x.ResourceType, 10000) != nil || strictToken(x.ResourceID, 10000) != nil || strictToken(x.Derivation, 10000) != nil || len(x.SourceRuleIDs) == 0 || !sortedStrings(x.SourceRuleIDs) {
			return fmt.Errorf("invalid effective binding")
		}
		for _, rule := range x.SourceRuleIDs {
			if strings.TrimSpace(rule) != rule {
				return fmt.Errorf("source rule IDs must be trimmed")
			}
		}
		prev = key
		switch x.ApplicationScope.Kind {
		case "exact":
			app, ok := a[appID]
			if !ok || app.Lifecycle != "active" || app.OrganizationID != x.ApplicationScope.OrganizationID {
				return fmt.Errorf("invalid exact application scope")
			}
		case "wildcard":
			if x.ApplicationScope.ApplicationID != nil {
				return fmt.Errorf("wildcard scope cannot select application")
			}
		default:
			return fmt.Errorf("invalid application scope")
		}
	}
	digest, e := SnapshotDigest(v)
	if e != nil {
		return e
	}
	if digest != v.SnapshotDigest {
		return fmt.Errorf("snapshotDigest does not match canonical snapshot")
	}
	return nil
}
func (s *Store) ImportAuthoritySnapshotV2(ctx context.Context, v AuthoritySnapshotV2) (AuthoritySnapshotV2Result, error) {
	var out AuthoritySnapshotV2Result
	if e := validateSnapshot(v); e != nil {
		return out, e
	}
	canonical, e := canonicalJSON(v, "")
	if e != nil {
		return out, e
	}
	r, e := s.Query(ctx, ReadStrong, "SELECT applied_at,prior_generation,canonical_json=? AS canonical_match FROM authority_snapshots_v2 WHERE source_id=? AND generation=?", canonical, v.SourceID, v.Generation)
	if e != nil {
		return out, e
	}
	applied := len(r) == 0
	var prev *uint64
	now := nowString()
	if !applied {
		if rowInt(r[0], "canonical_match") != 1 {
			return out, fmt.Errorf("authority v2 generation conflict")
		}
		now = rowString(r[0], "applied_at")
		if r[0]["prior_generation"] != nil {
			n := uint64(rowInt(r[0], "prior_generation"))
			prev = &n
		}
	} else {
		if v.Generation > 1 {
			n := v.Generation - 1
			prev = &n
		}
		stageID := strings.TrimPrefix(digestBytes([]byte(canonical)), "sha256:")
		if e = s.stageText(ctx, stageID, "v2", canonical); e != nil {
			return out, e
		}
		_, e = s.Execute(ctx, requestID(), Statement{SQL: `INSERT INTO authority_snapshots_v2(source_id,generation,prior_generation,snapshot_digest,canonical_json,source_revision,source_ledger_digest,source_watermark,produced_at,principal_count,organization_count,membership_count,application_count,effective_binding_count,applied_at)
 SELECT ?,?,?,?,` + stagedTextSQL() + `,?,?,?,?,?,?,?,?,?,? WHERE ?=COALESCE((SELECT MAX(generation) FROM authority_snapshots_v2 WHERE source_id=?),0)+1`, Args: []any{v.SourceID, v.Generation, prev, v.SnapshotDigest, stageID, "v2", v.SourceRevision, v.SourceLedgerDigest, v.SourceWatermark, v.ProducedAt, len(v.Principals), len(v.Organizations), len(v.Memberships), len(v.Applications), len(v.EffectiveBindings), now, v.Generation, v.SourceID}, ExpectedRowsAffected: requireOne()}, Statement{SQL: "DELETE FROM grant_import_payloads WHERE import_id=? AND kind='v2'", Args: []any{stageID}})
		if e != nil {
			// A concurrent identical import may have won the same generation.
			winner, readErr := s.Query(ctx, ReadStrong, "SELECT canonical_json=? AS canonical_match FROM authority_snapshots_v2 WHERE source_id=? AND generation=?", canonical, v.SourceID, v.Generation)
			if readErr == nil && len(winner) == 1 && rowInt(winner[0], "canonical_match") == 1 {
				return s.ImportAuthoritySnapshotV2(ctx, v)
			}
			return out, e
		}
	}
	return AuthoritySnapshotV2Result{Applied: applied, SchemaVersion: 2, SourceID: v.SourceID, Generation: v.Generation, PriorGeneration: prev, SnapshotDigest: v.SnapshotDigest, SourceRevision: v.SourceRevision, PrincipalCount: uint64(len(v.Principals)), OrganizationCount: uint64(len(v.Organizations)), MembershipCount: uint64(len(v.Memberships)), ApplicationCount: uint64(len(v.Applications)), EffectiveBindingCount: uint64(len(v.EffectiveBindings)), AppliedAt: now}, nil
}
