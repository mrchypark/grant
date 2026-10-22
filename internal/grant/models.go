package grant

type HealthStatus struct {
	Status                 string `json:"status"`
	ReadReady              bool   `json:"readReady"`
	WriteReady             bool   `json:"writeReady"`
	RuntimeMode            string `json:"runtimeMode"`
	DefaultReadConsistency string `json:"defaultReadConsistency"`
}

type Organization struct {
	ID            string  `json:"id"`
	PartyID       *string `json:"partyId,omitempty"`
	BusinessNo    *string `json:"businessNo,omitempty"`
	Name          string  `json:"name"`
	InactivatedAt *string `json:"inactivatedAt,omitempty"`
}

type OrganizationPartyLink struct {
	OrganizationID string                        `json:"organizationId"`
	PartyID        string                        `json:"partyId"`
	RelationType   OrganizationPartyRelationType `json:"relationType"`
}

type OrganizationPartyRelationType = string

const (
	OrganizationPartyRelationTypeLegal    = "legal"
	OrganizationPartyRelationTypeBilling  = "billing"
	OrganizationPartyRelationTypeContract = "contract"
	OrganizationPartyRelationTypeOwner    = "owner"
)

type OrganizationMembership struct {
	ID             string           `json:"id"`
	OrganizationID string           `json:"organizationId"`
	UserID         string           `json:"userId"`
	MembershipRole MembershipRole   `json:"membershipRole"`
	Status         MembershipStatus `json:"status"`
}

type MembershipRole = string

const (
	MembershipRoleOwner  = "owner"
	MembershipRoleAdmin  = "admin"
	MembershipRoleMember = "member"
)

type MembershipStatus = string

const (
	MembershipStatusActive   = "active"
	MembershipStatusDisabled = "disabled"
)

type Group struct {
	ID             string  `json:"id"`
	OrganizationID string  `json:"organizationId"`
	Name           string  `json:"name"`
	Description    *string `json:"description,omitempty"`
}

type GroupMember struct {
	GroupID                  string `json:"groupId"`
	OrganizationMembershipID string `json:"organizationMembershipId"`
	UserID                   string `json:"userId"`
}

type Agreement struct {
	ID                    string          `json:"id"`
	AgreementType         AgreementType   `json:"agreementType"`
	GrantorOrganizationID string          `json:"grantorOrganizationId"`
	GranteeOrganizationID string          `json:"granteeOrganizationId"`
	Status                AgreementStatus `json:"status"`
	Title                 string          `json:"title"`
	ReferenceNo           *string         `json:"referenceNo,omitempty"`
	StartsAt              *string         `json:"startsAt,omitempty"`
	EndsAt                *string         `json:"endsAt,omitempty"`
}

type AgreementType = string

const (
	AgreementTypeDelegation  = "delegation"
	AgreementTypeEpc         = "epc"
	AgreementTypeOm          = "om"
	AgreementTypeDistributor = "distributor"
	AgreementTypeDealer      = "dealer"
)

type AgreementStatus = string

const (
	AgreementStatusDraft      = "draft"
	AgreementStatusActive     = "active"
	AgreementStatusExpired    = "expired"
	AgreementStatusTerminated = "terminated"
)

type Action struct {
	ID                      string          `json:"id"`
	ProviderID              string          `json:"providerId"`
	Code                    string          `json:"code"`
	Name                    string          `json:"name"`
	Description             *string         `json:"description,omitempty"`
	RiskLevel               ActionRiskLevel `json:"riskLevel"`
	VisibilityGranting      bool            `json:"visibilityGranting"`
	GrantableToApplications bool            `json:"grantableToApplications"`
	ConsentLabel            *string         `json:"consentLabel,omitempty"`
	ConsentDescription      *string         `json:"consentDescription,omitempty"`
}

type ActionRiskLevel = string

const (
	ActionRiskLevelLow      = "low"
	ActionRiskLevelMedium   = "medium"
	ActionRiskLevelHigh     = "high"
	ActionRiskLevelCritical = "critical"
)

type FieldViewMode = string

const (
	FieldViewModeNone       = "none"
	FieldViewModeAnonymized = "anonymized"
	FieldViewModeReal       = "real"
)

type Application struct {
	ID                 string                        `json:"id"`
	OrganizationID     string                        `json:"organizationId"`
	Slug               string                        `json:"slug"`
	Name               string                        `json:"name"`
	Description        *string                       `json:"description,omitempty"`
	ApplicationType    ApplicationType               `json:"applicationType"`
	AppType            ApplicationAppType            `json:"appType"`
	FirstParty         bool                          `json:"firstParty"`
	ConsentPolicy      ApplicationConsentPolicy      `json:"consentPolicy"`
	Active             bool                          `json:"active"`
	ProvisioningStatus ApplicationProvisioningStatus `json:"provisioningStatus"`
	RauthyClientID     *string                       `json:"rauthyClientId,omitempty"`
	CreatedByUserID    string                        `json:"createdByUserId"`
}

type ApplicationProvisioningStatus = string

const (
	ApplicationProvisioningStatusUnprovisioned = "unprovisioned"
	ApplicationProvisioningStatusReady         = "ready"
)

type ApplicationRedirectUri struct {
	ApplicationID string `json:"applicationId"`
	RedirectURI   string `json:"redirectUri"`
	CreatedAt     string `json:"createdAt"`
}

type ApplicationType = string

const (
	ApplicationTypeConfidential = "confidential"
	ApplicationTypePublic       = "public"
)

type ApplicationAppType = string

const (
	ApplicationAppTypeWeb      = "web"
	ApplicationAppTypeMobile   = "mobile"
	ApplicationAppTypeServer   = "server"
	ApplicationAppTypeInternal = "internal"
)

type ApplicationConsentPolicy = string

const (
	ApplicationConsentPolicyExplicit    = "explicit"
	ApplicationConsentPolicyAutoApprove = "auto_approve"
)

type ApplicationAction struct {
	ApplicationID string `json:"applicationId"`
	ActionID      string `json:"actionId"`
}

type ApplicationGrant struct {
	ID              string                 `json:"id"`
	UserID          string                 `json:"userId"`
	ApplicationID   string                 `json:"applicationId"`
	OrganizationID  string                 `json:"organizationId"`
	Status          ApplicationGrantStatus `json:"status"`
	ConsentedAt     string                 `json:"consentedAt"`
	RevokedAt       *string                `json:"revokedAt,omitempty"`
	RevokedByUserID *string                `json:"revokedByUserId,omitempty"`
	CreatedAt       string                 `json:"createdAt"`
	UpdatedAt       string                 `json:"updatedAt"`
}

type ApplicationGrantStatus = string

const (
	ApplicationGrantStatusActive  = "active"
	ApplicationGrantStatusRevoked = "revoked"
	ApplicationGrantStatusExpired = "expired"
)

type ApplicationGrantAction struct {
	ApplicationGrantID string `json:"applicationGrantId"`
	ActionID           string `json:"actionId"`
}

type Role struct {
	ID             string  `json:"id"`
	OrganizationID *string `json:"organizationId,omitempty"`
	Code           string  `json:"code"`
	Name           string  `json:"name"`
	Description    *string `json:"description,omitempty"`
}

type RoleBinding struct {
	ID                    string                 `json:"id"`
	SubjectType           RoleBindingSubjectType `json:"subjectType"`
	SubjectID             string                 `json:"subjectId"`
	SubjectOrganizationID string                 `json:"subjectOrganizationId"`
	ProviderID            string                 `json:"providerId"`
	ResourceType          string                 `json:"resourceType"`
	ResourceID            string                 `json:"resourceId"`
	RoleID                string                 `json:"roleId"`
	AgreementID           *string                `json:"agreementId,omitempty"`
	State                 RoleBindingState       `json:"state"`
	CreatedByUserID       string                 `json:"createdByUserId"`
	CreatedAt             string                 `json:"createdAt"`
	UpdatedAt             string                 `json:"updatedAt"`
}

type RoleBindingSubjectType = string

const (
	RoleBindingSubjectTypeOrganization = "organization"
	RoleBindingSubjectTypeGroup        = "group"
	RoleBindingSubjectTypeUser         = "user"
)

type ServiceProvider struct {
	ID          string  `json:"id"`
	Name        string  `json:"name"`
	Description *string `json:"description,omitempty"`
	Active      bool    `json:"active"`
}

type RoleBindingState = string

const (
	RoleBindingStateActive   = "active"
	RoleBindingStateDisabled = "disabled"
)

type User struct {
	ID          string  `json:"id"`
	Email       *string `json:"email,omitempty"`
	DisplayName string  `json:"displayName"`
	Active      bool    `json:"active"`
}

type Party struct {
	ID         string `json:"id"`
	BusinessNo string `json:"businessNo"`
	LegalName  string `json:"legalName"`
	Active     bool   `json:"active"`
}
