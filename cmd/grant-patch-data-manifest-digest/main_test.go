package main

import (
	"bytes"
	"strings"
	"testing"
)

func TestTypedManifestShape(t *testing.T) {
	for _, input := range []string{`{}`, `null`, `{"manifestVersion":null}`, `{"manifestVersion":"grant.patch-data-authority.v1","generation":"1","ledgerDigest":"x","users":null,"organizations":[],"memberships":[],"hierarchyEdges":[],"plantBindings":[],"applicationAssignments":[]}`} {
		var out bytes.Buffer
		if e := run(strings.NewReader(input), &out); e == nil {
			t.Fatalf("accepted %s", input)
		}
		if out.Len() != 0 {
			t.Fatal("printed digest for rejected input")
		}
	}
}

func TestInvalidManifestEnums(t *testing.T) {
	for _, entry := range []string{`"memberships":[{"membershipId":"m","userId":"u","organizationId":"o","role":"bogus","status":"active"}]`, `"memberships":[{"membershipId":"m","userId":"u","organizationId":"o","role":"member","status":"bogus"}]`, `"plantBindings":[{"bindingId":"b","subjectType":"bogus","subjectId":"u","subjectOrganizationId":"o","plantId":"p","state":"active"}]`, `"plantBindings":[{"bindingId":"b","subjectType":"user","subjectId":"u","subjectOrganizationId":"o","plantId":"p","state":"bogus"}]`} {
		input := `{"manifestVersion":"grant.patch-data-authority.v1","generation":"1","ledgerDigest":"x","users":[],"organizations":[],"memberships":[],"hierarchyEdges":[],"plantBindings":[],"applicationAssignments":[],` + entry + `}`
		var out bytes.Buffer
		if err := run(strings.NewReader(input), &out); err == nil || !strings.Contains(err.Error(), "invalid") || out.Len() != 0 {
			t.Fatalf("accepted invalid enum: %s %v", input, err)
		}
	}
}
