package grant

import (
	"encoding/json"
	"os"
	"testing"

	"github.com/mrchypark/rhiza"
)

func TestStandaloneRenderedConfig(t *testing.T) {
	// Hermetic env: no inherited operator/auth/storage settings enter the fixture.
	for _, entry := range os.Environ() {
		for i := range entry {
			if entry[i] == '=' {
				t.Setenv(entry[:i], "")
				break
			}
		}
	}
	raw, err := os.ReadFile(standaloneConfigFixture)
	if err != nil {
		t.Fatal(err)
	}
	var env map[string]string
	if err := json.Unmarshal(raw, &env); err != nil {
		t.Fatal(err)
	}
	for key, value := range env {
		t.Setenv(key, value)
	}
	// Synthetic credentials exist only in this no-Open configuration test.
	t.Setenv("GRANT_MASTER_KEY", "test-master")
	t.Setenv("GRANT_APP_AUTH_KEY", "test-app")
	t.Setenv("GRANT_AUTH_PROVIDER", "rauthy")
	t.Setenv("GRANT_RAUTHY_CLIENT_ID", "test")
	t.Setenv("GRANT_RAUTHY_CLIENT_SECRET", "test")
	t.Setenv("RHIZA_OBJSTORE_PROVIDER", "s3")
	t.Setenv("RHIZA_OBJSTORE_BUCKET", "test-archive")
	t.Setenv("RHIZA_OBJSTORE_PREFIX", "test-fresh-prefix")
	cfg, err := ConfigFromEnv()
	if err != nil {
		t.Fatal(err)
	}
	c, err := effectiveRhizaConfig(cfg.Database)
	if err != nil {
		t.Fatal(err)
	}
	if !c.Local || c.NodeID != "grant-standalone" || c.ClusterID != "grant-standalone-v1" || len(c.Members) != 0 || c.PeerAddr != "" || c.BindAddr != "" {
		t.Fatalf("invalid Local config: Local=%v node=%s cluster=%s members=%d peer=%q bind=%q", c.Local, c.NodeID, c.ClusterID, len(c.Members), c.PeerAddr, c.BindAddr)
	}
	if c.ObjStoreProvider != "s3" || c.ObjStoreBucket != "test-archive" || c.ObjStorePrefix != "test-fresh-prefix" || c.ObjStoreDurability != rhiza.ObjectStoreDurabilityBeforeAck {
		t.Fatal("object-store env did not merge with Local JSON")
	}
	if cfg.Database.DataDir != "/var/lib/grant/rhiza" || cfg.Database.LocalNodeRole != "voter" || cfg.Database.RuntimeMode != "embedded" {
		t.Fatal("Grant aliases changed the standalone runtime")
	}
	if c.DataDir != cfg.Database.DataDir {
		t.Fatal("restore/main effective DataDir differs from the rendered shared path")
	}
}
