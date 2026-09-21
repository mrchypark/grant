package grant

type CreateApplicationInput struct {
	ID              string             `json:"id"`
	Slug            string             `json:"slug"`
	Name            string             `json:"name"`
	Description     *string            `json:"description"`
	ApplicationType ApplicationType    `json:"applicationType"`
	AppType         ApplicationAppType `json:"appType"`
	Active          bool               `json:"active"`
	RedirectURI     *string            `json:"redirectUri"`
}

type ApplicationRead struct {
	Application Application `json:"application"`
	RedirectURI *string     `json:"redirectUri"`
}

type UpsertGroupInput struct {
	ID          string  `json:"id"`
	Name        string  `json:"name"`
	Description *string `json:"description"`
}

type UpsertAgreementInput struct {
	ID                    string          `json:"id"`
	AgreementType         AgreementType   `json:"agreementType"`
	GrantorOrganizationID string          `json:"grantorOrganizationId"`
	GranteeOrganizationID string          `json:"granteeOrganizationId"`
	Status                AgreementStatus `json:"status"`
	Title                 string          `json:"title"`
	ReferenceNo           *string         `json:"referenceNo"`
	StartsAt              *string         `json:"startsAt"`
	EndsAt                *string         `json:"endsAt"`
}

type CreateApplicationGrantInput struct {
	ID            string   `json:"id"`
	UserID        string   `json:"userId"`
	ApplicationID string   `json:"applicationId"`
	ActionIDs     []string `json:"actionIds"`
}

type CreateRoleInput struct {
	ID          string  `json:"id"`
	Code        string  `json:"code"`
	Name        string  `json:"name"`
	Description *string `json:"description"`
}

type CreateRoleBindingInput struct {
	ID                    string                 `json:"id"`
	SubjectType           RoleBindingSubjectType `json:"subjectType"`
	SubjectID             string                 `json:"subjectId"`
	SubjectOrganizationID string                 `json:"subjectOrganizationId"`
	ProviderID            string                 `json:"providerId"`
	ResourceType          string                 `json:"resourceType"`
	ResourceID            string                 `json:"resourceId"`
	RoleID                string                 `json:"roleId"`
	AgreementID           *string                `json:"agreementId"`
	State                 RoleBindingState       `json:"state"`
}

type AuthorizationSubject struct {
	SubjectType    string `json:"subjectType"`
	ID             string `json:"id"`
	OrganizationID string `json:"organizationId"`
}

type AuthorizationResource struct {
	ProviderID   string `json:"providerId"`
	ResourceType string `json:"resourceType"`
	ID           string `json:"id"`
}

type AuthorizationCheck struct {
	Subject  AuthorizationSubject  `json:"subject"`
	Action   string                `json:"action"`
	Resource AuthorizationResource `json:"resource"`
}

type ApplicationAuthorizationCheck struct {
	Action   string                `json:"action"`
	Resource AuthorizationResource `json:"resource"`
}

type AuthorizationDecision struct {
	Allowed bool   `json:"allowed"`
	Reason  string `json:"reason"`
}

type HttpCapabilityCredentialEvidence struct {
	Subject  string `json:"subject"`
	Audience string `json:"audience"`
}

type HttpCapabilityResourceEvidence struct {
	Kind           string  `json:"kind"`
	Name           *string `json:"name,omitempty"`
	CanonicalValue string  `json:"canonicalValue"`
}

type HttpCapabilityDecisionInput struct {
	ProviderID               string                           `json:"providerId"`
	PEPClientID              string                           `json:"pepClientId"`
	CatalogGeneration        uint64                           `json:"catalogGeneration"`
	CatalogDigest            string                           `json:"catalogDigest"`
	CapabilityID             string                           `json:"capabilityId"`
	CapabilityVersion        uint64                           `json:"capabilityVersion"`
	ContractDigest           string                           `json:"contractDigest"`
	Method                   string                           `json:"method"`
	CanonicalPath            string                           `json:"canonicalPath"`
	CanonicalQuery           string                           `json:"canonicalQuery"`
	CanonicalQueryDigest     string                           `json:"canonicalQueryDigest"`
	ForwardedHeaderDigest    string                           `json:"forwardedHeaderDigest"`
	BodyDigest               string                           `json:"bodyDigest"`
	BodyLength               uint64                           `json:"bodyLength"`
	ProviderCredentialDigest string                           `json:"providerCredentialDigest"`
	ProviderCredential       HttpCapabilityCredentialEvidence `json:"providerCredential"`
	ResourceEvidence         HttpCapabilityResourceEvidence   `json:"resourceEvidence"`
	ResourceType             string                           `json:"resourceType"`
	ResourceID               string                           `json:"resourceId"`
	RequestBinding           string                           `json:"requestBinding"`
	BatchChildren            []HttpBatchChild                 `json:"batchChildren,omitempty"`
	BatchDigest              string                           `json:"batchDigest,omitempty"`
	ResponseDigest           string                           `json:"responseDigest,omitempty"`
	ResponseLength           uint64                           `json:"responseLength,omitempty"`
	ResponseReferencesDigest string                           `json:"responseReferencesDigest,omitempty"`
	ResponseReferences       []HttpResponseReference          `json:"responseReferences,omitempty"`
}

type HttpResponseReference struct {
	ResourceKey   string  `json:"resourceKey"`
	ID            string  `json:"id"`
	ParentIndex   *uint64 `json:"parentIndex,omitempty"`
	RelationField string  `json:"relationField,omitempty"`
}

// Child bodies represent semantic JSON extracted by the trusted PEP. Their
// digest is not a claim about the provider's synthesized internal HTTP bytes.
type HttpBatchChild struct {
	Index                 uint64                         `json:"index"`
	OriginalMethod        string                         `json:"originalMethod"`
	OriginalPath          string                         `json:"originalPath"`
	UpsertID              string                         `json:"upsertId,omitempty"`
	CapabilityID          string                         `json:"capabilityId"`
	CapabilityVersion     uint64                         `json:"capabilityVersion"`
	ContractDigest        string                         `json:"contractDigest"`
	Method                string                         `json:"method"`
	CanonicalPath         string                         `json:"canonicalPath"`
	CanonicalQuery        string                         `json:"canonicalQuery"`
	CanonicalQueryDigest  string                         `json:"canonicalQueryDigest"`
	ForwardedHeaderDigest string                         `json:"forwardedHeaderDigest"`
	BodyDigest            string                         `json:"bodyDigest"`
	BodyLength            uint64                         `json:"bodyLength"`
	ResourceEvidence      HttpCapabilityResourceEvidence `json:"resourceEvidence"`
	ResourceType          string                         `json:"resourceType"`
	ResourceID            string                         `json:"resourceId"`
}

type HttpCapabilityDecision struct {
	Allowed                  bool             `json:"allowed"`
	Reason                   string           `json:"reason"`
	ProviderID               string           `json:"providerId"`
	PEPClientID              string           `json:"pepClientId"`
	CatalogGeneration        uint64           `json:"catalogGeneration"`
	CatalogDigest            string           `json:"catalogDigest"`
	CapabilityID             string           `json:"capabilityId"`
	CapabilityVersion        uint64           `json:"capabilityVersion"`
	ContractDigest           string           `json:"contractDigest"`
	ActionID                 string           `json:"actionId"`
	ResourceType             string           `json:"resourceType"`
	ResourceID               string           `json:"resourceId"`
	Method                   string           `json:"method"`
	CanonicalPath            string           `json:"canonicalPath"`
	CanonicalQueryDigest     string           `json:"canonicalQueryDigest"`
	ForwardedHeaderDigest    string           `json:"forwardedHeaderDigest"`
	BodyDigest               string           `json:"bodyDigest"`
	BodyLength               uint64           `json:"bodyLength"`
	ProviderCredentialDigest string           `json:"providerCredentialDigest"`
	ProviderSubject          string           `json:"providerSubject"`
	ProviderAudience         string           `json:"providerAudience"`
	RequestBinding           string           `json:"requestBinding"`
	PrincipalDigest          string           `json:"principalDigest"`
	IssuedAt                 string           `json:"issuedAt"`
	ExpiresAt                string           `json:"expiresAt"`
	RecordScope              *HttpRecordScope `json:"recordScope,omitempty"`
	BatchDigest              string           `json:"batchDigest,omitempty"`
	ResponseDigest           string           `json:"responseDigest,omitempty"`
	ResponseLength           uint64           `json:"responseLength,omitempty"`
	ResponseReferencesDigest string           `json:"responseReferencesDigest,omitempty"`
}
type HttpRecordScope struct {
	IDs []string `json:"ids"`
}

type BackupInfo struct {
	Name         string  `json:"name"`
	LastModified int64   `json:"lastModified"`
	Size         *uint64 `json:"size"`
	Storage      string  `json:"storage"`
}

type CurrentUserView struct {
	Issuer               string         `json:"issuer,omitempty"`
	CollectionID         string         `json:"collectionId,omitempty"`
	Provider             string         `json:"provider"`
	ExternalUserID       string         `json:"externalUserId"`
	User                 User           `json:"user"`
	Organizations        []Organization `json:"organizations"`
	ActiveOrganizationID *string        `json:"activeOrganizationId"`
}

type MaterializedUserView struct {
	UserID           string `json:"userId"`
	OrganizationID   string `json:"organizationId"`
	MembershipStatus string `json:"membershipStatus"`
}

type UserActionCheck struct {
	UserID, OrganizationID, ProviderID, ResourceType, ResourceID, ActionID string
	ApplicationID                                                          *string
	Consistency                                                            ReadConsistency
}
type UserActionDecision struct {
	Allowed, RBACAllowed, TrustedApplication bool
	ApplicationAllowed                       bool `json:"-"`
}
type AuthenticatedPrincipal struct{ UserID, OrganizationID string }
type ExternalIdentity struct {
	Provider, ExternalUserID, ClientID string
	Issuer, CollectionID               string
	Email                              *string
	DisplayName                        string
	Active                             bool
}
