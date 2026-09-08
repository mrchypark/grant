# Snapshot canonical oracle

Baseline c716877: [crates/app/src/bin/grant-patch-data-authority-snapshot.rs](https://github.com/Conalog/grant/blob/c716877e7b970b7d38392b377f64384936992884/crates/app/src/bin/grant-patch-data-authority-snapshot.rs) and [crates/infra/src/database/authority_snapshot_v2.rs::snapshot_digest](https://github.com/Conalog/grant/blob/c716877e7b970b7d38392b377f64384936992884/crates/infra/src/database/authority_snapshot_v2.rs::snapshot_digest).

The small SQLite fixture in main_test.go has organization o owned by user u, membership u→o, plant p owned by o, and all other source tables empty. The expected snapshot JSON here was independently transcribed from the baseline field/ordering rules; it is not produced by the Go exporter. No baseline Rust binary was executed.

The source watermark frames each UTF-8 string with an eight-byte big-endian length, in sequence: `o`, `1`, `user`, `u`, empty, `["o"]`, `o`, `u`, `p`, `o`. POSIX printf of those length-prefixed bytes piped to `shasum -a 256` gives `f3c71c19fa99588541db89a6aac38912c8876e1c6987c96e020b7e72069b506f`.

Rust converts the snapshot into serde_json::Value, removes snapshotDigest, and hashes compact recursively sorted JSON. Independently reproduce the frozen digest with:

```sh
jq -cSj . cmd/grant-patch-data-authority-snapshot/testdata/snapshot-simple.json | shasum -a 256
```

Expected `d7bb40927cf19e06e515bfdd562092031865531575ead14c455d68c8d89fa06b`. This fixture remains after Rust source retirement.
