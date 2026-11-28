package main

import (
	"encoding/json"
	"fmt"
	"io"
	"os"

	"github.com/mrchypark/rhiza"
)

type replicaMemberInput struct {
	rhiza.Member
	Token string `json:"token"`
}

type replicaMembersRequest struct {
	ClusterID string
	Members   []replicaMemberInput
}

func runReplicaMembers() {
	decoder := json.NewDecoder(io.LimitReader(os.Stdin, 1<<20))
	decoder.DisallowUnknownFields()
	var input replicaMembersRequest
	if err := decoder.Decode(&input); err != nil || input.ClusterID == "" || len(input.Members) == 0 {
		failReplicaMembers()
	}
	out := make([]rhiza.ReplicaMember, 0, len(input.Members))
	for _, m := range input.Members {
		key := m.PublicKey
		if m.Token != "" {
			derived := rhiza.PeerPublicKey(input.ClusterID, string(m.ID), m.Token)
			if key != (rhiza.PublicKey{}) && key != derived {
				failReplicaMembers()
			}
			key = derived
		}
		member := m.Member
		member.PublicKey = key
		replica, err := rhiza.NewReplicaMember(input.ClusterID, member)
		if err != nil {
			failReplicaMembers()
		}
		out = append(out, replica)
	}
	if err := json.NewEncoder(os.Stdout).Encode(out); err != nil {
		failReplicaMembers()
	}
}

func failReplicaMembers() {
	fmt.Fprintln(os.Stderr, "invalid replica member input")
	os.Exit(1)
}
