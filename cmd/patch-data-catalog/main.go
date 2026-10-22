// patch-data-catalog emits explicit native record mappings for review and staging.
package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"os"

	"github.com/mrchypark/grant/internal/grant"
)

func main() {
	generation := flag.Uint64("generation", 1, "new catalog generation")
	previous := flag.Uint64("previous-generation", 0, "expected latest generation (0 means none)")
	actions := flag.Bool("actions", false, "emit action definitions instead of stage payload")
	readOnly := flag.Bool("read-only", false, "emit only collection list/detail reads (168 routes)")
	existingActions := flag.String("existing-actions", "", "with -actions, merge an existing provider action array without removing actions or changing existing metadata")
	flag.Parse()
	var expected *uint64
	if *previous != 0 {
		expected = previous
	}
	build := grant.PatchDataCatalog
	if *readOnly {
		build = grant.PatchDataReadCatalog
	}
	stage, definitions, err := build(*generation, expected)
	if err == nil && *existingActions != "" {
		if !*actions {
			err = fmt.Errorf("-existing-actions requires -actions")
		} else {
			var raw []byte
			var existing []grant.Action
			raw, err = os.ReadFile(*existingActions)
			if err == nil {
				err = json.Unmarshal(raw, &existing)
			}
			if err == nil {
				definitions, err = grant.MergePatchDataReadActions(existing, definitions)
			}
		}
	}
	if err == nil && flag.NArg() != 0 {
		err = fmt.Errorf("unexpected arguments")
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	var output any = stage
	if *actions {
		output = definitions
	}
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "  ")
	if err = encoder.Encode(output); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
