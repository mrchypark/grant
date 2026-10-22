// grant-management-catalog emits the explicit management route registry for
// review and staging.
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
	flag.Parse()
	var expected *uint64
	if *previous != 0 {
		expected = previous
	}
	stage, definitions, err := grant.ManagementCatalog(*generation, expected)
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
