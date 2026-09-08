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
type AuthoritySnapshotV2 struct {
	SchemaVersion      uint64                    `json:"schemaVersion"`
	SourceID           string                    `json:"sourceId"`
	Generation         uint64                    `json:"generation"`
	SourceRevision     string                    `json:"sourceRevision"`
	SourceLedgerDigest string                    `json:"sourceLedgerDigest"`
	SourceWatermark    string                    `json:"sourceWatermark"`
	ProducedAt         string                    `json:"producedAt"`
	Principals         []AuthorityPrincipalV2    `json:"principals"`
	Organizations      []AuthorityOrganizationV2 `json:"organizations"`
	Memberships        []AuthorityMembershipV2   `json:"memberships"`
	Applications       []AuthorityApplicationV2  `json:"applications"`
	EffectiveBindings  []EffectiveBindingV2      `json:"effectiveBindings"`
	SnapshotDigest     string                    `json:"snapshotDigest"`
}
type AuthorityPrincipalV2 struct {
	ID              string `json:"id"`
	Kind            string `json:"kind"`
	Active          bool   `json:"active"`
	ExternalSubject string `json:"externalSubject"`
}
type AuthorityOrganizationV2 struct {
	ID     string `json:"id"`
	Active bool   `json:"active"`
}
type AuthorityMembershipV2 struct {
	ID             string   `json:"id"`
	PrincipalID    string   `json:"principalId"`
	OrganizationID string   `json:"organizationId"`
	Role           string   `json:"role"`
	PrincipalType  string   `json:"principalType"`
	Lifecycle      string   `json:"lifecycle"`
	ValidUntil     *string  `json:"validUntil"`
	SourceRefs     []string `json:"sourceRefs"`
}
type AuthorityApplicationV2 struct {
	ID             string `json:"id"`
	OrganizationID string `json:"organizationId"`
	Lifecycle      string `json:"lifecycle"`
}
type ApplicationScopeV2 struct {
	Kind           string  `json:"kind"`
	ApplicationID  *string `json:"applicationId"`
	OrganizationID string  `json:"organizationId"`
}
type EffectiveBindingV2 struct {
	PrincipalID      string             `json:"principalId"`
	ApplicationScope ApplicationScopeV2 `json:"applicationScope"`
	ProviderID       string             `json:"providerId"`
	ActionID         string             `json:"actionId"`
	ResourceType     string             `json:"resourceType"`
	ResourceID       string             `json:"resourceId"`
	Effect           string             `json:"effect"`
	SourceRuleIDs    []string           `json:"sourceRuleIds"`
	Derivation       string             `json:"derivation"`
}
type AuthoritySnapshotV2Result struct {
	Applied               bool    `json:"applied"`
	SchemaVersion         uint64  `json:"schemaVersion"`
	SourceID              string  `json:"sourceId"`
	Generation            uint64  `json:"generation"`
	PriorGeneration       *uint64 `json:"priorGeneration"`
	SnapshotDigest        string  `json:"snapshotDigest"`
	SourceRevision        string  `json:"sourceRevision"`
	PrincipalCount        uint64  `json:"principalCount"`
	OrganizationCount     uint64  `json:"organizationCount"`
	MembershipCount       uint64  `json:"membershipCount"`
	ApplicationCount      uint64  `json:"applicationCount"`
	EffectiveBindingCount uint64  `json:"effectiveBindingCount"`
	AppliedAt             string  `json:"appliedAt"`
}
