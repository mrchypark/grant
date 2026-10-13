package grant

import (
	"encoding/json"
	"fmt"
	"github.com/mrchypark/rhiza"
	"net/url"
	"os"
	"strconv"
	"strings"
)

type ReadConsistency = string

const (
	ReadLocal        = "local"
	ReadStrong       = "strong"
	ReadPreferStrong = "prefer-strong"
)

type ServerConfig struct{ BindAddr string }
type SecurityConfig struct{ MasterKey, AppAuthKey, HTTPCapabilityPEPApplicationID, ManagementOrganizationID, BootstrapUserID, BootstrapUserName string }
type RauthyConfig struct {
	BaseURL, IntrospectPath, UserinfoPath, ClientID, ClientSecret, ClientReadAPIKey string
	CacheTTLSeconds                                                                 uint64
}
type PocketBaseConfig struct{ BaseURL, Collection, ApplicationID string }
type GoauthyConfig struct {
	Issuer, IntrospectionClientID, IntrospectionClientSecret, ExpectedClientID, Resource, ApplicationID string
	RequiredScopes                                                                                      []string
}
type AuthConfig struct {
	ProviderMode string
	Rauthy       RauthyConfig
	PocketBase   *PocketBaseConfig
	Goauthy      *GoauthyConfig
}
type DatabaseConfig struct {
	ClusterRestoreSnapshot                                                                                             string
	DataDir, NodeID, ClusterID, PeerAddr, RuntimeMode, LocalNodeRole, DefaultReadConsistency, LegacyDataDir, BackupDir string
	Members                                                                                                            []rhiza.Member
	Rhiza                                                                                                              rhiza.Config
	Replica                                                                                                            rhiza.ReplicaConfig
	RemoteNodes                                                                                                        []string
	RemoteAPISecret, RuntimeBindAddr, OperatorBindAddr, ReadFenceKey                                                   string
}
type Config struct {
	Server   ServerConfig
	Security SecurityConfig
	Auth     AuthConfig
	Database DatabaseConfig
}

type configuredMember struct {
	rhiza.Member
	Token string `json:"token"`
}

func envDefault(k, v string) string {
	if x, ok := os.LookupEnv(k); ok {
		return x
	}
	return v
}
func ConfigFromEnv() (Config, error) {
	c := Config{Server: ServerConfig{BindAddr: envDefault("GRANT_BIND_ADDR", "0.0.0.0:8080")}, Security: SecurityConfig{MasterKey: envDefault("GRANT_MASTER_KEY", "dev-only-master-key"), AppAuthKey: os.Getenv("GRANT_APP_AUTH_KEY"), HTTPCapabilityPEPApplicationID: strings.TrimSpace(os.Getenv("GRANT_HTTP_CAPABILITY_PEP_APPLICATION_ID")), ManagementOrganizationID: strings.TrimSpace(os.Getenv("GRANT_MANAGEMENT_ORGANIZATION_ID")), BootstrapUserID: envDefault("GRANT_BOOTSTRAP_USER_ID", "conalog-grant"), BootstrapUserName: envDefault("GRANT_BOOTSTRAP_USER_NAME", "Conalog Grant")}}
	if c.Security.AppAuthKey == "" || c.Security.AppAuthKey == c.Security.MasterKey {
		return c, fmt.Errorf("GRANT_APP_AUTH_KEY is required and must differ from GRANT_MASTER_KEY")
	}
	c.Auth.ProviderMode = envDefault("GRANT_AUTH_PROVIDER", "rauthy")
	c.Auth.Rauthy = RauthyConfig{BaseURL: envDefault("GRANT_RAUTHY_BASE_URL", "http://127.0.0.1:8080"), IntrospectPath: envDefault("GRANT_RAUTHY_INTROSPECT_PATH", "/auth/v1/oidc/introspect"), UserinfoPath: envDefault("GRANT_RAUTHY_USERINFO_PATH", "/auth/v1/oidc/userinfo"), ClientID: os.Getenv("GRANT_RAUTHY_CLIENT_ID"), ClientSecret: os.Getenv("GRANT_RAUTHY_CLIENT_SECRET"), ClientReadAPIKey: os.Getenv("GRANT_RAUTHY_CLIENT_READ_API_KEY")}
	var err error
	c.Auth.Rauthy.CacheTTLSeconds, err = strconv.ParseUint(envDefault("GRANT_RAUTHY_CACHE_TTL_SECONDS", "30"), 10, 64)
	if err != nil {
		return c, err
	}
	switch c.Auth.ProviderMode {
	case "rauthy":
		if c.Auth.Rauthy.ClientID == "" || c.Auth.Rauthy.ClientSecret == "" {
			return c, fmt.Errorf("Rauthy client ID and client secret are required")
		}
	case "pocketbase":
		p := &PocketBaseConfig{BaseURL: os.Getenv("GRANT_POCKETBASE_BASE_URL"), Collection: envDefault("GRANT_POCKETBASE_COLLECTION", "users"), ApplicationID: os.Getenv("GRANT_POCKETBASE_APPLICATION_ID")}
		u, e := url.Parse(p.BaseURL)
		if e != nil || u.Host == "" || (u.Scheme != "http" && u.Scheme != "https") || u.User != nil || u.RawQuery != "" || u.Fragment != "" || (u.Path != "" && u.Path != "/") || p.ApplicationID == "" {
			return c, fmt.Errorf("invalid PocketBase origin/application configuration")
		}
		p.BaseURL, e = canonicalPocketBaseOrigin(p.BaseURL)
		if e != nil || !pocketBaseID(p.Collection) {
			return c, fmt.Errorf("invalid PocketBase origin/collection configuration")
		}
		c.Auth.PocketBase = p
	case "goauthy":
		g := &GoauthyConfig{Issuer: strings.TrimSpace(os.Getenv("GRANT_GOAUTHY_ISSUER")), IntrospectionClientID: strings.TrimSpace(os.Getenv("GRANT_GOAUTHY_INTROSPECTION_CLIENT_ID")), IntrospectionClientSecret: os.Getenv("GRANT_GOAUTHY_INTROSPECTION_CLIENT_SECRET"), ExpectedClientID: strings.TrimSpace(os.Getenv("GRANT_GOAUTHY_EXPECTED_CLIENT_ID")), Resource: strings.TrimSpace(os.Getenv("GRANT_GOAUTHY_RESOURCE")), ApplicationID: strings.TrimSpace(os.Getenv("GRANT_GOAUTHY_APPLICATION_ID")), RequiredScopes: strings.Fields(os.Getenv("GRANT_GOAUTHY_REQUIRED_SCOPES"))}
		var e error
		g.Issuer, e = canonicalGoauthyOrigin(g.Issuer)
		if e != nil || g.IntrospectionClientID == "" || g.IntrospectionClientSecret == "" || g.Resource == "" || g.ApplicationID == "" || len(g.RequiredScopes) == 0 {
			return c, fmt.Errorf("invalid Goauthy configuration")
		}
		g.Resource, e = canonicalGoauthyResource(g.Resource)
		if e != nil || g.ExpectedClientID == "" {
			return c, fmt.Errorf("invalid Goauthy configuration")
		}
		seen := map[string]bool{}
		for _, scope := range g.RequiredScopes {
			if scope == "" || seen[scope] {
				return c, fmt.Errorf("invalid Goauthy required scopes")
			}
			seen[scope] = true
		}
		c.Auth.Goauthy = g
	default:
		return c, fmt.Errorf("invalid GRANT_AUTH_PROVIDER")
	}
	c.Database = DatabaseConfig{DataDir: envDefault("GRANT_DATA_DIR", "./rhiza-data"), NodeID: envDefault("GRANT_NODE_ID", "grant-1"), ClusterID: envDefault("GRANT_CLUSTER_ID", "grant"), PeerAddr: envDefault("GRANT_PEER_ADDR", "127.0.0.1:0"), RuntimeMode: envDefault("GRANT_RUNTIME_MODE", "embedded"), LocalNodeRole: envDefault("GRANT_LOCAL_NODE_ROLE", "voter"), DefaultReadConsistency: envDefault("GRANT_READ_CONSISTENCY", ReadLocal), LegacyDataDir: os.Getenv("HQL_DATA_DIR"), BackupDir: envDefault("GRANT_BACKUP_DIR", "./grant-backups")}
	c.Database.ClusterRestoreSnapshot = os.Getenv("GRANT_CLUSTER_RESTORE_SNAPSHOT")
	switch c.Database.DefaultReadConsistency {
	case ReadLocal, ReadStrong, ReadPreferStrong:
	default:
		return c, fmt.Errorf("invalid read consistency")
	}
	if err := loadDatabaseConfig(&c); err != nil {
		return c, err
	}
	return c, nil
}

func loadDatabaseConfig(c *Config) error {
	if os.Getenv("RHIZA_BIND_ADDR") != "" {
		return loadOperatorDatabaseConfig(c)
	}
	d := &c.Database
	d.NodeID = envDefault("GRANT_NODE_ID", envDefault("POD_NAME", "grant-1"))
	d.RemoteAPISecret = os.Getenv("GRANT_REMOTE_API_SECRET")
	d.ReadFenceKey = os.Getenv("GRANT_READ_FENCE_KEY")
	d.RuntimeBindAddr = os.Getenv("GRANT_RUNTIME_BIND_ADDR")
	for _, v := range strings.Split(os.Getenv("GRANT_REMOTE_NODES"), ",") {
		if v = strings.TrimSpace(v); v != "" {
			if !strings.Contains(v, "://") {
				v = "http://" + v
			}
			u, e := url.Parse(v)
			if e != nil || u.Host == "" || u.User != nil || (u.Scheme != "http" && u.Scheme != "https") || u.RawQuery != "" || u.Fragment != "" {
				return fmt.Errorf("invalid GRANT_REMOTE_NODES")
			}
			d.RemoteNodes = append(d.RemoteNodes, strings.TrimRight(v, "/"))
		}
	}
	if err := validateRuntimeConfig(*d, c.Security); err != nil {
		return err
	}
	switch d.RuntimeMode {
	case "embedded", "remote", "split":
	default:
		return fmt.Errorf("invalid runtime mode")
	}
	if (d.RuntimeMode == "remote" || d.RuntimeMode == "split") && len(d.RemoteNodes) == 0 {
		return fmt.Errorf("remote/split needs GRANT_REMOTE_NODES")
	}
	switch d.LocalNodeRole {
	case "voter", "learner":
	default:
		return fmt.Errorf("invalid local node role")
	}
	var clusterMembers []configuredMember
	clusterMembersSet := os.Getenv("GRANT_CLUSTER_MEMBERS") != ""
	if raw := os.Getenv("GRANT_CLUSTER_MEMBERS"); raw != "" {
		if e := json.Unmarshal([]byte(raw), &clusterMembers); e != nil {
			return e
		}
	}
	var rhizaMembers []configuredMember
	rhizaMembersSet := false
	if raw := os.Getenv("GRANT_RHIZA_CONFIG_JSON"); raw != "" {
		if e := json.Unmarshal([]byte(raw), &d.Rhiza); e != nil {
			return e
		}
		var fields map[string]json.RawMessage
		if e := json.Unmarshal([]byte(raw), &fields); e != nil {
			return e
		}
		for name, value := range fields {
			if strings.EqualFold(name, "members") {
				rhizaMembersSet = true
				if e := json.Unmarshal(value, &rhizaMembers); e != nil {
					return e
				}
			}
		}
	}
	if raw := os.Getenv("GRANT_RHIZA_REPLICA_CONFIG_JSON"); raw != "" {
		if e := json.Unmarshal([]byte(raw), &d.Replica); e != nil {
			return e
		}
	}
	if d.LocalNodeRole == "learner" {
		if d.Rhiza.PeerToken != "" || os.Getenv("RHIZA_PEER_TOKEN") != "" {
			return fmt.Errorf("learner must not receive a voter peer token")
		}
		if len(d.Replica.Members) == 0 {
			return fmt.Errorf("learner requires public replica members")
		}
	}
	effective, err := effectiveRhizaConfig(*d)
	if err != nil {
		return err
	}
	peerToken := d.Rhiza.PeerToken
	if token := os.Getenv("RHIZA_PEER_TOKEN"); token != "" {
		if peerToken != "" && peerToken != token {
			return fmt.Errorf("RHIZA_PEER_TOKEN does not match configured peer token")
		}
		peerToken = token
	}
	if clusterMembersSet {
		d.Members, peerToken, err = publicMembers(clusterMembers, effective.ClusterID, effective.NodeID, peerToken, d.LocalNodeRole == "learner")
		if err != nil {
			return err
		}
	}
	if rhizaMembersSet {
		d.Rhiza.Members, peerToken, err = publicMembers(rhizaMembers, effective.ClusterID, effective.NodeID, peerToken, d.LocalNodeRole == "learner")
		if err != nil {
			return err
		}
	}
	d.Rhiza.PeerToken = peerToken
	if _, err := effectiveRhizaConfig(*d); err != nil {
		return err
	}
	r := &d.Rhiza
	for _, v := range []struct {
		k string
		p *string
	}{{"PROVIDER", &r.ObjStoreProvider}, {"ENDPOINT", &r.ObjStoreEndpoint}, {"BUCKET", &r.ObjStoreBucket}, {"DIR", &r.ObjStoreDir}, {"PREFIX", &r.ObjStorePrefix}, {"REGION", &r.ObjStoreRegion}, {"ACCESS_KEY", &r.ObjStoreAccessKey}, {"SECRET_KEY", &r.ObjStoreSecretKey}, {"SESSION_TOKEN", &r.ObjStoreSessionToken}} {
		if value, ok := os.LookupEnv("RHIZA_OBJSTORE_" + v.k); ok {
			*v.p = value
		}
	}
	if raw := os.Getenv("RHIZA_OBJSTORE_DURABILITY"); raw != "" {
		switch raw {
		case "async":
			r.ObjStoreDurability = rhiza.ObjectStoreDurabilityAsync
		case "before-ack":
			r.ObjStoreDurability = rhiza.ObjectStoreDurabilityBeforeAck
		default:
			return fmt.Errorf("invalid object store durability")
		}
	}
	if raw := os.Getenv("RHIZA_OBJSTORE_INSECURE"); raw != "" {
		v, e := strconv.ParseBool(raw)
		if e != nil {
			return e
		}
		r.ObjStoreInsecure = v
	}
	if strings.Contains(r.ObjStoreEndpoint, "://") {
		u, e := url.Parse(r.ObjStoreEndpoint)
		if e != nil || u.Host == "" || u.User != nil || (u.Path != "" && u.Path != "/") || u.RawQuery != "" || u.Fragment != "" || (u.Scheme != "http" && u.Scheme != "https") {
			return fmt.Errorf("object-store endpoint must be an HTTP(S) origin")
		}
		r.ObjStoreEndpoint = u.Host
		if u.Scheme == "http" {
			r.ObjStoreInsecure = true
		}
	}
	return nil
}

func validateRuntimeConfig(d DatabaseConfig, security SecurityConfig) error {
	if d.RuntimeBindAddr != "" || (len(d.RemoteNodes) > 0 && d.LocalNodeRole != "learner") {
		if len(d.RemoteAPISecret) < 32 || d.RemoteAPISecret == security.MasterKey || d.RemoteAPISecret == security.AppAuthKey {
			return fmt.Errorf("runtime transport needs distinct GRANT_REMOTE_API_SECRET of at least 32 bytes")
		}
	}
	if d.ReadFenceKey != "" && (len(d.ReadFenceKey) < 32 || d.ReadFenceKey == d.RemoteAPISecret || d.ReadFenceKey == security.MasterKey || d.ReadFenceKey == security.AppAuthKey) {
		return fmt.Errorf("read fence key must be distinct and at least 32 bytes")
	}
	if d.LocalNodeRole == "learner" && (len(d.RemoteNodes) == 0 || len(d.ReadFenceKey) < 32 || d.RuntimeBindAddr != "" || d.RemoteAPISecret != "") {
		return fmt.Errorf("learner requires private read-fence transport, without runtime writer credentials/listener")
	}
	return nil
}

// loadOperatorDatabaseConfig uses the generation-owned RHIZA_* environment.
// GRANT's older topology aliases are rejected so a stale Deployment value
// cannot silently replace a generation written by the Rhiza operator.
func loadOperatorDatabaseConfig(c *Config) error {
	for _, key := range []string{
		"GRANT_DATA_DIR", "GRANT_NODE_ID", "GRANT_CLUSTER_ID", "GRANT_PEER_ADDR",
		"GRANT_CLUSTER_MEMBERS", "GRANT_RHIZA_CONFIG_JSON", "GRANT_RHIZA_REPLICA_CONFIG_JSON",
		"GRANT_REMOTE_NODES", "GRANT_CLUSTER_RESTORE_SNAPSHOT",
	} {
		if os.Getenv(key) != "" {
			return fmt.Errorf("%s conflicts with operator-managed RHIZA_* configuration", key)
		}
	}
	if mode := os.Getenv("GRANT_RUNTIME_MODE"); mode != "" && mode != "embedded" {
		return fmt.Errorf("GRANT_RUNTIME_MODE conflicts with operator-managed embedded Rhiza")
	}
	if role := os.Getenv("GRANT_LOCAL_NODE_ROLE"); role != "" && role != "voter" {
		return fmt.Errorf("GRANT_LOCAL_NODE_ROLE conflicts with operator-managed Rhiza learner")
	}

	r, err := rhiza.ConfigFromEnv()
	if err != nil {
		return fmt.Errorf("operator Rhiza environment: %w", err)
	}
	if err := validateOperatorRhizaConfig(r, c.Security); err != nil {
		return err
	}
	c.Database.DataDir = r.DataDir
	c.Database.NodeID = r.NodeID
	c.Database.ClusterID = r.ClusterID
	c.Database.PeerAddr = r.PeerAddr
	c.Database.RuntimeMode = "embedded"
	c.Database.LocalNodeRole = "voter"
	c.Database.Rhiza = r
	c.Database.OperatorBindAddr = r.BindAddr
	c.Database.RemoteAPISecret = os.Getenv("GRANT_REMOTE_API_SECRET")
	c.Database.ReadFenceKey = os.Getenv("GRANT_READ_FENCE_KEY")
	c.Database.RuntimeBindAddr = os.Getenv("GRANT_RUNTIME_BIND_ADDR")
	if err := validateRuntimeConfig(c.Database, c.Security); err != nil {
		return err
	}
	for _, token := range []string{c.Database.RemoteAPISecret, c.Database.ReadFenceKey} {
		if token != "" && (token == r.PeerToken || token == r.AdminToken) {
			return fmt.Errorf("Grant runtime credentials must differ from Rhiza credentials")
		}
	}
	return nil
}

func validateOperatorRhizaConfig(r rhiza.Config, security SecurityConfig) error {
	if r.BindAddr == "" {
		return fmt.Errorf("RHIZA_BIND_ADDR is required for operator-managed Rhiza")
	}
	if len(r.Members) != 3 {
		return fmt.Errorf("operator-managed Rhiza requires exactly three voter members")
	}
	seen := make(map[string]bool, len(r.Members))
	for _, member := range r.Members {
		id := string(member.ID)
		if id == "" || member.PeerURL == "" || member.PublicKey == (rhiza.PublicKey{}) || seen[id] {
			return fmt.Errorf("operator-managed Rhiza has invalid voter membership")
		}
		seen[id] = true
	}
	if !seen[r.NodeID] && r.Learner == nil {
		return fmt.Errorf("RHIZA_NODE_ID is not in operator voter membership")
	}
	if r.Learner == nil && os.Getenv("RHIZA_PEER_TOKENS") == "" {
		return fmt.Errorf("three-voter operator-managed Rhiza requires RHIZA_PEER_TOKENS")
	}
	for _, token := range []struct{ name, value string }{
		{"RHIZA_PEER_TOKENS/RHIZA_PEER_TOKEN", r.PeerToken},
		{"RHIZA_ADMIN_TOKEN", r.AdminToken},
	} {
		if len(token.value) < 32 {
			return fmt.Errorf("%s must be at least 32 bytes", token.name)
		}
		if token.value == security.MasterKey || token.value == security.AppAuthKey {
			return fmt.Errorf("%s must differ from Grant business credentials", token.name)
		}
	}
	if r.PeerToken == r.AdminToken {
		return fmt.Errorf("RHIZA peer and admin tokens must differ")
	}
	return nil
}

func publicMembers(input []configuredMember, clusterID, localID, peerToken string, learner bool) ([]rhiza.Member, string, error) {
	if input == nil {
		return nil, peerToken, nil
	}
	members := make([]rhiza.Member, 0, len(input))
	for _, in := range input {
		member := in.Member
		if in.Token != "" {
			if learner {
				return nil, "", fmt.Errorf("learner must not receive voter member tokens")
			}
			key := rhiza.PeerPublicKey(clusterID, string(member.ID), in.Token)
			if member.PublicKey != (rhiza.PublicKey{}) && member.PublicKey != key {
				return nil, "", fmt.Errorf("member public key does not match its token")
			}
			member.PublicKey = key
			if string(member.ID) == localID {
				if peerToken != "" && peerToken != in.Token {
					return nil, "", fmt.Errorf("local peer token does not match membership")
				}
				peerToken = in.Token
			}
		}
		if string(member.ID) == localID && peerToken != "" && rhiza.PeerPublicKey(clusterID, localID, peerToken) != member.PublicKey {
			return nil, "", fmt.Errorf("local peer token does not match membership")
		}
		members = append(members, member)
	}
	return members, peerToken, nil
}
