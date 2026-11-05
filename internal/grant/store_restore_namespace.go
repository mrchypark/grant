package grant

import (
	"context"
	"encoding/json"
	"fmt"
	"net"
	"net/url"
	"os"
	"strings"
	"time"

	"cloud.google.com/go/storage"
	"github.com/Azure/azure-sdk-for-go/sdk/azcore"
	"github.com/Azure/azure-sdk-for-go/sdk/azidentity"
	"github.com/Azure/azure-sdk-for-go/sdk/storage/azblob"
	"github.com/minio/minio-go/v7"
	"github.com/mrchypark/rhiza"
	"google.golang.org/api/iterator"
	"google.golang.org/api/option"
)

// Vacancy is observational. The namespace must be reserved to one offline
// migration actor; neither List nor the local pending marker fences other writers.
func restoreNamespaceEmpty(ctx context.Context, c rhiza.Config) error {
	ctx, cancel := context.WithTimeout(ctx, 30*time.Second)
	defer cancel()
	occupied := func(name string) bool {
		return name == c.ObjStorePrefix || strings.HasPrefix(name, c.ObjStorePrefix+"/")
	}
	// Do not expose SDK errors: they can contain signed URLs and credentials.
	failure := func() error {
		if ctx.Err() != nil {
			return ctx.Err()
		}
		return fmt.Errorf("restore object namespace listing failed (%s)", c.ObjStoreProvider)
	}
	switch c.ObjStoreProvider {
	case "s3":
		if c.ObjStoreAccessKey == "" || c.ObjStoreSecretKey == "" {
			return fmt.Errorf("restore S3 listing requires explicit access and secret keys")
		}
		client, err := (&Store{cfg: Config{Database: DatabaseConfig{Rhiza: c}}}).backupS3()
		if err != nil {
			return failure()
		}
		for item := range client.ListObjects(ctx, c.ObjStoreBucket, minio.ListObjectsOptions{Prefix: c.ObjStorePrefix, Recursive: true}) {
			if item.Err != nil {
				return failure()
			}
			if occupied(item.Key) {
				return fmt.Errorf("restore object namespace is occupied")
			}
		}
	case "gcs":
		if c.ObjStoreEndpoint != "" || c.ObjStoreInsecure {
			return fmt.Errorf("restore GCS does not accept endpoint or insecure config overrides")
		}
		if err := restoreGCSEmulatorPreflight(c); err != nil {
			return err
		}
		var options []option.ClientOption
		if c.ObjStoreServiceAccount != "" {
			options = append(options, option.WithCredentialsJSON([]byte(c.ObjStoreServiceAccount)))
		}
		client, err := storage.NewClient(ctx, options...)
		if err != nil {
			return failure()
		}
		defer client.Close()
		objects := client.Bucket(c.ObjStoreBucket).Objects(ctx, &storage.Query{Prefix: c.ObjStorePrefix})
		for {
			item, err := objects.Next()
			if err == iterator.Done {
				break
			}
			if err != nil {
				return failure()
			}
			if occupied(item.Name) {
				return fmt.Errorf("restore object namespace is occupied")
			}
		}
	case "azure":
		client, err := restoreAzureClient(c)
		if err != nil {
			return failure()
		}
		pager := client.NewListBlobsFlatPager(c.ObjStoreBucket, &azblob.ListBlobsFlatOptions{Prefix: &c.ObjStorePrefix})
		for pager.More() {
			page, err := pager.NextPage(ctx)
			if err != nil {
				return failure()
			}
			if page.Segment == nil {
				continue
			}
			for _, item := range page.Segment.BlobItems {
				if item != nil && item.Name != nil && occupied(*item.Name) {
					return fmt.Errorf("restore object namespace is occupied")
				}
			}
		}
	default:
		return fmt.Errorf("unsupported restore object storage provider")
	}
	if ctx.Err() != nil {
		return ctx.Err()
	}
	return nil
}

func restoreGCSEmulatorPreflight(c rhiza.Config) error {
	emulator := os.Getenv("STORAGE_EMULATOR_HOST")
	if emulator == "" {
		return nil
	}
	if !strings.Contains(emulator, "://") {
		emulator = "http://" + emulator
	}
	u, err := url.Parse(emulator)
	if err != nil || (u.Scheme != "http" && u.Scheme != "https") || net.ParseIP(u.Hostname()) == nil || !net.ParseIP(u.Hostname()).IsLoopback() || u.Port() == "" || u.User != nil || u.RawQuery != "" || u.Fragment != "" || (u.Path != "" && u.Path != "/") {
		return fmt.Errorf("GCS restore emulator must use an explicit loopback origin")
	}
	raw := []byte(c.ObjStoreServiceAccount)
	if len(raw) == 0 {
		raw, err = os.ReadFile(os.Getenv("GOOGLE_APPLICATION_CREDENTIALS"))
		if err != nil {
			return fmt.Errorf("GCS restore emulator requires disposable ADC")
		}
	}
	var account struct {
		Type     string `json:"type"`
		TokenURI string `json:"token_uri"`
	}
	if len(raw) > 1<<20 || json.Unmarshal(raw, &account) != nil || account.Type != "service_account" {
		return fmt.Errorf("GCS restore emulator requires disposable service-account ADC")
	}
	token, err := url.Parse(account.TokenURI)
	if err != nil || token.Scheme != u.Scheme || token.Host != u.Host || token.Path != "/token" || token.User != nil || token.RawQuery != "" || token.Fragment != "" {
		return fmt.Errorf("GCS restore emulator ADC token URI must use the same loopback origin and /token")
	}
	return nil
}

func restoreAzureClient(c rhiza.Config) (*azblob.Client, error) {
	if c.ObjStoreAzureConnectionString != "" {
		if c.ObjStoreEndpoint != "" || c.ObjStoreAzureStorageAccountKey != "" || c.ObjStoreAzureTenantID != "" || c.ObjStoreAzureClientID != "" || c.ObjStoreAzureClientSecret != "" || c.ObjStoreAzureUserAssignedID != "" {
			return nil, fmt.Errorf("ambiguous Azure endpoint")
		}
		fields := map[string]string{}
		for _, entry := range strings.Split(c.ObjStoreAzureConnectionString, ";") {
			if entry == "" {
				continue
			}
			key, value, ok := strings.Cut(entry, "=")
			key = strings.ToLower(key)
			if _, duplicate := fields[key]; !ok || duplicate {
				return nil, fmt.Errorf("ambiguous Azure connection string")
			}
			fields[key] = value
		}
		if c.ObjStoreAzureStorageAccount != "" && fields["accountname"] != c.ObjStoreAzureStorageAccount {
			return nil, fmt.Errorf("Azure connection string account does not match configured account")
		}
		return azblob.NewClientFromConnectionString(c.ObjStoreAzureConnectionString, nil)
	}
	if c.ObjStoreAzureStorageAccount == "" || c.ObjStoreEndpoint != "" || c.ObjStoreInsecure {
		return nil, fmt.Errorf("explicit Azure storage account without endpoint override required")
	}
	endpoint := "https://" + c.ObjStoreAzureStorageAccount + ".blob.core.windows.net/"
	if c.ObjStoreAzureStorageAccountKey != "" {
		if c.ObjStoreAzureTenantID != "" || c.ObjStoreAzureClientID != "" || c.ObjStoreAzureClientSecret != "" || c.ObjStoreAzureUserAssignedID != "" {
			return nil, fmt.Errorf("ambiguous Azure credential")
		}
		key, err := azblob.NewSharedKeyCredential(c.ObjStoreAzureStorageAccount, c.ObjStoreAzureStorageAccountKey)
		if err != nil {
			return nil, err
		}
		return azblob.NewClientWithSharedKeyCredential(endpoint, key, nil)
	}
	var credential azcore.TokenCredential
	var err error
	if c.ObjStoreAzureTenantID != "" && c.ObjStoreAzureClientID != "" && c.ObjStoreAzureClientSecret != "" && c.ObjStoreAzureUserAssignedID == "" {
		credential, err = azidentity.NewClientSecretCredential(c.ObjStoreAzureTenantID, c.ObjStoreAzureClientID, c.ObjStoreAzureClientSecret, nil)
	} else if c.ObjStoreAzureUserAssignedID != "" && c.ObjStoreAzureTenantID == "" && c.ObjStoreAzureClientID == "" && c.ObjStoreAzureClientSecret == "" {
		credential, err = azidentity.NewManagedIdentityCredential(&azidentity.ManagedIdentityCredentialOptions{ID: azidentity.ClientID(c.ObjStoreAzureUserAssignedID)})
	} else {
		return nil, fmt.Errorf("explicit Azure credential required")
	}
	if err != nil {
		return nil, err
	}
	return azblob.NewClient(endpoint, credential, nil)
}
