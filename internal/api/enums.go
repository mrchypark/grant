package api

import (
	"fmt"
	"github.com/mrchypark/grant/internal/grant"
)

func oneOf(field, value string, allowed ...string) error {
	for _, v := range allowed {
		if value == v {
			return nil
		}
	}
	return failure(422, fmt.Sprintf("unknown variant %q for %s", value, field))
}
func validateEnums(v any) error {
	switch x := v.(type) {
	case grant.CreateApplicationInput:
		if e := oneOf("applicationType", x.ApplicationType, "confidential", "public"); e != nil {
			return e
		}
		return oneOf("appType", x.AppType, "web", "mobile", "server", "internal")
	case grant.UpsertAgreementInput:
		return oneOf("status", x.Status, "draft", "active", "expired", "terminated")
	case grant.CreateRoleBindingInput:
		if e := oneOf("subjectType", x.SubjectType, "organization", "group", "user"); e != nil {
			return e
		}
		return oneOf("state", x.State, "active", "disabled")
	case providerActions:
		for _, a := range x.Actions {
			if e := oneOf("riskLevel", a.RiskLevel, "low", "medium", "high", "critical"); e != nil {
				return e
			}
		}
	case grant.PatchDataAuthorityImport:
		for _, m := range x.Memberships {
			if e := oneOf("role", m.Role, "owner", "admin", "member"); e != nil {
				return e
			}
			if e := oneOf("status", m.Status, "active", "disabled"); e != nil {
				return e
			}
		}
		for _, b := range x.PlantBindings {
			if e := oneOf("subjectType", b.SubjectType, "organization", "group", "user"); e != nil {
				return e
			}
			if e := oneOf("state", b.State, "active", "disabled"); e != nil {
				return e
			}
		}
	}
	return nil
}
