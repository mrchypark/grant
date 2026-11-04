package grant

import (
	"context"
	"errors"
	"fmt"

	"github.com/mrchypark/rhiza"
)

// RestoreLocalStorage reconciles the archive without starting Grant SQL or services.
// Async recovery restores archived history, not necessarily every acknowledged write.
func RestoreLocalStorage(ctx context.Context, cfg Config) error {
	d := cfg.Database
	c, err := effectiveRhizaConfig(d)
	if err != nil {
		return err
	}
	if !c.Local || (d.RuntimeMode != "" && d.RuntimeMode != "embedded") || (d.LocalNodeRole != "" && d.LocalNodeRole != "voter") || len(d.RemoteNodes) != 0 || len(d.Members) != 0 || len(c.Members) != 0 || c.Learner != nil || c.EnableReconfiguration || c.PeerAddr != "" || c.PeerToken != "" || c.AdminToken != "" || d.OperatorBindAddr != "" || c.BindAddr != "" || d.ClusterRestoreSnapshot != "" || len(d.Replica.Members) != 0 {
		return fmt.Errorf("restore-local-storage requires embedded Local mode without HA, remote, learner, Operator, or cluster restore configuration")
	}
	if c.ObjStoreProvider == "" || (c.ObjStoreDurability != rhiza.ObjectStoreDurabilityBeforeAck && c.ObjStoreDurability != rhiza.ObjectStoreDurabilityAsync) {
		return fmt.Errorf("restore-local-storage requires object storage and explicit async or before-ack durability")
	}
	if err := localStoragePreflight(cfg, false); err != nil {
		return err
	}
	if _, err := clusterRestorePreflight(ctx, cfg); err != nil {
		return err
	}
	if err := ctx.Err(); err != nil {
		return err
	}
	db, err := rhiza.Open(ctx, c)
	if err != nil {
		return fmt.Errorf("open local storage for recovery: %w", err)
	}
	closeErr := db.Close()
	if closeErr != nil {
		closeErr = fmt.Errorf("close local storage after recovery: %w", closeErr)
	}
	return errors.Join(ctx.Err(), closeErr)
}
