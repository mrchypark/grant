package grant

import (
	"testing"
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
