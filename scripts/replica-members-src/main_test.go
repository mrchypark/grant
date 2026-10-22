package main

import (
	"bytes"
	"encoding/json"
	"strings"
	"testing"

	"github.com/mrchypark/rhiza"
)

func TestReplicaMemberInputs(t *testing.T) {
	const clusterID, token = "cluster-a", "private-token"
	key := rhiza.PeerPublicKey(clusterID, "n1", token)
	keyJSON, err := json.Marshal(key)
	if err != nil {
		t.Fatal(err)
	}

	for _, test := range []struct {
		name, member string
	}{
		{name: "legacy token", member: `"token":"private-token"`},
		{name: "public key only", member: `"public_key":` + string(keyJSON)},
	} {
		t.Run(test.name, func(t *testing.T) {
			inputJSON := `{"ClusterID":"cluster-a","Members":[{"node_id":"n1","peer_url":"quic://n1","log_url":"http://n1/log","wal_identity":"wal-1",` + test.member + `}]}`
			input, err := decodeInput(strings.NewReader(inputJSON))
			if err != nil {
				t.Fatal(err)
			}
			if input.Members[0].LogURL != "http://n1/log" || input.Members[0].WALIdentity != "wal-1" {
				t.Fatalf("native member fields lost: %+v", input.Members[0].Member)
			}
			got, err := convert(input.ClusterID, input.Members)
			if err != nil {
				t.Fatal(err)
			}
			if len(got) != 1 || got[0].PublicKey != [32]byte(key) {
				t.Fatalf("replica member = %+v", got)
			}
			output, err := json.Marshal(got)
			if err != nil {
				t.Fatal(err)
			}
			if bytes.Contains(output, []byte(token)) || !bytes.Contains(output, []byte(`"PublicKey"`)) {
				t.Fatalf("output does not preserve public-only contract: %s", output)
			}
		})
	}
}

func TestReplicaMemberRejectsMismatchedTokenAndKey(t *testing.T) {
	input := memberInput{
		Member: rhiza.Member{ID: "n1", PeerURL: "quic://n1", PublicKey: rhiza.PeerPublicKey("cluster-a", "n1", "expected-token")},
		Token:  "wrong-token",
	}
	if _, err := convert("cluster-a", []memberInput{input}); err == nil {
		t.Fatal("mismatched token and public key accepted")
	}
}
