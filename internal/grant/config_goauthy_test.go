package grant

import "testing"

func TestGoauthyConfig(t *testing.T) {
	for key, value := range map[string]string{
		"GRANT_MASTER_KEY": "master", "GRANT_APP_AUTH_KEY": "app", "GRANT_AUTH_PROVIDER": "goauthy",
		"GRANT_GOAUTHY_ISSUER": "https://gw-auth.dev.conalog.com", "GRANT_GOAUTHY_INTROSPECTION_CLIENT_ID": "introspector", "GRANT_GOAUTHY_INTROSPECTION_CLIENT_SECRET": "secret",
		"GRANT_GOAUTHY_EXPECTED_CLIENT_ID": "ied-login", "GRANT_GOAUTHY_RESOURCE": "https://grant.dev.conalog.com", "GRANT_GOAUTHY_APPLICATION_ID": "ied-management-data", "GRANT_GOAUTHY_REQUIRED_SCOPES": "openid goauthy.read",
	} {
		t.Setenv(key, value)
	}
	cfg, err := ConfigFromEnv()
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Auth.Goauthy == nil || cfg.Auth.Goauthy.Issuer != "https://gw-auth.dev.conalog.com" || cfg.Auth.Goauthy.ExpectedClientID != "ied-login" || len(cfg.Auth.Goauthy.RequiredScopes) != 2 {
		t.Fatalf("unexpected Goauthy config: %#v", cfg.Auth.Goauthy)
	}
}

func TestGoauthyConfigRejectsUnsafeOrIncompleteValues(t *testing.T) {
	for _, test := range []struct{ name, key, value string }{{"http issuer", "GRANT_GOAUTHY_ISSUER", "http://gw-auth.dev.conalog.com"}, {"query resource", "GRANT_GOAUTHY_RESOURCE", "https://grant.dev.conalog.com?x=1"}, {"empty scopes", "GRANT_GOAUTHY_REQUIRED_SCOPES", ""}, {"missing expected client", "GRANT_GOAUTHY_EXPECTED_CLIENT_ID", ""}} {
		t.Run(test.name, func(t *testing.T) {
			for key, value := range map[string]string{"GRANT_MASTER_KEY": "master", "GRANT_APP_AUTH_KEY": "app", "GRANT_AUTH_PROVIDER": "goauthy", "GRANT_GOAUTHY_ISSUER": "https://gw-auth.dev.conalog.com", "GRANT_GOAUTHY_INTROSPECTION_CLIENT_ID": "client", "GRANT_GOAUTHY_INTROSPECTION_CLIENT_SECRET": "secret", "GRANT_GOAUTHY_EXPECTED_CLIENT_ID": "login", "GRANT_GOAUTHY_RESOURCE": "https://grant.dev.conalog.com", "GRANT_GOAUTHY_APPLICATION_ID": "ied", "GRANT_GOAUTHY_REQUIRED_SCOPES": "openid"} {
				t.Setenv(key, value)
			}
			t.Setenv(test.key, test.value)
			if _, err := ConfigFromEnv(); err == nil {
				t.Fatal("unsafe Goauthy config accepted")
			}
		})
	}
}
