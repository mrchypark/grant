package grant

type PatchDataHierarchyEdge struct {
	ParentOrganizationID string `json:"parentOrganizationId"`
	ChildOrganizationID  string `json:"childOrganizationId"`
}
type PatchDataImportedUser struct {
	UserID         string  `json:"userId"`
	ExternalUserID string  `json:"externalUserId"`
	Email          *string `json:"email"`
	DisplayName    string  `json:"displayName"`
	Active         bool    `json:"active"`
}
type PatchDataImportedOrganization struct {
	OrganizationID string `json:"organizationId"`
	Name           string `json:"name"`
	Active         bool   `json:"active"`
}
type PatchDataImportedMembership struct {
	MembershipID   string           `json:"membershipId"`
	OrganizationID string           `json:"organizationId"`
	UserID         string           `json:"userId"`
	Role           MembershipRole   `json:"role"`
	Status         MembershipStatus `json:"status"`
}
type PatchDataImportedPlantBinding struct {
	BindingID             string                 `json:"bindingId"`
	SubjectType           RoleBindingSubjectType `json:"subjectType"`
	SubjectID             string                 `json:"subjectId"`
	SubjectOrganizationID string                 `json:"subjectOrganizationId"`
	PlantID               string                 `json:"plantId"`
	State                 RoleBindingState       `json:"state"`
}
type PatchDataImportedApplicationAssignment struct {
	ApplicationID  string `json:"applicationId"`
	OrganizationID string `json:"organizationId"`
}
type PatchDataAuthorityImport struct {
	ManifestVersion            string                                   `json:"manifestVersion"`
	Generation                 string                                   `json:"generation"`
	LedgerDigest               string                                   `json:"ledgerDigest"`
	ManifestDigest             string                                   `json:"manifestDigest"`
	ExpectedPreviousGeneration *string                                  `json:"expectedPreviousGeneration"`
	Users                      []PatchDataImportedUser                  `json:"users"`
	Organizations              []PatchDataImportedOrganization          `json:"organizations"`
	Memberships                []PatchDataImportedMembership            `json:"memberships"`
	HierarchyEdges             []PatchDataHierarchyEdge                 `json:"hierarchyEdges"`
	PlantBindings              []PatchDataImportedPlantBinding          `json:"plantBindings"`
	ApplicationAssignments     []PatchDataImportedApplicationAssignment `json:"applicationAssignments"`
}
type PatchDataAuthorityImportResult struct {
	Applied         bool   `json:"applied"`
	ManifestVersion string `json:"manifestVersion"`
	Generation      string `json:"generation"`
	LedgerDigest    string `json:"ledgerDigest"`
	ManifestDigest  string `json:"manifestDigest"`
	AppliedAt       string `json:"appliedAt"`
}
