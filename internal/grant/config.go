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
type SecurityConfig struct{ MasterKey, AppAuthKey, HTTPCapabilityPEPApplicationID, BootstrapUserID, BootstrapUserName string }
type RauthyConfig struct {
	BaseURL, IntrospectPath, UserinfoPath, ClientID, ClientSecret, ClientReadAPIKey string
	CacheTTLSeconds                                                                 uint64
}
type PocketBaseConfig struct{ BaseURL, Collection, ApplicationID string }
type AuthConfig struct {
	ProviderMode string
	Rauthy       RauthyConfig
	PocketBase   *PocketBaseConfig
}
type DatabaseConfig struct {
	ClusterRestoreSnapshot                                                                                             string
	DataDir, NodeID, ClusterID, PeerAddr, RuntimeMode, LocalNodeRole, DefaultReadConsistency, LegacyDataDir, BackupDir string
	Members                                                                                                            []rhiza.Member
	Rhiza                                                                                                              rhiza.Config
	Replica                                                                                                            rhiza.ReplicaConfig
	RemoteNodes                                                                                                        []string
	RemoteAPISecret, RuntimeBindAddr, ReadFenceKey                                                                     string
}
type Config struct {
	Server   ServerConfig
	Security SecurityConfig
	Auth     AuthConfig
	Database DatabaseConfig
}

func envDefault(k, v string) string {
	if x, ok := os.LookupEnv(k); ok {
		return x
	}
	return v
}
func ConfigFromEnv() (Config, error) {
	c := Config{Server: ServerConfig{BindAddr: envDefault("GRANT_BIND_ADDR", "0.0.0.0:8080")}, Security: SecurityConfig{MasterKey: envDefault("GRANT_MASTER_KEY", "dev-only-master-key"), AppAuthKey: os.Getenv("GRANT_APP_AUTH_KEY"), HTTPCapabilityPEPApplicationID: strings.TrimSpace(os.Getenv("GRANT_HTTP_CAPABILITY_PEP_APPLICATION_ID")), BootstrapUserID: envDefault("GRANT_BOOTSTRAP_USER_ID", "conalog-grant"), BootstrapUserName: envDefault("GRANT_BOOTSTRAP_USER_NAME", "Conalog Grant")}}
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
		if c.Auth.Rauthy.ClientID == "" || c.Auth.Rauthy.ClientSecret == "" || c.Auth.Rauthy.ClientReadAPIKey == "" {
			return c, fmt.Errorf("Rauthy client ID, client secret and client read API key are required")
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
	if d.RuntimeBindAddr != "" || (len(d.RemoteNodes) > 0 && d.LocalNodeRole != "learner") {
		if len(d.RemoteAPISecret) < 32 || d.RemoteAPISecret == c.Security.MasterKey || d.RemoteAPISecret == c.Security.AppAuthKey {
			return fmt.Errorf("runtime transport needs distinct GRANT_REMOTE_API_SECRET of at least 32 bytes")
		}
	}
	if d.ReadFenceKey != "" && (len(d.ReadFenceKey) < 32 || d.ReadFenceKey == d.RemoteAPISecret || d.ReadFenceKey == c.Security.MasterKey || d.ReadFenceKey == c.Security.AppAuthKey) {
		return fmt.Errorf("read fence key must be distinct and at least 32 bytes")
	}
	if d.LocalNodeRole == "learner" && (len(d.RemoteNodes) == 0 || len(d.ReadFenceKey) < 32 || d.RuntimeBindAddr != "" || d.RemoteAPISecret != "") {
		return fmt.Errorf("learner requires private read-fence transport, without runtime writer credentials/listener")
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
	if raw := os.Getenv("GRANT_CLUSTER_MEMBERS"); raw != "" {
		if e := json.Unmarshal([]byte(raw), &d.Members); e != nil {
			return e
		}
	}
	if raw := os.Getenv("GRANT_RHIZA_CONFIG_JSON"); raw != "" {
		if e := json.Unmarshal([]byte(raw), &d.Rhiza); e != nil {
			return e
		}
	}
	if raw := os.Getenv("GRANT_RHIZA_REPLICA_CONFIG_JSON"); raw != "" {
		if e := json.Unmarshal([]byte(raw), &d.Replica); e != nil {
			return e
		}
	}
	if d.LocalNodeRole == "learner" {
		if len(d.Replica.Members) == 0 {
			return fmt.Errorf("learner requires public replica members")
		}
		for _, members := range [][]rhiza.Member{d.Members, d.Rhiza.Members} {
			for _, member := range members {
				if member.Token != "" {
					return fmt.Errorf("learner must not receive voter member tokens")
				}
			}
		}
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
