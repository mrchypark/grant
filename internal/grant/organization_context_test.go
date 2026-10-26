package grant

import (
	"encoding/json"
	"os"
	"strings"
	"testing"
)

func organizationContextVector(t *testing.T) HttpCapability {
	t.Helper()
	var vectors []struct {
		Generation canonicalGeneration `json:"generation"`
	}
	data, err := os.ReadFile("testdata/organization-context-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	if err = json.Unmarshal(data, &vectors); err != nil {
		t.Fatal(err)
	}
	return vectors[0].Generation.Capabilities[0]
}

func TestOrganizationContextIndependentDigestVectors(t *testing.T) {
	var vectors []struct {
		Policy        string              `json:"policy"`
		Generation    canonicalGeneration `json:"generation"`
		CatalogDigest string              `json:"catalogDigest"`
	}
	data, err := os.ReadFile("testdata/organization-context-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	if err = json.Unmarshal(data, &vectors); err != nil {
		t.Fatal(err)
	}
	seen := map[string]bool{}
	for _, v := range vectors {
		t.Run(v.Policy, func(t *testing.T) {
			c := v.Generation.Capabilities[0]
			got, err := HttpCapabilityContractDigest(c)
			if err != nil || got != c.ContractDigest {
				t.Fatalf("contract %s != %s (%v)", got, c.ContractDigest, err)
			}
			stage := StageHttpCapabilityGeneration{SchemaVersion: v.Generation.SchemaVersion, Generation: v.Generation.Generation, Capabilities: v.Generation.Capabilities}
			got, err = HttpCapabilityCatalogDigest(v.Generation.ProviderID, stage)
			if err != nil || got != v.CatalogDigest {
				t.Fatalf("generation %s != %s (%v)", got, v.CatalogDigest, err)
			}
			if err = validateGeneration(v.Generation.ProviderID, StageHttpCapabilityGeneration{SchemaVersion: 1, Generation: 1, Capabilities: stage.Capabilities, CatalogDigest: v.CatalogDigest}); err != nil {
				t.Fatal(err)
			}
			if seen[got] {
				t.Fatal("explicit policy collapsed to omission")
			}
			seen[got] = true
			encoded, _ := json.Marshal(c)
			if v.Policy == "omitted" {
				if strings.Contains(string(encoded), "organizationContext") {
					t.Fatal("omission normalized")
				}
			} else if !strings.HasSuffix(string(encoded), `"organizationContext":"`+v.Policy+`"}`) {
				t.Fatal("policy must serialize last", string(encoded))
			}
		})
	}
}

func TestOrganizationContextStrictWireAndTypedValidation(t *testing.T) {
	base := organizationContextVector(t)
	raw, err := json.Marshal(base)
	if err != nil {
		t.Fatal(err)
	}
	prefix := strings.TrimSuffix(string(raw), "}")
	for _, wire := range []string{`null`, `""`, `"none"`, `"OPTIONAL"`, `"unknown"`, `true`, `1`, `[]`, `{}`} {
		for _, enabled := range []bool{true, false} {
			text := prefix + `,"organizationContext":` + wire + `}`
			if !enabled {
				text = strings.Replace(text, `"enabled":true`, `"enabled":false`, 1)
			}
			var c HttpCapability
			if json.Unmarshal([]byte(text), &c) == nil {
				t.Fatal("accepted invalid policy", text)
			}
		}
	}
	for _, tail := range []string{`,"organizationContext":"optional","organizationContext":"required"}`, `,"OrganizationContext":"optional"}`, `,"organizationContext":"optional","ORGANIZATIONCONTEXT":"optional"}`, `,"organizationContext":"required","unknown":true}`} {
		var c HttpCapability
		if json.Unmarshal([]byte(prefix+tail), &c) == nil {
			t.Fatal("accepted duplicate/alias/unknown", tail)
		}
	}
	for _, policy := range []string{"", "required", "optional"} {
		c := base
		c.OrganizationContext = policy
		c.ContractDigest, _ = HttpCapabilityContractDigest(c)
		encoded, _ := json.Marshal(c)
		var decoded HttpCapability
		if err = json.Unmarshal(encoded, &decoded); err != nil || decoded.OrganizationContext != policy {
			t.Fatal(policy, err)
		}
		if err = validateCapability(c); err != nil {
			t.Fatal(err)
		}
	}
	for _, mutate := range []func(*HttpCapability){
		func(c *HttpCapability) { c.OrganizationContext = "bogus" },
		func(c *HttpCapability) { c.OrganizationContext = "optional"; c.RecordScope = &HttpRecordScopePolicy{} },
		func(c *HttpCapability) {
			c.OrganizationContext = "optional"
			c.ResponseAuthorization = &HttpResponseAuthorization{}
		},
		func(c *HttpCapability) {
			c.OrganizationContext = "required"
			c.AccessMode = "public_authn"
			c.ActionID = nil
			c.Resource = nil
		},
	} {
		c := base
		mutate(&c)
		c.Enabled = false
		c.ContractDigest, _ = HttpCapabilityContractDigest(c)
		if validateCapability(c) == nil {
			t.Fatal("accepted disabled invalid policy", c)
		}
	}
	// Reusing a pin cannot authorize a policy change.
	base.OrganizationContext = "optional"
	if validateCapability(base) == nil {
		t.Fatal("policy tampering reused contract pin")
	}
}

func TestOrganizationContextBatchRequiresEveryChildIncludingUpserts(t *testing.T) {
	stage, root, _ := batchFixture(t)
	for i := range stage.Capabilities {
		if stage.Capabilities[i].Batch != nil {
			stage.Capabilities[i].OrganizationContext = "optional"
			root = stage.Capabilities[i]
		} else {
			stage.Capabilities[i].OrganizationContext = "optional"
		}
	}
	if err := validateBatchReferences(stage.Capabilities); err != nil {
		t.Fatal(err)
	}
	ids := append([]string(nil), root.Batch.AllowedCapabilityIDs...)
	for _, u := range root.Batch.Upserts {
		ids = append(ids, u.CreateCapabilityID, u.UpdateCapabilityID)
	}
	for _, id := range ids {
		for _, policy := range []string{"", "required"} {
			caps := append([]HttpCapability(nil), stage.Capabilities...)
			for i := range caps {
				if caps[i].CapabilityID == id {
					caps[i].OrganizationContext = policy
				}
			}
			if validateBatchReferences(caps) == nil {
				t.Fatal("optional batch admitted required child", id, policy)
			}
		}
	}
	// Required roots can reference either policy.
	for i := range stage.Capabilities {
		if stage.Capabilities[i].Batch != nil {
			stage.Capabilities[i].OrganizationContext = "required"
		} else {
			stage.Capabilities[i].OrganizationContext = ""
		}
	}
	if err := validateBatchReferences(stage.Capabilities); err != nil {
		t.Fatal(err)
	}
}
