# Grant standalone Local overlay

Follow the [single-node guide](../../../docs/single-node.md) for mandatory local
image publication, external auth/storage secrets, retained disk and writer exclusion.
Both containers target `ghcr.io/mrchypark/grant:v0.4.1` (`IfNotPresent`), with Rhiza v0.21.2.
This published image does not contain the source's Rhiza v0.22.0 integration
candidate. No candidate image or rollout is qualified; v0.3.0 does not contain
the required command.

The single init runs `/usr/local/bin/grant restore-local-storage`: public Rhiza
Open/Close only, with the same config and DataDir as main, no application SQL or
services. Main reopens and validates the archive binding. Local has one stable
identity and no peers; Grant's `voter` role selects its writable API path.

Run `scripts/verify-standalone-deployment.sh` from the repository root.
No cluster deployment or published image release is authorized by this overlay.
