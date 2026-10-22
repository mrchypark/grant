package main

import (
	"encoding/json"
	"fmt"
	"io"
	"os"

	"github.com/mrchypark/rhiza"
)

type memberInput struct {
	rhiza.Member
	Token string `json:"token"`
}

type requestInput struct {
	ClusterID string
	Members   []memberInput
}

func decodeInput(reader io.Reader) (requestInput, error) {
	var input requestInput
	decoder := json.NewDecoder(io.LimitReader(reader, 1<<20))
	decoder.DisallowUnknownFields()
	err := decoder.Decode(&input)
	return input, err
}

func convert(clusterID string, members []memberInput) ([]rhiza.ReplicaMember, error) {
	if clusterID == "" || len(members) == 0 {
		return nil, fmt.Errorf("cluster ID and members are required")
	}
	out := make([]rhiza.ReplicaMember, 0, len(members))
	for _, input := range members {
		key := input.PublicKey
		if input.Token != "" {
			derived := rhiza.PeerPublicKey(clusterID, string(input.ID), input.Token)
			if key != (rhiza.PublicKey{}) && key != derived {
				return nil, fmt.Errorf("member token does not match public key")
			}
			key = derived
		}
		member := input.Member
		member.PublicKey = key
		replica, err := rhiza.NewReplicaMember(clusterID, member)
		if err != nil {
			return nil, err
		}
		out = append(out, replica)
	}
	return out, nil
}

func main() {
	input, err := decodeInput(os.Stdin)
	if err != nil {
		fail()
	}
	out, err := convert(input.ClusterID, input.Members)
	if err != nil {
		fail()
	}
	if err := json.NewEncoder(os.Stdout).Encode(out); err != nil {
		fail()
	}
}

func fail() { fmt.Fprintln(os.Stderr, "invalid replica member input"); os.Exit(1) }
