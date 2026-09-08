package main

import (
	"encoding/json"
	"fmt"
	"github.com/mrchypark/rhiza"
	"io"
	"os"
)

func main() {
	var in struct {
		ClusterID string
		Members   []rhiza.Member
	}
	d := json.NewDecoder(io.LimitReader(os.Stdin, 1<<20))
	d.DisallowUnknownFields()
	if err := d.Decode(&in); err != nil {
		fail()
	}
	if in.ClusterID == "" || len(in.Members) == 0 {
		fail()
	}
	out := make([]rhiza.ReplicaMember, 0, len(in.Members))
	for _, m := range in.Members {
		r, err := rhiza.NewReplicaMember(in.ClusterID, m)
		if err != nil {
			fail()
		}
		out = append(out, r)
	}
	if err := json.NewEncoder(os.Stdout).Encode(out); err != nil {
		fail()
	}
}
func fail() { fmt.Fprintln(os.Stderr, "invalid replica member input"); os.Exit(1) }
