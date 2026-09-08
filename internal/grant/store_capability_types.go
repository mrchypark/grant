package grant

type HttpCapabilityResource struct {
	ResourceType string                `json:"type"`
	ID           HttpResourceExtractor `json:"id"`
}
type HttpCapability struct {
	CapabilityID   string                   `json:"capabilityId"`
	Version        uint64                   `json:"version"`
	Enabled        bool                     `json:"enabled"`
	AccessMode     HttpCapabilityAccessMode `json:"accessMode"`
	Method         string                   `json:"method"`
	PathTemplate   string                   `json:"pathTemplate"`
	ActionID       *string                  `json:"actionId"`
	Resource       *HttpCapabilityResource  `json:"resource"`
	Query          HttpQueryPolicy          `json:"query"`
	Request        HttpRequestPolicy        `json:"request"`
	Response       HttpResponsePolicy       `json:"response"`
	ContractDigest string                   `json:"contractDigest"`
	PathParameters []HttpPathParameter      `json:"pathParameters,omitempty"`
	RecordScope    *HttpRecordScopePolicy   `json:"recordScope,omitempty"`
}
type HttpRecordScopePolicy struct {
	FilterParameter string                `json:"filterParameter"`
	RowKey          string                `json:"rowKey"`
	Grammar         string                `json:"grammar"`
	MaxIDs          uint64                `json:"maxIds"`
	MaxFilterBytes  uint64                `json:"maxFilterBytes"`
	AllowedFields   []string              `json:"allowedFields"`
	SortParameter   string                `json:"sortParameter,omitempty"`
	Pagination      *HttpPaginationPolicy `json:"pagination,omitempty"`
}
type HttpPaginationPolicy struct {
	PageParameter      string `json:"pageParameter,omitempty"`
	PageSizeParameter  string `json:"pageSizeParameter,omitempty"`
	SkipTotalParameter string `json:"skipTotalParameter,omitempty"`
}
type HttpPathParameter struct {
	Name     string `json:"name"`
	Format   string `json:"format"`
	MaxBytes uint64 `json:"maxBytes"`
}
type HttpQueryParameter struct {
	Name     string `json:"name"`
	Format   string `json:"format"`
	Required bool   `json:"required"`
	MaxBytes uint64 `json:"maxBytes,omitempty"`
	Repeated bool   `json:"repeated,omitempty"`
}
type HttpRequestPolicy struct {
	Body           HttpRequestBodyMode `json:"body"`
	MaxBytes       uint64              `json:"maxBytes"`
	ForwardHeaders []string            `json:"forwardHeaders"`
	Authorization  string              `json:"authorization,omitempty"`
	ContentTypes   []string            `json:"contentTypes,omitempty"`
}
type HttpResponsePolicy struct {
	MaxBytes       uint64             `json:"maxBytes"`
	ForwardHeaders []string           `json:"forwardHeaders"`
	Redirects      HttpRedirectPolicy `json:"redirects"`
	ErrorStatuses  []int              `json:"errorStatuses,omitempty"`
}
type StageHttpCapabilityGeneration struct {
	SchemaVersion            uint64           `json:"schemaVersion"`
	Generation               uint64           `json:"generation"`
	ExpectedLatestGeneration *uint64          `json:"expectedLatestGeneration"`
	Capabilities             []HttpCapability `json:"capabilities"`
	CatalogDigest            string           `json:"catalogDigest"`
}
type StagedHttpCapabilityGeneration struct {
	Staged        bool   `json:"staged"`
	ProviderID    string `json:"providerId"`
	Generation    uint64 `json:"generation"`
	CatalogDigest string `json:"catalogDigest"`
	TotalCount    uint64 `json:"totalCount"`
	EnabledCount  uint64 `json:"enabledCount"`
	DisabledCount uint64 `json:"disabledCount"`
}
type ActivateHttpCapabilityGeneration struct {
	ExpectedGeneration    *uint64 `json:"expectedGeneration"`
	ExpectedCatalogDigest *string `json:"expectedCatalogDigest"`
	TargetGeneration      uint64  `json:"targetGeneration"`
	TargetCatalogDigest   string  `json:"targetCatalogDigest"`
}
type HttpCapabilityActivation struct {
	ProviderID          string  `json:"providerId"`
	ActivationReceiptID string  `json:"activationReceiptId"`
	ActivationSequence  uint64  `json:"activationSequence"`
	FromGeneration      *uint64 `json:"fromGeneration"`
	ToGeneration        uint64  `json:"toGeneration"`
	CatalogDigest       string  `json:"catalogDigest"`
	ActivatedAt         string  `json:"activatedAt"`
	ActivatedBy         string  `json:"activatedBy"`
}
type UpsertProviderPepClient struct {
	ApplicationID string                     `json:"applicationId"`
	Lifecycle     ProviderPepClientLifecycle `json:"lifecycle"`
	ValidUntil    *string                    `json:"validUntil"`
}
type ProviderPepClient struct {
	PepClientID   string  `json:"pepClientId"`
	ApplicationID string  `json:"applicationId"`
	ProviderID    string  `json:"providerId"`
	Lifecycle     string  `json:"lifecycle"`
	ValidUntil    *string `json:"validUntil"`
	CreatedAt     string  `json:"createdAt"`
	UpdatedAt     string  `json:"updatedAt"`
}
type ActiveHttpCapabilityCatalog struct {
	SchemaVersion uint64           `json:"schemaVersion"`
	ProviderID    string           `json:"providerId"`
	PepClientID   string           `json:"pepClientId"`
	Generation    uint64           `json:"generation"`
	CatalogDigest string           `json:"catalogDigest"`
	Capabilities  []HttpCapability `json:"capabilities"`
	IssuedAt      string           `json:"issuedAt"`
	ExpiresAt     string           `json:"expiresAt"`
}
type HttpCapabilityAccessMode = string
type HttpRequestBodyMode = string
type HttpRedirectPolicy = string
type ProviderPepClientLifecycle = string
type HttpResourceExtractor struct {
	Kind    string `json:"kind"`
	Name    string `json:"name,omitempty"`
	Format  string `json:"format,omitempty"`
	Pointer string `json:"pointer,omitempty"`
	Value   string `json:"value,omitempty"`
}
type HttpQueryPolicy struct {
	Mode       string               `json:"mode"`
	Parameters []HttpQueryParameter `json:"parameters,omitempty"`
}
