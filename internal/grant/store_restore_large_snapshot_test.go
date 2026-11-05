package grant

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"testing"
)

// Only synthetic snapshot data changes here. The native import implementation
// remains the v0.9.0 implementation (f0ab0e7684f279dc84f82f22c7537d3679780eb9).
// Raw snapshot/cell sizes do not establish the size of a JSON-encoded command.
func TestLargeRestoreSnapshotFixture(t *testing.T) {
	largeRestoreFrozenSource(t, context.Background())
}

func largeRestoreFrozenSource(t *testing.T, ctx context.Context) StoreSnapshot {
	t.Helper()
	snap := restoreFrozenSource(t, ctx)
	table := func(name string) *SnapshotTable {
		for i := range snap.Tables {
			if snap.Tables[i].Name == name {
				return &snap.Tables[i]
			}
		}
		t.Fatalf("missing table %s", name)
		return nil
	}
	appendRow := func(dst *SnapshotTable, values map[string]any) {
		row := make([]any, len(dst.Columns))
		for i, column := range dst.Columns {
			value, ok := values[column]
			if !ok {
				t.Fatalf("missing synthetic %s.%s", dst.Name, column)
			}
			row[i] = value
		}
		dst.Rows = append(dst.Rows, row)
	}
	jsonBytes := func(value any) []byte {
		b, err := json.Marshal(value)
		if err != nil {
			t.Fatal(err)
		}
		return b
	}
	const provider = "grant-management"
	const timestamp = "2026-01-01T00:00:00Z"
	appendRow(table("service_providers"), map[string]any{
		"id": provider, "name": "Synthetic management provider", "description": nil,
		"active": int64(1), "created_at": timestamp, "updated_at": timestamp,
	})
	stage, actions, err := ManagementCatalog(1, nil)
	if err != nil {
		t.Fatal(err)
	}
	if len(stage.Capabilities) != 77 {
		t.Fatalf("release fixture requires official 77 capabilities, got %d", len(stage.Capabilities))
	}
	actionTable := table("actions")
	addAction := func(id, code, name string, description any, grantable int64) {
		appendRow(actionTable, map[string]any{
			"id": id, "provider_id": provider, "code": code, "name": name,
			"description": description, "risk_level": "high", "visibility_granting": int64(0),
			"grantable_to_applications": grantable, "consent_label": nil, "consent_description": nil,
			"created_at": timestamp, "updated_at": timestamp,
		})
	}
	for _, action := range actions {
		var description any
		if action.Description != nil {
			description = *action.Description
		}
		addAction(action.ID, action.Code, action.Name, description, 1)
	}
	for len(actionTable.Rows) < 130 {
		code := fmt.Sprintf("synthetic.%03d", len(actionTable.Rows))
		addAction(provider+"."+code, code, "Synthetic action", "Synthetic fixture metadata", 0)
	}
	if len(actionTable.Rows) != 130 {
		t.Fatalf("expected 130 actions, got %d", len(actionTable.Rows))
	}
	// Match the reported action-table scale without copying deployment metadata.
	for i, column := range actionTable.Columns {
		if column == "description" {
			row := actionTable.Rows[len(actionTable.Rows)-1]
			if deficit := 31651 - len(jsonBytes(actionTable.Rows)); deficit > 0 {
				row[i] = row[i].(string) + strings.Repeat("x", deficit)
			}
		}
	}
	catalogTable := table("provider_http_capability_sets")
	addCatalog := func(c StageHttpCapabilityGeneration) string {
		if err := validateGeneration(provider, c); err != nil {
			t.Fatal(err)
		}
		canonical, err := generationJSON(provider, c)
		if err != nil {
			t.Fatal(err)
		}
		appendRow(catalogTable, map[string]any{
			"provider_id": provider, "generation": int64(c.Generation),
			"catalog_digest": c.CatalogDigest, "canonical_json": canonical, "created_at": timestamp,
		})
		return canonical
	}
	first := addCatalog(stage) // Official generation 1 is deliberately unchanged.
	previous := uint64(1)
	second, _, err := ManagementCatalog(2, &previous)
	if err != nil {
		t.Fatal(err)
	}
	// Keep every official capability and reference in both generations. This
	// deliberately exceeds the second catalog's reported scale rather than
	// constructing a subset with potentially incomplete cross-references.
	secondJSON := addCatalog(second)
	if len(second.Capabilities) != 77 || len(catalogTable.Rows) != 2 ||
		len(secondJSON) < 50000 || max(len(first), len(secondJSON)) < 66099 ||
		len(jsonBytes(catalogTable.Rows)) < 116201 || len(jsonBytes(actionTable.Rows)) < 31651 {
		t.Fatalf("synthetic size/count requirements unmet: secondCapabilities=%d catalogs=%d firstBytes=%d secondBytes=%d catalogsRowsBytes=%d actionsRowsBytes=%d",
			len(second.Capabilities), len(catalogTable.Rows), len(first), len(secondJSON), len(jsonBytes(catalogTable.Rows)), len(jsonBytes(actionTable.Rows)))
	}
	if len(snap.Tables) != 31 {
		t.Fatalf("expected 31 canonical tables, got %d", len(snap.Tables))
	}
	for i := range snap.Tables {
		snap.Tables[i].Count = int64(len(snap.Tables[i].Rows))
		snap.Tables[i].Digest, err = tableDigest(snap.Tables[i])
		if err != nil {
			t.Fatal(err)
		}
	}
	snap.Digest, err = snapshotDigest(snap)
	if err != nil {
		t.Fatal(err)
	}
	if err := ValidateStoreSnapshot(snap); err != nil {
		t.Fatal(err)
	}
	t.Logf("synthetic restore fixture: tables=%d actions=%d actionsRowsJSONBytes=%d catalogs=%d catalogsRowsJSONBytes=%d officialCapabilities=%d officialCanonicalBytes=%d secondCanonicalBytes=%d maxCanonicalCellBytes=%d snapshotJSONBytes=%d",
		len(snap.Tables), len(actionTable.Rows), len(jsonBytes(actionTable.Rows)), len(catalogTable.Rows), len(jsonBytes(catalogTable.Rows)), len(stage.Capabilities), len(first), len(secondJSON), max(len(first), len(secondJSON)), len(jsonBytes(snap)))
	return snap
}
