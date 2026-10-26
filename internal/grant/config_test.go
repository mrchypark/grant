package grant

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/mrchypark/rhiza"
)

func TestConfigManagementOrganizationID(t *testing.T) {
	t.Setenv("GRANT_MASTER_KEY", "master-key")
	t.Setenv("GRANT_APP_AUTH_KEY", "app-key")
	t.Setenv("GRANT_AUTH_PROVIDER", "pocketbase")
	t.Setenv("GRANT_POCKETBASE_BASE_URL", "http://127.0.0.1:9999")
	t.Setenv("GRANT_POCKETBASE_APPLICATION_ID", "app-id")
	t.Setenv("GRANT_MANAGEMENT_ORGANIZATION_ID", "ops-org")
	cfg, err := ConfigFromEnv()
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Security.ManagementOrganizationID != "ops-org" {
		t.Fatalf("expected ops-org, got %q", cfg.Security.ManagementOrganizationID)
	}
}

func TestConfigManagementOrganizationIDUnset(t *testing.T) {
	t.Setenv("GRANT_MASTER_KEY", "master-key")
	t.Setenv("GRANT_APP_AUTH_KEY", "app-key")
	t.Setenv("GRANT_AUTH_PROVIDER", "pocketbase")
	t.Setenv("GRANT_POCKETBASE_BASE_URL", "http://127.0.0.1:9999")
	t.Setenv("GRANT_POCKETBASE_APPLICATION_ID", "app-id")
	t.Setenv("GRANT_MANAGEMENT_ORGANIZATION_ID", "")
	cfg, err := ConfigFromEnv()
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Security.ManagementOrganizationID != "" {
		t.Fatalf("expected empty, got %q", cfg.Security.ManagementOrganizationID)
	}
}

func TestManagementAndPEPIdentitiesUseSeparateConfig(t *testing.T) {
	t.Setenv("GRANT_MASTER_KEY", "master-key")
	t.Setenv("GRANT_APP_AUTH_KEY", "app-key")
	t.Setenv("GRANT_AUTH_PROVIDER", "pocketbase")
	t.Setenv("GRANT_POCKETBASE_BASE_URL", "http://127.0.0.1:9999")
	t.Setenv("GRANT_POCKETBASE_APPLICATION_ID", "app-id")
	t.Setenv("GRANT_HTTP_CAPABILITY_PEP_CLIENT_ID", "exposure-pep")
	t.Setenv("GRANT_MANAGEMENT_APPLICATION_ID", "management-app")
	t.Setenv("GRANT_HTTP_CAPABILITY_PEP_APPLICATION_ID", "")
	cfg, err := ConfigFromEnv()
	if err != nil || cfg.Security.HTTPCapabilityPEPClientID != "exposure-pep" || cfg.Security.ManagementApplicationID != "management-app" {
		t.Fatalf("independent identities not loaded: %+v %v", cfg.Security, err)
	}

	t.Setenv("GRANT_MANAGEMENT_APPLICATION_ID", "")
	t.Setenv("GRANT_HTTP_CAPABILITY_PEP_APPLICATION_ID", "legacy-management-app")
	cfg, err = ConfigFromEnv()
	if err != nil || cfg.Security.ManagementApplicationID != "legacy-management-app" || cfg.Security.HTTPCapabilityPEPClientID != "exposure-pep" {
		t.Fatalf("deprecated app alias changed PEP identity: %+v %v", cfg.Security, err)
	}

	t.Setenv("GRANT_MANAGEMENT_APPLICATION_ID", "management-app")
	if _, err = ConfigFromEnv(); err == nil {
		t.Fatal("conflicting management application settings accepted")
	}
}

func TestLoadDatabaseConfigMembership(t *testing.T) {
	legacy := `[{"node_id":"n1","peer_url":"quic://n1:8100","token":"token1"},{"node_id":"n2","peer_url":"quic://n2:8100","token":"token2"}]`
	public := []rhiza.Member{
		{ID: "n1", PeerURL: "quic://n1:8100", PublicKey: rhiza.PeerPublicKey("cluster", "n1", "token1")},
		{ID: "n2", PeerURL: "quic://n2:8100", PublicKey: rhiza.PeerPublicKey("cluster", "n2", "token2")},
	}
	publicJSON, err := json.Marshal(public)
	if err != nil {
		t.Fatal(err)
	}
	for _, tc := range []struct {
		name, members, override, token string
		wantErr, wantFallback          bool
	}{
		{"legacy members alone", legacy, "", "", false, true},
		{"omitted JSON members", legacy, `{}`, "", false, true},
		{"null JSON members", legacy, `{"Members":null}`, "", false, true},
		{"JSON legacy members", "", `{"Members":` + legacy + `}`, "", false, false},
		{"public members and local token", string(publicJSON), "", "token2", false, true},
		{"explicit public override", legacy, `{"Members":` + string(publicJSON) + `,"PeerToken":"token2"}`, "token2", false, false},
		{"empty override rejects cluster", legacy, `{"Members":[]}`, "", true, false},
		{"local mode rejects cluster", legacy, `{"Local":true}`, "", true, false},
		{"legacy local token mismatch", legacy, "", "wrong", true, false},
		{"public local token mismatch", string(publicJSON), "", "wrong", true, false},
		{"environment config token conflict", legacy, `{"PeerToken":"token2"}`, "wrong", true, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			clearDatabaseConfigEnv(t)
			t.Setenv("GRANT_CLUSTER_MEMBERS", tc.members)
			t.Setenv("GRANT_RHIZA_CONFIG_JSON", tc.override)
			t.Setenv("RHIZA_PEER_TOKEN", tc.token)
			cfg := Config{Database: DatabaseConfig{ClusterID: "cluster", RuntimeMode: "embedded", LocalNodeRole: "voter"}}
			err := loadDatabaseConfig(&cfg)
			if tc.wantErr {
				if err == nil {
					t.Fatal("unsafe configuration accepted")
				}
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			if tc.wantFallback && cfg.Database.Rhiza.Members != nil {
				t.Fatal("absent override became non-nil membership")
			}
			effective, err := effectiveRhizaConfig(cfg.Database)
			if err != nil || effective.Local || effective.ClusterID != "cluster" || effective.NodeID != "n2" || effective.PeerToken != "token2" || len(effective.Members) != 2 {
				t.Fatalf("effective config: %+v, error: %v", effective, err)
			}
			for i, member := range effective.Members {
				if member != public[i] {
					t.Fatalf("member %d: got %+v, want %+v", i, member, public[i])
				}
			}
		})
	}
	for _, key := range []string{"GRANT_CLUSTER_MEMBERS", "GRANT_RHIZA_CONFIG_JSON", "RHIZA_PEER_TOKEN"} {
		t.Run("learner rejects "+key, func(t *testing.T) {
			clearDatabaseConfigEnv(t)
			t.Setenv("GRANT_REMOTE_NODES", "http://voter:8081")
			t.Setenv("GRANT_READ_FENCE_KEY", strings.Repeat("f", 40))
			t.Setenv("GRANT_RHIZA_REPLICA_CONFIG_JSON", `{"Members":[{"ID":"n1"}]}`)
			value := legacy
			if key == "GRANT_RHIZA_CONFIG_JSON" {
				value = `{"Members":` + legacy + `}`
			} else if key == "RHIZA_PEER_TOKEN" {
				value = "token2"
			}
			t.Setenv(key, value)
			cfg := Config{Database: DatabaseConfig{ClusterID: "cluster", RuntimeMode: "embedded", LocalNodeRole: "learner"}}
			if err := loadDatabaseConfig(&cfg); err == nil {
				t.Fatal("learner accepted voter secrets")
			}
		})
	}
	t.Run("default membership stays nil", func(t *testing.T) {
		clearDatabaseConfigEnv(t)
		cfg := Config{Database: DatabaseConfig{RuntimeMode: "embedded", LocalNodeRole: "voter"}}
		if err := loadDatabaseConfig(&cfg); err != nil {
			t.Fatal(err)
		}
		effective, err := effectiveRhizaConfig(cfg.Database)
		if err != nil || effective.Members != nil || effective.PeerToken != "" {
			t.Fatalf("unexpected defaults: %+v, error: %v", effective, err)
		}
	})
}

func TestLoadOperatorDatabaseConfig(t *testing.T) {
	clearDatabaseConfigEnv(t)
	t.Setenv("GRANT_NODE_ID", "")
	members := make([]rhiza.Member, 3)
	tokens := map[string]string{}
	for i, id := range []string{"n1", "n2", "n3"} {
		token := strings.Repeat(string(rune('a'+i)), 40)
		tokens[id] = token
		members[i] = rhiza.Member{PeerURL: "quic://" + id + ":9090", PublicKey: rhiza.PeerPublicKey("operator", id, token)}
		switch id {
		case "n1":
			members[i].ID = "n1"
		case "n2":
			members[i].ID = "n2"
		case "n3":
			members[i].ID = "n3"
		}
	}
	rawMembers, err := json.Marshal(members)
	if err != nil {
		t.Fatal(err)
	}
	rawTokens, err := json.Marshal(tokens)
	if err != nil {
		t.Fatal(err)
	}
	t.Setenv("RHIZA_BIND_ADDR", "127.0.0.1:9091")
	t.Setenv("RHIZA_NODE_ID", "n2")
	t.Setenv("RHIZA_CLUSTER_ID", "operator")
	t.Setenv("RHIZA_DATA_DIR", "/data")
	t.Setenv("RHIZA_PEER_ADDR", "0.0.0.0:9090")
	t.Setenv("RHIZA_CLUSTER_MEMBERS", string(rawMembers))
	t.Setenv("RHIZA_PEER_TOKENS", string(rawTokens))
	t.Setenv("RHIZA_ADMIN_TOKEN", strings.Repeat("z", 40))
	t.Setenv("RHIZA_ENABLE_RECONFIGURATION", "true")
	t.Setenv("GRANT_RUNTIME_BIND_ADDR", "127.0.0.1:9444")
	t.Setenv("GRANT_REMOTE_API_SECRET", strings.Repeat("r", 40))
	t.Setenv("GRANT_READ_FENCE_KEY", strings.Repeat("f", 40))

	cfg := Config{Security: SecurityConfig{MasterKey: "master", AppAuthKey: "app"}, Database: DatabaseConfig{DefaultReadConsistency: ReadLocal}}
	if err := loadDatabaseConfig(&cfg); err != nil {
		t.Fatal(err)
	}
	if cfg.Database.OperatorBindAddr != "127.0.0.1:9091" || cfg.Database.RuntimeBindAddr != "127.0.0.1:9444" || cfg.Database.Rhiza.NodeID != "n2" || !cfg.Database.Rhiza.EnableReconfiguration || len(cfg.Database.Rhiza.Members) != 3 {
		t.Fatalf("operator config was not preserved: %+v", cfg.Database)
	}

	t.Setenv("GRANT_CLUSTER_ID", "stale")
	if err := loadDatabaseConfig(&cfg); err == nil {
		t.Fatal("stale Grant topology alias was accepted")
	}
	t.Setenv("GRANT_CLUSTER_ID", "")
	tokens["n4"] = strings.Repeat("q", 40)
	rawTokens, err = json.Marshal(tokens)
	if err != nil {
		t.Fatal(err)
	}
	learner := rhiza.Member{ID: "n4", PeerURL: "quic://n4:9090", PublicKey: rhiza.PeerPublicKey("operator", "n4", tokens["n4"])}
	rawLearner, err := json.Marshal(learner)
	if err != nil {
		t.Fatal(err)
	}
	t.Setenv("RHIZA_NODE_ID", "n4")
	t.Setenv("RHIZA_PEER_TOKENS", string(rawTokens))
	t.Setenv("RHIZA_LEARNER", string(rawLearner))
	if err := loadDatabaseConfig(&cfg); err != nil {
		t.Fatal(err)
	}
	if cfg.Database.Rhiza.Learner == nil || cfg.Database.Rhiza.Learner.ID != "n4" {
		t.Fatalf("operator learner was not preserved: %+v", cfg.Database.Rhiza.Learner)
	}
}

func clearDatabaseConfigEnv(t *testing.T) {
	t.Helper()
	for _, key := range []string{"GRANT_DATA_DIR", "GRANT_NODE_ID", "GRANT_CLUSTER_ID", "GRANT_PEER_ADDR", "GRANT_RUNTIME_MODE", "GRANT_LOCAL_NODE_ROLE", "GRANT_REMOTE_NODES", "GRANT_REMOTE_API_SECRET", "GRANT_READ_FENCE_KEY", "GRANT_RUNTIME_BIND_ADDR", "GRANT_CLUSTER_MEMBERS", "GRANT_RHIZA_CONFIG_JSON", "GRANT_RHIZA_REPLICA_CONFIG_JSON", "RHIZA_BIND_ADDR", "RHIZA_NODE_ID", "RHIZA_CLUSTER_ID", "RHIZA_DATA_DIR", "RHIZA_PEER_ADDR", "RHIZA_CLUSTER_MEMBERS", "RHIZA_PEER_TOKENS", "RHIZA_PEER_TOKEN", "RHIZA_ADMIN_TOKEN", "RHIZA_ENABLE_RECONFIGURATION", "RHIZA_LEARNER", "RHIZA_OBJSTORE_ENDPOINT", "RHIZA_OBJSTORE_DURABILITY", "RHIZA_OBJSTORE_INSECURE"} {
		t.Setenv(key, "")
	}
	t.Setenv("GRANT_NODE_ID", "n2")
}
