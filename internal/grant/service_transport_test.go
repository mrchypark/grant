package grant

import (
	"encoding/json"
	"os"
	"sort"
	"strings"
	"testing"
)

func TestCanonicalUTF8QueryAndExplicitRepeats(t *testing.T) {
	policy := HttpQueryPolicy{Mode: "exact", Parameters: []HttpQueryParameter{{Name: "filter", Format: "text", Required: true, MaxBytes: 128}, {Name: "tag", Format: "text", MaxBytes: 32, Repeated: true}}}
	for _, q := range []string{"filter=name%3D%22%ED%95%9C%20%EA%B8%80%22&tag=a&tag=b", "filter="} {
		if _, e := canonicalQuery(policy, q); e != nil {
			t.Fatalf("%s: %v", q, e)
		}
	}
	for _, q := range []string{"filter=a+b", "filter=%2f", "filter=%FF", "filter=%00", "filter=a&filter=b", "filter=x&tag=" + strings.Repeat("a", 33), "tag=a&filter=x", "unknown=x"} {
		if _, e := canonicalQuery(policy, q); e == nil {
			t.Errorf("accepted %s", q)
		}
	}
}
func TestEncodedFilenameCaptureIsExplicit(t *testing.T) {
	template := "/files/{record}/{filename}"
	params := []HttpPathParameter{{Name: "filename", Format: "utf8Segment", MaxBytes: 64}}
	captures, e := matchPolicyPath(template, "/files/record1/%ED%95%9C%20%EA%B8%80.pdf", params)
	if e != nil || captures["filename"] != "한 글.pdf" {
		t.Fatal(captures, e)
	}
	for _, path := range []string{"/files/%72ecord1/a", "/files/record1/%2fetc", "/files/record1/%5cetc", "/files/record1/%252e%252e", "/files/record1/%2e%2e", "/files/record1/%FF", "/files/record1/%00", "/files/record1/" + strings.Repeat("a", 65)} {
		if _, e := matchPolicyPath(template, path, params); e == nil {
			t.Errorf("accepted %s", path)
		}
	}
}
func TestOpaqueMultipartAndNestedConstantPolicy(t *testing.T) {
	action := "record.write"
	c := HttpCapability{CapabilityID: "upload", Version: 1, Enabled: true, AccessMode: "authorized", Method: "POST", PathTemplate: "/collections/{collection}/records", ActionID: &action, Resource: &HttpCapabilityResource{ResourceType: "collection", ID: HttpResourceExtractor{Kind: "constant", Value: "records"}}, Query: HttpQueryPolicy{Mode: "none"}, Request: HttpRequestPolicy{Body: "multipart", MaxBytes: 1024, ForwardHeaders: []string{"content-type"}, ContentTypes: []string{"multipart/form-data"}}, Response: HttpResponsePolicy{MaxBytes: 1024, ForwardHeaders: []string{"content-disposition"}, Redirects: "deny", ErrorStatuses: []int{304, 412, 416}}}
	if e := validateTestCapability(c); e != nil {
		t.Fatal(e)
	}
	c.Request.Body = "opaque"
	c.Request.ContentTypes = []string{"application/octet-stream"}
	if e := validateTestCapability(c); e != nil {
		t.Fatal(e)
	}
	for _, types := range [][]string{nil, {"application/octet-stream; charset=utf-8"}, {"Application/Octet-Stream"}, {"application/octet-stream", "application/octet-stream"}} {
		c.Request.ContentTypes = types
		if e := validateTestCapability(c); e == nil {
			t.Errorf("accepted %v", types)
		}
	}
	c.Request.ContentTypes = []string{"application/octet-stream"}
	c.Resource.ID = HttpResourceExtractor{Kind: "jsonPointer", Pointer: "/id", Format: "asciiToken"}
	if e := validateTestCapability(c); e == nil {
		t.Fatal("opaque body pointer accepted")
	}
	c.Request.Body = "json"
	c.Request.ContentTypes = nil
	if e := validateTestCapability(c); e != nil {
		t.Fatal("nested JSON pointer", e)
	}
	c.Request.Body = "none"
	c.Request.MaxBytes = 0
	c.Query = HttpQueryPolicy{Mode: "exact", Parameters: []HttpQueryParameter{{Name: "id", Format: "asciiToken", Required: true}}}
	c.Resource.ID = HttpResourceExtractor{Kind: "queryScalar", Name: "id", Format: "asciiToken"}
	if e := validateTestCapability(c); e != nil {
		t.Fatal("nested query scalar", e)
	}
	c.Query.Parameters[0].Repeated = true
	if e := validateTestCapability(c); e == nil {
		t.Fatal("ambiguous repeated resource accepted")
	}
}

func TestScopeOnlyAcceptsDeclaredPaginationRoles(t *testing.T) {
	raw, e := os.ReadFile("testdata/pocketbase-plants-list.json")
	if e != nil {
		t.Fatal(e)
	}
	var c HttpCapability
	if e = json.Unmarshal(raw, &c); e != nil {
		t.Fatal(e)
	}
	if e = validateTestCapability(c); e != nil {
		t.Fatal(e)
	}
	for _, format := range []string{"text", "asciiToken", "uint", "bool"} {
		copy := c
		copy.Query.Parameters = append([]HttpQueryParameter(nil), c.Query.Parameters...)
		extra := HttpQueryParameter{Name: "expand", Format: format}
		if format == "text" {
			extra.MaxBytes = 128
		}
		copy.Query.Parameters = append(copy.Query.Parameters, extra)
		sort.Slice(copy.Query.Parameters, func(i, j int) bool { return copy.Query.Parameters[i].Name < copy.Query.Parameters[j].Name })
		if e = validateTestCapability(copy); e == nil {
			t.Fatalf("undeclared expand format%s accepted", format)
		}
	}
	copy := c
	scope := *c.RecordScope
	scope.SortParameter = ""
	copy.RecordScope = &scope
	if e = validateTestCapability(copy); e == nil {
		t.Fatal("unbound sort accepted")
	}
	for _, v := range []string{"owner", "-relation.field", "+1", "01", "4294967296"} {
		if e = scalarFormat("uint", v); e == nil {
			t.Errorf("uint accepted %s", v)
		}
	}
	for _, v := range []string{"owner", "1", "TRUE", ""} {
		if e = scalarFormat("bool", v); e == nil {
			t.Errorf("bool accepted %s", v)
		}
	}
}
