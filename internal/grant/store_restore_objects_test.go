package grant

import (
	"context"
	"errors"
	"net"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"cloud.google.com/go/storage"
	"github.com/Azure/azure-sdk-for-go/sdk/azcore"
	"github.com/Azure/azure-sdk-for-go/sdk/storage/azblob"
	"github.com/minio/minio-go/v7"
	"github.com/minio/minio-go/v7/pkg/credentials"
	"github.com/mrchypark/rhiza"
	"google.golang.org/api/iterator"
	"google.golang.org/api/option"
)

// The opt-in owns pinned local emulators, never production cloud resources.
func TestSingleVoterRestoreObjectAdapters(t *testing.T) {
	if os.Getenv("GRANT_SINGLE_VOTER_RESTORE_CONTRACT") != "1" {
		t.Skip("UNVERIFIED: requires GRANT_SINGLE_VOTER_RESTORE_CONTRACT=1 and native local Docker")
	}
	if os.Getenv("GRANT_SINGLE_VOTER_RESTORE_SCOPE") == "" {
		if os.Getenv("GITHUB_ACTIONS") == "true" {
			t.Fatal("CI restore contract requires a run/attempt/job cleanup scope")
		}
		t.Setenv("GRANT_SINGLE_VOTER_RESTORE_SCOPE", "local-"+independentToken())
	}
	for _, image := range independentImages {
		t.Run(image.name, func(t *testing.T) {
			ctx, cancel := context.WithTimeout(context.Background(), 3*time.Minute)
			defer cancel()
			endpoint := restoreContractEmulator(t, ctx, image.name, image.image)
			cfg := singleVoterRestoreFixture()
			cfg.Database.DataDir = filepath.Join(t.TempDir(), "target")
			c := &cfg.Database.Rhiza
			c.ObjStoreProvider = image.name
			c.ObjStoreBucket = "restore-" + independentToken()
			c.ObjStorePrefix = "fresh/import"
			put, remove := restoreObjectFixture(t, ctx, c, endpoint)
			if err := restoreNamespaceEmpty(ctx, *c); err != nil {
				t.Fatalf("empty namespace: %v", err)
			}
			sibling := c.ObjStorePrefix + "-sibling/marker"
			put(sibling)
			if err := restoreNamespaceEmpty(ctx, *c); err != nil {
				t.Fatalf("unrelated namespace denied: %v", err)
			}
			marker := c.ObjStorePrefix + "/occupied"
			put(marker)
			snap := restoreFrozenSource(t, ctx)
			if err := RestoreStoreSnapshot(ctx, cfg, snap); err == nil {
				t.Fatal("occupied object namespace admitted")
			}
			if _, err := os.Lstat(cfg.Database.DataDir); !os.IsNotExist(err) {
				t.Fatal("occupied namespace touched the target directory")
			}
			remove(marker)
			udp, err := net.ListenPacket("udp", "127.0.0.1:0")
			if err != nil {
				t.Fatal(err)
			}
			c.PeerAddr = udp.LocalAddr().String()
			_ = udp.Close()
			c.Members[0].PeerURL = "quic://" + c.PeerAddr
			if err := RestoreStoreSnapshot(ctx, cfg, snap); err != nil {
				t.Fatalf("single-voter import: %v", err)
			}
			if _, err := os.Stat(filepath.Join(cfg.Database.DataDir, "grant-restore-pending.json")); !os.IsNotExist(err) {
				t.Fatal("successful restore left pending marker")
			}
			if _, err := os.Stat(filepath.Join(cfg.Database.DataDir, "grant-cutover-verified.json")); err != nil {
				t.Fatal("successful restore lacks verified marker")
			}
			// The offline seed is before-ack; steady state explicitly permits
			// batched async durability without reimporting or changing identity.
			c.ObjStoreDurability = rhiza.ObjectStoreDurabilityAsync
			// This delays periodic object sync only; it does not prove that
			// checkpoints or other paths performed no archive writes.
			c.ObjStoreSyncInterval = time.Hour
			voter, err := OpenStore(ctx, cfg)
			if err != nil {
				t.Fatal(err)
			}
			t.Cleanup(func() {
				if err := voter.Close(); err != nil {
					t.Error("voter cleanup failed")
				}
			})
			verifyRestoredTables(t, ctx, voter, snap)
			cfg.Database.ReadFenceKey = strings.Repeat("f", 40)
			voter.cfg.Database.ReadFenceKey = cfg.Database.ReadFenceKey
			fence := httptest.NewServer(voter.RuntimeHandler())
			t.Cleanup(fence.Close)
			member, err := rhiza.NewReplicaMember(c.ClusterID, c.Members[0])
			if err != nil {
				t.Fatal(err)
			}
			learnerCfg := cfg
			learnerCfg.Database.DataDir = t.TempDir()
			learnerCfg.Database.LocalNodeRole = "learner"
			learnerCfg.Database.RemoteNodes = []string{fence.URL}
			learnerCfg.Database.Rhiza.PeerToken = ""
			learnerCfg.Database.Replica = rhiza.ReplicaConfig{ClusterID: c.ClusterID, ReplicaID: "restore-reader", AdminToken: c.AdminToken, Members: []rhiza.ReplicaMember{member}, SyncInterval: time.Hour}
			learner, err := OpenStore(ctx, learnerCfg)
			if err != nil {
				t.Fatal(err)
			}
			t.Cleanup(func() {
				if err := learner.Close(); err != nil {
					t.Error("learner cleanup failed")
				}
			})
			svc := NewService(learnerCfg, learner)
			_, err = localReplicaRead(ctx, svc, func(readCtx context.Context) (bool, error) {
				rows, err := learner.Query(readCtx, ReadLocal, "SELECT display_name FROM users WHERE id='import-preserved'")
				if err == nil && (len(rows) != 1 || rowString(rows[0], "display_name") != "Preserved") {
					t.Fatal("passive learner lost imported identity")
				}
				return true, err
			})
			if err != nil {
				t.Fatal(err)
			}
			checkRole := func(readCtx context.Context) (bool, error) {
				decisions, err := learner.EvaluateUserActions(readCtx, []UserActionCheck{{UserID: "conalog-grant", OrganizationID: "conalog-grant", ProviderID: "grant", ActionID: "organization.group.manage", ResourceType: "organization", ResourceID: "conalog-grant", Consistency: ReadStrong}})
				if err != nil {
					return false, err
				}
				return decisions[0].Allowed, nil
			}
			if allowed, err := localReplicaRead(ctx, svc, checkRole); err != nil || !allowed {
				t.Fatal("restored owner authorization missing before async revocation")
			}
			now := nowString()
			if err := voter.Transaction(ctx, []Statement{
				{SQL: "INSERT INTO users(id,display_name,active,created_at) VALUES('async-next-owner','Async owner',1,?)", Args: []any{now}},
				{SQL: "INSERT INTO organization_memberships(id,organization_id,user_id,membership_role,status,created_at,updated_at) VALUES('async-owner-membership','conalog-grant','async-next-owner','owner','active',?,?)", Args: []any{now, now}},
				{SQL: "UPDATE organization_memberships SET membership_role='member',updated_at=? WHERE organization_id='conalog-grant' AND user_id='conalog-grant' AND membership_role='owner'", Args: []any{now}},
			}); err != nil {
				t.Fatal("async write/revocation failed")
			}
			ack, err := voter.db.Query(ctx, rhiza.QueryRequest{SQL: "SELECT 1", Consistency: rhiza.ConsistencyLinearizable})
			if err != nil || learner.replica.Status().AppliedSlot >= ack.AppliedSlot {
				t.Fatal("test did not establish a learner behind the acknowledged async write")
			}
			var observedSlot uint64
			allowed, err := localReplicaRead(ctx, svc, func(readCtx context.Context) (bool, error) {
				rows, err := learner.Query(readCtx, ReadLocal, "SELECT id FROM users WHERE id='async-next-owner'")
				if err != nil {
					return false, err
				}
				if len(rows) != 1 {
					t.Fatal("fresh fence missed the acknowledged async write")
				}
				allowed, err := checkRole(readCtx)
				if err == nil {
					observedSlot = readScope(readCtx).slot
				}
				return allowed, err
			})
			if err != nil || allowed || observedSlot < ack.AppliedSlot {
				t.Fatal("fresh learner fence failed to observe async write and revocation")
			}
			fence.Close()
			if _, err := localReplicaRead(ctx, svc, func(context.Context) (bool, error) { return true, nil }); ErrorStatus(err) != 503 {
				t.Fatal("disconnected fence did not fail closed")
			}
			if err := RestoreStoreSnapshot(ctx, cfg, snap); err == nil {
				t.Fatal("occupied directory/namespace was silently reset")
			}
			remove(sibling)
		})
	}
}

func restoreContractEmulator(t *testing.T, ctx context.Context, provider, image string) string {
	t.Helper()
	socket := os.Getenv("DOCKER_HOST")
	if socket == "" {
		socket = "unix:///var/run/docker.sock"
	}
	if !independentNativeSocket(socket) {
		t.Fatal("UNVERIFIED: only a known native local Docker socket is allowed")
	}
	dir, logs := t.TempDir(), t.TempDir()
	if err := os.Mkdir(filepath.Join(dir, "docker-config"), 0700); err != nil {
		t.Fatal(err)
	}
	f := &independentFixture{t: t, ctx: ctx, dir: dir, logs: logs, id: "grant-restore-" + independentToken()}
	f.env = []string{"PATH=" + os.Getenv("PATH"), "DOCKER_HOST=" + socket, "DOCKER_CONFIG=" + filepath.Join(dir, "docker-config")}
	platform, e1 := f.docker("version", "--format", "{{.Server.Platform.Name}}")
	operatingSystem, e2 := f.docker("info", "--format", "{{.OperatingSystem}}")
	if e1 != nil || e2 != nil || !independentNativeServer(strings.TrimSpace(string(platform)), strings.TrimSpace(string(operatingSystem))) {
		t.Fatal("UNVERIFIED: native Docker identity is unavailable")
	}
	scope := os.Getenv("GRANT_SINGLE_VOTER_RESTORE_SCOPE")
	t.Logf("provider=%s pinned image=%s scope=%s owner=%s", provider, image, scope, f.id)
	// Only safe identity metadata reaches CI logs; ADC and emulator logs do not.
	t.Cleanup(f.cleanup)
	if _, err := f.docker("image", "inspect", image); err != nil {
		if _, err := f.docker("pull", image); err != nil {
			t.Fatal("UNVERIFIED: pinned emulator image unavailable")
		}
	}
	if err := independentAdmitCreate(f.docker, f.id); err != nil {
		t.Fatal("emulator name/owner admission failed")
	}
	port := map[string]string{"s3": "4566", "gcs": "4588", "azure": "4577"}[provider]
	f.createAdmitted = true
	raw, err := f.docker("create", "--name", f.id, "--pull=never", "--label", "grant.independent.owner="+f.id, "--label", "io.grant.single-voter-restore="+scope, "--memory=384m", "--cpus=1", "--pids-limit=128", "-p", "127.0.0.1::"+port, image)
	if err != nil {
		t.Fatal("emulator create failed; owned identity reconciliation required")
	}
	f.cid = strings.TrimSpace(string(raw))
	t.Logf("owned restore emulator container=%s scope=%s", f.cid, scope)
	if evidence := os.Getenv("GRANT_SINGLE_VOTER_RESTORE_EVIDENCE"); evidence != "" {
		metadata := "scope=" + scope + "\nowner=" + f.id + "\ncontainer=" + f.cid + "\nimage=" + image + "\n"
		if err := os.WriteFile(filepath.Join(evidence, provider+"-container.txt"), []byte(metadata), 0600); err != nil {
			t.Fatal("cannot preserve owned container identity evidence")
		}
	}
	if _, err := f.docker("start", f.cid); err != nil {
		t.Fatal("emulator start failed")
	}
	raw, err = f.docker("port", f.cid, port+"/tcp")
	if err != nil {
		t.Fatal("emulator published port unavailable")
	}
	address := strings.TrimSpace(string(raw))
	host, _, err := net.SplitHostPort(address)
	if err != nil || host != "127.0.0.1" {
		t.Fatal("emulator was not isolated on loopback")
	}
	return "http://" + address
}

func restoreFrozenSource(t *testing.T, ctx context.Context) StoreSnapshot {
	t.Helper()
	root := t.TempDir()
	source, err := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: filepath.Join(root, "source"), Rhiza: rhiza.Config{Local: true, ClusterID: "source-local", NodeID: "source-local", ObjStoreProvider: "filesystem", ObjStoreDir: filepath.Join(root, "archive"), ObjStoreDurability: rhiza.ObjectStoreDurabilityBeforeAck}}})
	if err != nil {
		t.Fatal(err)
	}
	closed := false
	t.Cleanup(func() {
		if !closed {
			if err := source.Close(); err != nil {
				t.Error("source cleanup failed")
			}
		}
	})
	if err := source.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	if _, err := source.Exec(ctx, "INSERT INTO users(id,display_name,created_at) VALUES('import-preserved','Preserved','now')"); err != nil {
		t.Fatal(err)
	}
	mode := "user"
	if _, err := source.UpsertProviderPepClient(ctx, "grant", "restore-source-pep", "conalog-grant", UpsertProviderPepClient{Lifecycle: "disabled", AuthorizationMode: &mode}); err != nil {
		t.Fatal(err)
	}
	snap, err := source.ExportSnapshot(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if err := source.Close(); err != nil {
		t.Fatal(err)
	}
	closed = true
	return snap
}

func verifyRestoredTables(t *testing.T, ctx context.Context, voter *Store, snap StoreSnapshot) {
	t.Helper()
	got, err := voter.ExportSnapshot(ctx)
	if err != nil {
		t.Fatal(err)
	}
	for i, table := range got.Tables {
		want, err := restoredTableDigest(snap.Tables[i], snap.SchemaDigest)
		if err != nil || table.Digest != want {
			t.Fatalf("imported table %s changed", table.Name)
		}
	}
}

func restoreObjectFixture(t *testing.T, ctx context.Context, c *rhiza.Config, endpoint string) (func(string), func(string)) {
	t.Helper()
	fixture := &independentFixture{t: t, ctx: ctx, dir: t.TempDir(), logs: t.TempDir(), id: c.ObjStoreBucket}
	fixture.provision(c.ObjStoreProvider, endpoint)
	cleanupContext := func() (context.Context, context.CancelFunc) {
		return context.WithTimeout(context.Background(), 20*time.Second)
	}
	switch c.ObjStoreProvider {
	case "s3":
		c.ObjStoreEndpoint = strings.TrimPrefix(endpoint, "http://")
		c.ObjStoreInsecure = true
		c.ObjStoreRegion = "us-east-1"
		c.ObjStoreAccessKey, c.ObjStoreSecretKey = "fake", "fake"
		client, err := minio.New(c.ObjStoreEndpoint, &minio.Options{Creds: credentials.NewStaticV4("fake", "fake", ""), Secure: false, Region: c.ObjStoreRegion})
		if err != nil {
			t.Fatal("S3 fixture client failed")
		}
		t.Cleanup(func() {
			ctx, cancel := cleanupContext()
			defer cancel()
			for object := range client.ListObjects(ctx, c.ObjStoreBucket, minio.ListObjectsOptions{Recursive: true}) {
				if object.Err != nil || client.RemoveObject(ctx, c.ObjStoreBucket, object.Key, minio.RemoveObjectOptions{}) != nil {
					t.Error("S3 fixture object cleanup failed")
				}
			}
			if client.RemoveBucket(ctx, c.ObjStoreBucket) != nil {
				t.Error("S3 fixture bucket cleanup failed")
			} else if exists, err := client.BucketExists(ctx, c.ObjStoreBucket); err != nil || exists {
				t.Error("S3 fixture bucket absence unverified")
			}
		})
		return func(key string) {
				if _, err := client.PutObject(ctx, c.ObjStoreBucket, key, strings.NewReader("owned"), 5, minio.PutObjectOptions{}); err != nil {
					t.Fatal("S3 fixture write failed")
				}
			}, func(key string) {
				if client.RemoveObject(ctx, c.ObjStoreBucket, key, minio.RemoveObjectOptions{}) != nil {
					t.Fatal("S3 fixture delete failed")
				}
			}
	case "gcs":
		// Exercise the production omission path: both the vacancy SDK and
		// native voter/learner obtain ADC without an inline service-account key.
		c.ObjStoreServiceAccount = ""
		t.Setenv("GOOGLE_APPLICATION_CREDENTIALS", filepath.Join(fixture.dir, "fake-adc.json"))
		client, err := storage.NewClient(ctx, option.WithoutAuthentication())
		if err != nil {
			t.Fatal("GCS fixture client failed")
		}
		bucket := client.Bucket(c.ObjStoreBucket)
		t.Cleanup(func() {
			defer client.Close()
			ctx, cancel := cleanupContext()
			defer cancel()
			objects := bucket.Objects(ctx, nil)
			for {
				object, err := objects.Next()
				if err == iterator.Done {
					break
				}
				if err != nil {
					t.Error("GCS fixture list cleanup failed")
					break
				}
				if bucket.Object(object.Name).Delete(ctx) != nil {
					t.Error("GCS fixture object cleanup failed")
				}
			}
			if bucket.Delete(ctx) != nil {
				t.Error("GCS fixture bucket cleanup failed")
			} else if _, err := bucket.Attrs(ctx); !errors.Is(err, storage.ErrBucketNotExist) {
				t.Error("GCS fixture bucket absence unverified")
			}
		})
		return func(key string) {
				writer := bucket.Object(key).NewWriter(ctx)
				_, err := writer.Write([]byte("owned"))
				closeErr := writer.Close()
				if err != nil || closeErr != nil {
					t.Fatal("GCS fixture write failed")
				}
			}, func(key string) {
				if bucket.Object(key).Delete(ctx) != nil {
					t.Fatal("GCS fixture delete failed")
				}
			}
	case "azure":
		c.ObjStoreAzureStorageAccount = "devstoreaccount1"
		c.ObjStoreAzureConnectionString = independentAzureConnection(endpoint)
		client, err := azblob.NewClientFromConnectionString(c.ObjStoreAzureConnectionString, nil)
		if err != nil {
			t.Fatal("Azure fixture client failed")
		}
		t.Cleanup(func() {
			ctx, cancel := cleanupContext()
			defer cancel()
			if _, err := client.DeleteContainer(ctx, c.ObjStoreBucket, nil); err != nil {
				t.Error("Azure fixture container cleanup failed")
			} else {
				_, err := client.ServiceClient().NewContainerClient(c.ObjStoreBucket).GetProperties(ctx, nil)
				var response *azcore.ResponseError
				if !errors.As(err, &response) || response.StatusCode != 404 {
					t.Error("Azure fixture container absence unverified")
				}
			}
		})
		return func(key string) {
				if _, err := client.UploadBuffer(ctx, c.ObjStoreBucket, key, []byte("owned"), nil); err != nil {
					t.Fatal("Azure fixture write failed")
				}
			}, func(key string) {
				if _, err := client.DeleteBlob(ctx, c.ObjStoreBucket, key, nil); err != nil {
					t.Fatal("Azure fixture delete failed")
				}
			}
	}
	t.Fatal("unsupported emulator provider")
	return nil, nil
}
