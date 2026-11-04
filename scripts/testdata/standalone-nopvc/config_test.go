package grant

import (
	"encoding/json"
	"fmt"
	"net"
	"os"
	"strings"
	"testing"

	"github.com/mrchypark/rhiza"
)

func noPVCEnv(t *testing.T, secret map[string]string) {
	t.Helper()
	for _, entry := range os.Environ() {
		if k, _, ok := strings.Cut(entry, "="); ok {
			t.Setenv(k, "")
		}
	}
	for k, v := range secret {
		t.Setenv(k, v)
	}
	raw, err := os.ReadFile(noPVCConfigFixture)
	if err != nil {
		t.Fatal(err)
	}
	var rendered map[string]string
	if err := json.Unmarshal(raw, &rendered); err != nil {
		t.Fatal(err)
	}
	for k, v := range rendered {
		t.Setenv(k, v)
	}
}

// This is a deployment-verifier policy, not an application/runtime fencing guard.
func noPVCRemoteConfig() error {
	cfg, err := ConfigFromEnv()
	if err != nil {
		return fmt.Errorf("ConfigFromEnv rejected configuration")
	}
	c, err := effectiveRhizaConfig(cfg.Database)
	if err != nil {
		return fmt.Errorf("effective Rhiza configuration rejected")
	}
	if !c.Local || c.NodeID != "grant-standalone" || c.ClusterID != "grant-standalone-v1" || len(c.Members) != 0 || c.PeerAddr != "" || c.BindAddr != "" || c.Learner != nil || c.EnableReconfiguration || c.AdminToken != "" {
		return fmt.Errorf("expected isolated stable Local identity without HA/Operator")
	}
	if cfg.Database.RuntimeMode != "embedded" || cfg.Database.LocalNodeRole != "voter" || len(cfg.Database.RemoteNodes) != 0 || cfg.Database.OperatorBindAddr != "" || cfg.Database.DataDir != "/var/lib/grant/rhiza" || c.DataDir != cfg.Database.DataDir {
		return fmt.Errorf("unexpected runtime or shared data path")
	}
	if c.ObjStoreProvider != "s3" || c.ObjStoreDir != "" || c.ObjStoreDurability != rhiza.ObjectStoreDurabilityBeforeAck || c.ObjStoreInsecure {
		return fmt.Errorf("requires remote S3 without filesystem directory, using TLS and before-ack")
	}
	for _, value := range []string{c.ObjStoreBucket, c.ObjStorePrefix, c.ObjStoreEndpoint, c.ObjStoreRegion, c.ObjStoreAccessKey, c.ObjStoreSecretKey} {
		if strings.TrimSpace(value) == "" {
			return fmt.Errorf("missing required remote S3 setting")
		}
	}
	host := c.ObjStoreEndpoint
	if h, _, err := net.SplitHostPort(host); err == nil {
		host = h
	}
	host = strings.Trim(strings.ToLower(host), "[]")
	if host == "localhost" || strings.HasSuffix(host, ".localhost") || host == "" || strings.ContainsAny(host, "/\\") {
		return fmt.Errorf("S3 endpoint must be remote")
	}
	if ip := net.ParseIP(host); ip != nil && (ip.IsLoopback() || ip.IsUnspecified() || ip.IsLinkLocalUnicast()) {
		return fmt.Errorf("S3 endpoint must be remote")
	}
	return nil
}

func noPVCTestSecret() map[string]string {
	return map[string]string{
		"GRANT_MASTER_KEY": "test-master", "GRANT_APP_AUTH_KEY": "test-app",
		"GRANT_AUTH_PROVIDER": "rauthy", "GRANT_RAUTHY_CLIENT_ID": "test",
		"GRANT_RAUTHY_CLIENT_SECRET": "test", "GRANT_RAUTHY_BASE_URL": "https://auth.invalid",
		"RHIZA_OBJSTORE_PROVIDER": "s3", "RHIZA_OBJSTORE_BUCKET": "test-archive",
		"RHIZA_OBJSTORE_PREFIX": "test-fresh-prefix", "RHIZA_OBJSTORE_ENDPOINT": "https://s3.remote.invalid",
		"RHIZA_OBJSTORE_REGION": "test-region", "RHIZA_OBJSTORE_ACCESS_KEY": "test-access",
		"RHIZA_OBJSTORE_SECRET_KEY": "test-secret",
	}
}

func TestNoPVCRemoteRenderedConfig(t *testing.T) {
	for _, tc := range []struct {
		name, key, value string
		reject           bool
	}{
		{name: "valid"},
		{name: "missing-bucket", key: "RHIZA_OBJSTORE_BUCKET", reject: true},
		{name: "missing-prefix", key: "RHIZA_OBJSTORE_PREFIX", reject: true},
		{name: "missing-endpoint", key: "RHIZA_OBJSTORE_ENDPOINT", reject: true},
		{name: "missing-region", key: "RHIZA_OBJSTORE_REGION", reject: true},
		{name: "missing-access-key", key: "RHIZA_OBJSTORE_ACCESS_KEY", reject: true},
		{name: "missing-secret-key", key: "RHIZA_OBJSTORE_SECRET_KEY", reject: true},
		{name: "http", key: "RHIZA_OBJSTORE_ENDPOINT", value: "http://s3.remote.invalid", reject: true},
		{name: "endpoint-path", key: "RHIZA_OBJSTORE_ENDPOINT", value: "https://s3.remote.invalid/archive", reject: true},
		{name: "filesystem-endpoint", key: "RHIZA_OBJSTORE_ENDPOINT", value: "file:///var/lib/grant/archive", reject: true},
		{name: "localhost", key: "RHIZA_OBJSTORE_ENDPOINT", value: "https://localhost:9000", reject: true},
		{name: "loopback", key: "RHIZA_OBJSTORE_ENDPOINT", value: "127.0.0.1:9000", reject: true},
		{name: "ipv6-loopback", key: "RHIZA_OBJSTORE_ENDPOINT", value: "[::1]:9000", reject: true},
		{name: "filesystem-secret-overridden", key: "RHIZA_OBJSTORE_PROVIDER", value: "filesystem"},
		{name: "local-dir-secret-cleared", key: "RHIZA_OBJSTORE_DIR", value: "/var/lib/grant/archive"},
		{name: "async-secret-overridden", key: "RHIZA_OBJSTORE_DURABILITY", value: "async"},
		{name: "json-secret-overridden", key: "GRANT_RHIZA_CONFIG_JSON", value: `{"Local":true,"ObjStoreProvider":"filesystem","ObjStoreDir":"/var/lib/grant/archive"}`},
	} {
		t.Run(tc.name, func(t *testing.T) {
			secret := noPVCTestSecret()
			if tc.key != "" {
				secret[tc.key] = tc.value
			}
			noPVCEnv(t, secret)
			err := noPVCRemoteConfig()
			if (err != nil) != tc.reject {
				t.Fatalf("configuration rejection=%v, expected %v", err != nil, tc.reject)
			}
			// Direct JSON conflicts must also be neutralized by explicit storage env fields.
			if tc.name == "json-secret-overridden" {
				t.Setenv("GRANT_RHIZA_CONFIG_JSON", `{"Local":true,"NodeID":"grant-standalone","ClusterID":"grant-standalone-v1","Members":[],"PeerAddr":"","ObjStoreProvider":"filesystem","ObjStoreDir":"/var/lib/grant/archive"}`)
				if err := noPVCRemoteConfig(); err != nil {
					t.Fatal(err)
				}
			}
		})
	}
}

func TestNoPVCProvidedConfig(t *testing.T) {
	raw, err := os.ReadFile(noPVCSecretFixture)
	if err != nil {
		t.Fatal(err)
	}
	if len(raw) == 0 {
		t.Skip("deployment secrets not supplied; synthetic configuration only")
	}
	var secret map[string]string
	if err := json.Unmarshal(raw, &secret); err != nil {
		t.Fatal("expected flat JSON environment object")
	}
	noPVCEnv(t, secret)
	if err := noPVCRemoteConfig(); err != nil {
		t.Fatal(err)
	}
}
