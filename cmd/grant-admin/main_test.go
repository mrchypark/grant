package main

import (
	"strings"
	"testing"
)

func TestConfigFromEnvRequiresPublicConfigurationAndSetsDefaults(t *testing.T) {
	values := map[string]string{
		"GRANT_ADMIN_API_URL":                    "https://grant.example.test",
		"GRANT_ADMIN_MANAGEMENT_ORGANIZATION_ID": "org-admin",
		"GRANT_ADMIN_PUBLIC_URL":                 "https://admin.example.test",
		"GRANT_ADMIN_GOAUTHY_ISSUER":             "https://login.example.test",
		"GRANT_ADMIN_GOAUTHY_CLIENT_ID":          "grant-admin",
		"GRANT_ADMIN_GOAUTHY_CLIENT_SECRET":      "dummy-secret",
	}
	getenv := func(key string) string { return values[key] }
	cfg, err := configFromEnv(getenv)
	if err != nil {
		t.Fatal(err)
	}
	if cfg.bindAddr != "127.0.0.1:8095" || len(cfg.scopes) != 0 {
		t.Fatalf("unexpected defaults: bind=%q scopes=%q", cfg.bindAddr, cfg.scopes)
	}
	if cfg.resource != "" {
		t.Fatalf("omitted resource must remain empty, got %q", cfg.resource)
	}
	values["GRANT_ADMIN_GOAUTHY_RESOURCE"] = "https://grant.example.test/management"
	cfg, err = configFromEnv(getenv)
	if err != nil || cfg.resource != values["GRANT_ADMIN_GOAUTHY_RESOURCE"] {
		t.Fatalf("explicit resource not preserved: resource=%q err=%v", cfg.resource, err)
	}
	delete(values, "GRANT_ADMIN_PUBLIC_URL")
	if _, err = configFromEnv(getenv); err == nil || !strings.Contains(err.Error(), "GRANT_ADMIN_PUBLIC_URL") {
		t.Fatalf("expected missing public URL error, got %v", err)
	}
}
