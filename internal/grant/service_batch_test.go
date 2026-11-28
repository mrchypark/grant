package grant

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
)

func testCapabilityCatalog(t *testing.T, generation uint64) (StageHttpCapabilityGeneration, []Action) {
	t.Helper()
	provider := "patch-data"
	mkAction := func(col, op, risk string) Action {
		id := provider + "." + col + "." + op
		return Action{ID: id, ProviderID: provider, Code: col + "." + op, Name: id, RiskLevel: risk, GrantableToApplications: true}
	}
	mkCap := func(id, method, path string, actionID string, resourceType string, extKind string, bodyMode string, queryMode string) HttpCapability {
		maxBytes := uint64(1 << 20)
		if bodyMode == "none" {
			maxBytes = 0
		}
		c := HttpCapability{
			CapabilityID: id, Version: 1, Enabled: true, AccessMode: "authorized",
			Method: method, PathTemplate: path, ActionID: &actionID,
			Resource: &HttpCapabilityResource{ResourceType: resourceType, ID: HttpResourceExtractor{Kind: extKind, Name: "id", Format: "asciiToken"}},
			Query:    HttpQueryPolicy{Mode: queryMode},
			Request:  HttpRequestPolicy{Body: bodyMode, MaxBytes: maxBytes, Authorization: "service-token", ForwardHeaders: []string{}},
			Response: HttpResponsePolicy{MaxBytes: 1 << 20, ForwardHeaders: []string{}, Redirects: "deny"},
		}
		if extKind == "constant" {
			c.Resource.ID = HttpResourceExtractor{Kind: "constant", Value: resourceType}
		}
		var err error
		c.ContractDigest, err = HttpCapabilityContractDigest(c)
		if err != nil {
			t.Fatal(err)
		}
		return c
	}
	actions := []Action{
		mkAction("files", "create", "medium"),
		mkAction("files", "update", "medium"),
		mkAction("files", "delete", "high"),
		mkAction("permissions", "read", "low"),
		mkAction("plant", "read", "low"),
		mkAction("batch", "execute", "high"),
	}
	permGet := mkCap("permissions.get.0", "GET", "/api/collections/permissions/records/{id}", actions[3].ID, "permissions", "pathSegment", "none", "exact")
	permGet.Query = HttpQueryPolicy{Mode: "exact", Parameters: []HttpQueryParameter{{Name: "expand", Format: "text", MaxBytes: 64}}}
	permGet.ResponseAuthorization = &HttpResponseAuthorization{RootKey: "permissions", RecordsPointer: "", ExpansionField: "expand", QueryParameter: "expand", MaxDepth: 2, MaxRecords: 50}
	var err error
	permGet.ContractDigest, err = HttpCapabilityContractDigest(permGet)
	if err != nil {
		t.Fatal(err)
	}
	permList := mkCap("permissions.list.0", "GET", "/api/collections/permissions/records", actions[3].ID, "permissions", "constant", "none", "exact")
	permList.Query = HttpQueryPolicy{Mode: "exact", Parameters: []HttpQueryParameter{{Name: "expand", Format: "text", MaxBytes: 64}, {Name: "filter", Format: "text", MaxBytes: 4096}}}
	permList.RecordScope = &HttpRecordScopePolicy{FilterParameter: "filter", RowKey: "id", Grammar: "fexpr", MaxIDs: 100, MaxFilterBytes: 4096, AllowedFields: []string{"id"}}
	permList.ResponseAuthorization = &HttpResponseAuthorization{RootKey: "permissions", RecordsPointer: "/items", ExpansionField: "expand", QueryParameter: "expand", MaxDepth: 2, MaxRecords: 50}
	permList.ContractDigest, err = HttpCapabilityContractDigest(permList)
	if err != nil {
		t.Fatal(err)
	}
	caps := []HttpCapability{
		mkCap("files.create.0", "POST", "/api/collections/files/records", actions[0].ID, "files", "constant", "json", "none"),
		mkCap("files.update.0", "PATCH", "/api/collections/files/records/{id}", actions[1].ID, "files", "pathSegment", "json", "none"),
		mkCap("files.delete.0", "DELETE", "/api/collections/files/records/{id}", actions[2].ID, "files", "pathSegment", "none", "none"),
		permList,
		permGet,
	}
	policy := &HttpBatchPolicy{
		Encoding: "request-array", RequestsPointer: "/requests", MethodField: "method", URLField: "url", BodyField: "body", HeadersField: "headers",
		MaxOperations:        20,
		AllowedCapabilityIDs: []string{"files.create.0", "files.delete.0", "files.update.0"},
		Upserts:              []HttpBatchUpsert{{Method: "PUT", PathTemplate: "/api/collections/files/records", CreateCapabilityID: "files.create.0", UpdateCapabilityID: "files.update.0", IDPointer: "/id"}},
		Multipart:            &HttpBatchMultipart{PayloadField: "@jsonPayload", FileFieldPatterns: []string{"requests.{index}.{field}", "requests[{index}].{field}"}},
	}
	batchCap := HttpCapability{
		CapabilityID: "patch-data.batch", Version: 1, Enabled: true, AccessMode: "authorized",
		Method: "POST", PathTemplate: "/api/batch", ActionID: &actions[5].ID,
		Resource: &HttpCapabilityResource{ResourceType: "batch", ID: HttpResourceExtractor{Kind: "constant", Value: "patch-data"}},
		Query:    HttpQueryPolicy{Mode: "none"},
		Request:  HttpRequestPolicy{Body: "opaque", MaxBytes: 128 << 20, Authorization: "service-token", ForwardHeaders: []string{"content-type"}, ContentTypes: []string{"application/json", "multipart/form-data"}},
		Response: HttpResponsePolicy{MaxBytes: 128 << 20, ForwardHeaders: []string{"content-type"}, Redirects: "deny", ErrorStatuses: []int{400, 403, 404, 409, 422}},
		Batch:    policy,
	}
	batchCap.ContractDigest, err = HttpCapabilityContractDigest(batchCap)
	if err != nil {
		t.Fatal(err)
	}
	caps = append(caps, batchCap)
	resNodes := []HttpResponseResource{
		{Key: "permissions", ResourceType: "permissions", ActionID: actions[3].ID, IDField: "id", IDFormat: "pocketbaseId", IdentityField: "collectionName", IdentityValue: "permissions", Relations: []HttpResponseRelation{{Field: "plant", TargetKey: "plants"}}},
		{Key: "plants", ResourceType: "plant", ActionID: actions[4].ID, IDField: "id", IDFormat: "pocketbaseId", IdentityField: "collectionName", IdentityValue: "plants"},
	}
	stage := StageHttpCapabilityGeneration{SchemaVersion: 1, Generation: generation, Capabilities: caps, ResponseResources: resNodes}
	stage.CatalogDigest, err = HttpCapabilityCatalogDigest(provider, stage)
	if err != nil {
		t.Fatal(err)
	}
	return stage, actions
}

func batchFixture(t *testing.T) (StageHttpCapabilityGeneration, HttpCapability, map[string]HttpCapability) {
	t.Helper()
	stage, _ := testCapabilityCatalog(t, 1)
	byID := map[string]HttpCapability{}
	for _, cap := range stage.Capabilities {
		byID[cap.CapabilityID] = cap
	}
	return stage, byID["patch-data.batch"], byID
}

func TestBatchWireDigestOmitsAbsentResourceName(t *testing.T) {
	_, _, byID := batchFixture(t)
	children := []HttpBatchChild{batchEvidenceChild(byID["files.create.0"], 0, "")}
	raw, err := json.Marshal(children)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(raw), `"name":null`) {
		t.Fatal("absent name changes PEP map digest")
	}
	var generic []map[string]any
	if err = json.Unmarshal(raw, &generic); err != nil {
		t.Fatal(err)
	}
	canonical, err := canonicalJSON(generic, "")
	if err != nil {
		t.Fatal(err)
	}
	digest, err := HttpBatchChildrenDigest(children)
	if err != nil || digest != sha256Text(canonical) {
		t.Fatal("wire digest", digest, err)
	}
}
func batchEvidenceChild(cap HttpCapability, index uint64, id string) HttpBatchChild {
	body := "{}"
	if cap.Request.Body == "none" {
		body = ""
	}
	path := cap.PathTemplate
	resourceID := cap.Resource.ID.Value
	evidence := HttpCapabilityResourceEvidence{Kind: cap.Resource.ID.Kind, CanonicalValue: resourceID}
	if cap.Resource.ID.Kind == "pathSegment" {
		name := cap.Resource.ID.Name
		resourceID = id
		path = strings.Replace(path, "{"+name+"}", id, 1)
		evidence.Name = &name
		evidence.CanonicalValue = id
	}
	return HttpBatchChild{Index: index, OriginalMethod: cap.Method, OriginalPath: path, CapabilityID: cap.CapabilityID, CapabilityVersion: cap.Version, ContractDigest: cap.ContractDigest, Method: cap.Method, CanonicalPath: path, CanonicalQueryDigest: sha256Text(""), ForwardedHeaderDigest: sha256Text(""), BodyDigest: sha256Text(body), BodyLength: uint64(len(body)), ResourceEvidence: evidence, ResourceType: cap.Resource.ResourceType, ResourceID: resourceID}
}
func batchEvidenceInput(t *testing.T, stage StageHttpCapabilityGeneration, root HttpCapability, children []HttpBatchChild) HttpCapabilityDecisionInput {
	t.Helper()
	digest, err := HttpBatchChildrenDigest(children)
	if err != nil {
		t.Fatal(err)
	}
	return HttpCapabilityDecisionInput{ProviderID: "patch-data", PEPClientID: "batch-pep", CatalogGeneration: stage.Generation, CatalogDigest: stage.CatalogDigest, CapabilityID: root.CapabilityID, CapabilityVersion: root.Version, ContractDigest: root.ContractDigest, Method: root.Method, CanonicalPath: root.PathTemplate, CanonicalQueryDigest: sha256Text(""), ForwardedHeaderDigest: sha256Text(""), BodyDigest: sha256Text("{}"), BodyLength: 2, ProviderCredentialDigest: CredentialDigest("token"), ProviderCredential: HttpCapabilityCredentialEvidence{Subject: "batch-user", Audience: "batch-client"}, ResourceEvidence: HttpCapabilityResourceEvidence{Kind: "constant", CanonicalValue: "patch-data"}, ResourceType: "batch", ResourceID: "patch-data", RequestBinding: strings.Repeat("a", 22), BatchChildren: children, BatchDigest: digest}
}

func TestBatchEvidenceRequiresCompleteUpsertPair(t *testing.T) {
	stage, root, byID := batchFixture(t)
	create := batchEvidenceChild(byID["files.create.0"], 0, "")
	update := batchEvidenceChild(byID["files.update.0"], 0, "aaaaaaaaaaaaaaa")
	create.OriginalMethod = "PUT"
	update.OriginalMethod = "PUT"
	update.OriginalPath = create.OriginalPath
	create.UpsertID = "aaaaaaaaaaaaaaa"
	update.UpsertID = create.UpsertID
	catalog := ActiveHttpCapabilityCatalog{Capabilities: stage.Capabilities}
	input := batchEvidenceInput(t, stage, root, []HttpBatchChild{create, update})
	if checks, err := validateBatchEvidence(root, catalog, input); err != nil || len(checks) != 2 {
		t.Fatal(checks, err)
	}
	for name, change := range map[string]func(*HttpCapabilityDecisionInput){
		"missing-update": func(in *HttpCapabilityDecisionInput) { in.BatchChildren = in.BatchChildren[:1] },
		"wrong-index":    func(in *HttpCapabilityDecisionInput) { in.BatchChildren[0].Index = 1 },
		"wrong-source":   func(in *HttpCapabilityDecisionInput) { in.BatchChildren[1].OriginalPath = "/other" },
		"different-id":   func(in *HttpCapabilityDecisionInput) { in.BatchChildren[1].UpsertID = "bbbbbbbbbbbbbbb" },
		"different-body": func(in *HttpCapabilityDecisionInput) { in.BatchChildren[1].BodyDigest = sha256Text("different") },
		"forged-cap":     func(in *HttpCapabilityDecisionInput) { in.BatchChildren[1].CapabilityID = "files.get.0" },
		"child-query":    func(in *HttpCapabilityDecisionInput) { in.BatchChildren[1].CanonicalQuery = "expand=plant" },
	} {
		t.Run(name, func(t *testing.T) {
			in := input
			in.BatchChildren = append([]HttpBatchChild(nil), input.BatchChildren...)
			change(&in)
			in.BatchDigest, _ = HttpBatchChildrenDigest(in.BatchChildren)
			if _, err := validateBatchEvidence(root, catalog, in); err == nil {
				t.Fatal("accepted invalid batch")
			}
		})
	}
	input.BatchDigest = sha256Text("wrong")
	if _, err := validateBatchEvidence(root, catalog, input); err == nil {
		t.Fatal("unbound child digest")
	}
	create.UpsertID = ""
	input = batchEvidenceInput(t, stage, root, []HttpBatchChild{create})
	if _, err := validateBatchEvidence(root, catalog, input); err != nil {
		t.Fatal("id-less upsert", err)
	}
}

func TestBatchPolicyAndOperationBounds(t *testing.T) {
	stage, root, byID := batchFixture(t)
	children := make([]HttpBatchChild, 20)
	for i := range children {
		children[i] = batchEvidenceChild(byID["files.create.0"], uint64(i), "")
	}
	input := batchEvidenceInput(t, stage, root, children)
	catalog := ActiveHttpCapabilityCatalog{Capabilities: stage.Capabilities}
	if _, err := validateBatchEvidence(root, catalog, input); err != nil {
		t.Fatal("bounded batch rejected", err)
	}
	input.BatchChildren = append(input.BatchChildren, batchEvidenceChild(byID["files.create.0"], 20, ""))
	input.BatchDigest, _ = HttpBatchChildrenDigest(input.BatchChildren)
	if _, err := validateBatchEvidence(root, catalog, input); err == nil {
		t.Fatal("too many operations")
	}
	for name, change := range map[string]func(*HttpCapability){
		"recursive":        func(c *HttpCapability) { c.Batch.AllowedCapabilityIDs = []string{c.CapabilityID} },
		"read-child":       func(c *HttpCapability) { c.Batch.AllowedCapabilityIDs = []string{"files.get.0"} },
		"limit":            func(c *HttpCapability) { c.Batch.MaxOperations = 21 },
		"ambiguous-fields": func(c *HttpCapability) { c.Batch.MethodField = c.Batch.URLField },
	} {
		t.Run(name, func(t *testing.T) {
			copy := root
			policy := *root.Batch
			copy.Batch = &policy
			change(&copy)
			if validateBatchPolicy(copy) == nil && validateBatchReferences(append(append([]HttpCapability(nil), stage.Capabilities[:len(stage.Capabilities)-1]...), copy)) == nil {
				t.Fatal("invalid policy accepted")
			}
		})
	}
}

func TestBatchDecisionRequiresRootAndEveryChild(t *testing.T) {
	testBatchDecisionRequiresRootAndEveryChild(t, false)
}

func TestOrglessBatchDecisionRequiresRootAndEveryChild(t *testing.T) {
	testBatchDecisionRequiresRootAndEveryChild(t, true)
}

func testBatchDecisionRequiresRootAndEveryChild(t *testing.T, orgless bool) {
	ctx := context.WithValue(context.Background(), replicaIdentityKey{}, ExternalIdentity{Provider: "rauthy", ExternalUserID: "batch-user", ClientID: "batch-client", Active: true})
	cfg := Config{Database: DatabaseConfig{DataDir: t.TempDir()}, Auth: AuthConfig{ProviderMode: "rauthy"}, Security: SecurityConfig{HTTPCapabilityPEPClientID: "batch-pep", ManagementApplicationID: "batch-app"}}
	store, err := OpenStore(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	defer store.Close()
	if err = store.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	if _, err = store.BindIdentityToUser(ctx, BindIdentityInput{UserID: "conalog-grant", Provider: "rauthy", ExternalUserID: "batch-user", DisplayName: "Batch user", Active: true}); err != nil {
		t.Fatal(err)
	}
	client := "batch-client"
	app := Application{ID: "batch-app", OrganizationID: "conalog-grant", Slug: "batch-app", Name: "Batch", RauthyClientID: &client, ApplicationType: "confidential", AppType: "server", FirstParty: true, ConsentPolicy: "auto_approve", Active: true, ProvisioningStatus: "ready", CreatedByUserID: "conalog-grant"}
	if err = store.InsertApplicationWithRedirectURI(ctx, app, nil); err != nil {
		t.Fatal(err)
	}
	svc := NewService(cfg, store)
	if _, err = svc.UpsertServiceProviderAdmin(ctx, ServiceProvider{ID: "patch-data", Name: "Patch Data", Active: true}); err != nil {
		t.Fatal(err)
	}
	stage, actions := testCapabilityCatalog(t, 1)
	if orgless {
		optional := map[string]bool{}
		for _, c := range stage.Capabilities {
			if c.Batch != nil {
				optional[c.CapabilityID] = true
				for _, id := range c.Batch.AllowedCapabilityIDs {
					optional[id] = true
				}
			}
		}
		for i := range stage.Capabilities {
			if optional[stage.Capabilities[i].CapabilityID] {
				stage.Capabilities[i].OrganizationContext = "optional"
				stage.Capabilities[i].ContractDigest, err = HttpCapabilityContractDigest(stage.Capabilities[i])
				if err != nil {
					t.Fatal(err)
				}
			}
		}
		stage.CatalogDigest, err = HttpCapabilityCatalogDigest("patch-data", stage)
		if err != nil {
			t.Fatal(err)
		}
	}
	if _, err = svc.ReplaceProviderActionsAdmin(ctx, "patch-data", actions); err != nil {
		t.Fatal(err)
	}
	if _, err = store.UpsertProviderPepClient(ctx, "patch-data", "batch-pep", "conalog-grant", UpsertProviderPepClient{Lifecycle: "active"}); err != nil {
		t.Fatal(err)
	}
	if _, err = store.StageHttpCapabilityGeneration(ctx, "patch-data", stage); err != nil {
		t.Fatal(err)
	}
	if _, err = store.ActivateHttpCapabilityGeneration(ctx, "patch-data", "conalog-grant", ActivateHttpCapabilityGeneration{TargetGeneration: 1, TargetCatalogDigest: stage.CatalogDigest}); err != nil {
		t.Fatal(err)
	}
	if _, err = store.Exec(ctx, "INSERT INTO application_actions(application_id,action_id) SELECT ?,id FROM actions WHERE provider_id='patch-data'", app.ID); err != nil {
		t.Fatal(err)
	}
	if _, err = store.Exec(ctx, "INSERT INTO role_actions(role_id,action_id) SELECT 'role-system-bootstrap-owner',id FROM actions WHERE provider_id='patch-data'"); err != nil {
		t.Fatal(err)
	}
	byID := map[string]HttpCapability{}
	for _, c := range stage.Capabilities {
		byID[c.CapabilityID] = c
	}
	root := byID["patch-data.batch"]
	input := batchEvidenceInput(t, stage, root, []HttpBatchChild{batchEvidenceChild(byID["files.update.0"], 0, "aaaaaaaaaaaaaaa"), batchEvidenceChild(byID["files.delete.0"], 1, "bbbbbbbbbbbbbbb")})
	check := func(want bool) {
		t.Helper()
		org := "conalog-grant"
		if orgless {
			org = ""
		}
		decision, e := svc.DecideHttpCapability(ctx, "token", org, input)
		if e != nil || decision.Allowed != want || decision.BatchDigest != input.BatchDigest {
			t.Fatal("batch decision", want, decision, e)
		}
	}
	check(false)
	for i, pair := range [][2]string{{"batch", "patch-data"}, {"files", "aaaaaaaaaaaaaaa"}, {"files", "bbbbbbbbbbbbbbb"}} {
		if orgless {
			action := *root.ActionID
			if i > 0 {
				action = *byID[input.BatchChildren[i-1].CapabilityID].ActionID
			}
			_, err = svc.SetOrglessUserBindingAdmin(ctx, pair[1], OrglessUserBinding{SubjectID: "conalog-grant", ProviderID: "patch-data", ResourceType: pair[0], ResourceID: pair[1], ActionID: action, State: "active"})
		} else {
			_, err = store.Exec(ctx, "INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,state,created_by_user_id,created_at,updated_at) VALUES(?,'user','conalog-grant','conalog-grant','patch-data',?,?,'role-system-bootstrap-owner','active','conalog-grant','now','now')", pair[1], pair[0], pair[1])
		}
		if err != nil {
			t.Fatal(err)
		}
		check(i == 2)
	}
	if orgless {
		if _, err = store.Exec(ctx, "UPDATE role_bindings SET state='disabled' WHERE id='patch-data'"); err != nil {
			t.Fatal(err)
		}
		check(false)
		if _, err = store.Exec(ctx, "UPDATE role_bindings SET state='active' WHERE id='patch-data'"); err != nil {
			t.Fatal(err)
		}
		check(true)
	}
	if _, err = store.Exec(ctx, "UPDATE role_bindings SET state='revoked' WHERE id='bbbbbbbbbbbbbbb'"); err != nil {
		t.Fatal(err)
	}
	check(false)
	if !orgless {
		t.Run("response-fence", func(t *testing.T) { testResponseDecisionInGrant(t, ctx, svc, stage) })
	}
}
