package grant

import (
	"fmt"
	"net/url"
	"path"
	"reflect"
	"strings"

	"github.com/mrchypark/rhiza"
)

// This admission is only for the offline seed, not steady-state durability or
// lost-WAL recovery. Normal OpenStore may use async after verified seed/close.
func singleVoterRestoreConfig(cfg Config, snap StoreSnapshot) (rhiza.Config, error) {
	d := cfg.Database
	c, err := effectiveRhizaConfig(d)
	if err != nil {
		return c, err
	}
	if (d.RuntimeMode != "" && d.RuntimeMode != "embedded") || (d.LocalNodeRole != "" && d.LocalNodeRole != "voter") || len(d.RemoteNodes) != 0 || len(d.Replica.Members) != 0 || d.RuntimeBindAddr != "" || d.OperatorBindAddr != "" || d.ClusterRestoreSnapshot != "" || c.Local || c.Learner != nil || c.EnableReconfiguration || c.BindAddr != "" {
		return c, fmt.Errorf("single-voter import requires isolated non-Local embedded voter configuration")
	}
	if len(c.Members) != 1 || (len(d.Members) != 0 && !reflect.DeepEqual(d.Members, c.Members)) {
		return c, fmt.Errorf("single-voter import requires exactly one explicit, consistent voter member")
	}
	if strings.TrimSpace(c.ClusterID) == "" || c.ClusterID != strings.TrimSpace(c.ClusterID) || c.ClusterID == snap.ClusterID || (snap.Backend == "rhiza" && snap.ClusterID == "") || (d.ClusterID != "" && d.ClusterID != c.ClusterID) {
		return c, fmt.Errorf("single-voter import requires a new explicit cluster identity")
	}
	member := c.Members[0]
	if (d.NodeID == "" && d.Rhiza.NodeID == "") || c.NodeID == "" || strings.TrimSpace(c.NodeID) != c.NodeID || string(member.ID) != c.NodeID || (d.NodeID != "" && d.NodeID != c.NodeID) || c.PeerToken == "" || c.PeerAddr == "" || member.PublicKey != rhiza.PeerPublicKey(c.ClusterID, c.NodeID, c.PeerToken) {
		return c, fmt.Errorf("single-voter import requires a fixed matching node, peer token and public member key")
	}
	peer, err := url.Parse(member.PeerURL)
	if err != nil || peer.Scheme != "quic" || peer.Hostname() == "" || peer.Hostname() == "0.0.0.0" || peer.Hostname() == "::" || peer.Port() == "" || peer.User != nil || peer.RawQuery != "" || peer.Fragment != "" || peer.Path != "" {
		return c, fmt.Errorf("single-voter import requires an explicit advertised QUIC member address")
	}
	if c.AdminToken == "" || c.AdminToken == c.PeerToken || c.AdminToken == cfg.Security.MasterKey || c.AdminToken == cfg.Security.AppAuthKey || c.AdminToken == d.ReadFenceKey || c.AdminToken == d.RemoteAPISecret || c.PeerToken == cfg.Security.MasterKey || c.PeerToken == cfg.Security.AppAuthKey {
		return c, fmt.Errorf("single-voter import requires separate peer and sync credentials")
	}
	if c.ObjStoreProvider != "s3" && c.ObjStoreProvider != "gcs" && c.ObjStoreProvider != "azure" {
		return c, fmt.Errorf("single-voter import requires S3, GCS or Azure object storage")
	}
	if strings.TrimSpace(c.ObjStoreBucket) == "" || c.ObjStoreBucket != strings.TrimSpace(c.ObjStoreBucket) || c.ObjStoreDir != "" || c.ObjStoreDurability != rhiza.ObjectStoreDurabilityBeforeAck {
		return c, fmt.Errorf("single-voter import requires an explicit bucket and before-ack durability")
	}
	prefix := c.ObjStorePrefix
	if prefix == "" || prefix != strings.TrimSpace(prefix) || prefix != strings.Trim(prefix, "/") || path.Clean(prefix) != prefix || prefix == "." || prefix == ".." || strings.HasPrefix(prefix, "../") || strings.ContainsAny(prefix, "\\\r\n\x00") {
		return c, fmt.Errorf("single-voter import requires a fresh dedicated canonical object prefix")
	}
	return c, nil
}
