use rusqlite::Connection;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, process::Command};

#[test]
fn exports_one_immutable_atomic_bundle_from_fixed_read_only_queries() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("data.db");
    let connection = Connection::open(&database).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE organizations(id TEXT PRIMARY KEY,owner TEXT);
         CREATE TABLE users(id TEXT PRIMARY KEY,deleted TEXT,organizations TEXT);
         CREATE TABLE viewers(id TEXT PRIMARY KEY,deleted TEXT,organizations TEXT);
         CREATE TABLE temporary_accounts(id TEXT PRIMARY KEY,expired TEXT,organization TEXT);
         CREATE TABLE service_principals(id TEXT PRIMARY KEY,enabled INTEGER);
         CREATE TABLE org_parent_map(organization TEXT,parent TEXT);
         CREATE TABLE plants(id TEXT PRIMARY KEY,organization TEXT,isArchive INTEGER);
         CREATE TABLE permissions(plant TEXT,users TEXT,viewers TEXT,temporary TEXT);
         INSERT INTO organizations VALUES('org-a','user-a'),('org-b','');
         INSERT INTO users VALUES('user-a','','[\"org-a\"]');
         INSERT INTO viewers VALUES('viewer-a','','[\"org-b\"]');
         INSERT INTO temporary_accounts VALUES('temporary-a','2027-01-01T00:00:00Z','org-b');
         INSERT INTO service_principals VALUES('service-a',1);
         INSERT INTO org_parent_map VALUES('org-b','org-a');
         INSERT INTO plants VALUES('plant-a','org-a',0),('plant-b','org-b',0);
         INSERT INTO permissions VALUES('plant-b','[]','[\"viewer-a\"]','[\"temporary-a\"]');",
        )
        .unwrap();
    drop(connection);
    let output = temp.path().join("bundle");
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_grant-patch-data-authority-snapshot"))
            .args([
                "--database",
                database.to_str().unwrap(),
                "--source-revision",
                "0123456789abcdef0123456789abcdef01234567",
                "--source-ledger-digest",
                &format!("sha256:{}", "a".repeat(64)),
                "--generation",
                "1",
                "--produced-at",
                "2026-08-26T00:00:00Z",
                "--output-dir",
                output.to_str().unwrap(),
            ])
            .output()
            .unwrap()
    };
    let first = run();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let snapshot_bytes = fs::read(output.join("snapshot.json")).unwrap();
    let snapshot: Value = serde_json::from_slice(&snapshot_bytes).unwrap();
    let receipt: Value =
        serde_json::from_slice(&fs::read(output.join("receipt.json")).unwrap()).unwrap();
    assert_eq!(snapshot["sourceId"], "patch-data");
    assert_eq!(snapshot["principals"].as_array().unwrap().len(), 4);
    assert!(
        snapshot["effectiveBindings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|binding| binding["resourceId"] == "plant-b")
    );
    assert_eq!(receipt["snapshotDigest"], snapshot["snapshotDigest"]);
    assert_eq!(
        receipt["snapshotArtifactSha256"],
        format!("sha256:{}", hex::encode(Sha256::digest(&snapshot_bytes)))
    );
    assert_ne!(receipt["snapshotArtifactSha256"], receipt["snapshotDigest"]);
    let retry = run();
    assert!(!retry.status.success());
    assert!(String::from_utf8_lossy(&retry.stderr).contains("already exists"));
}

#[test]
fn rejects_unknown_arguments_before_reading_a_database() {
    let output = Command::new(env!("CARGO_BIN_EXE_grant-patch-data-authority-snapshot"))
        .args(["--backend-url", "https://forbidden.invalid"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown argument"));
}

#[test]
fn rejects_a_database_with_live_wal_sidecars() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("data.db");
    fs::write(&database, []).unwrap();
    fs::write(temp.path().join("data.db-wal"), []).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_grant-patch-data-authority-snapshot"))
        .args([
            "--database",
            database.to_str().unwrap(),
            "--source-revision",
            "a",
            "--source-ledger-digest",
            &format!("sha256:{}", "a".repeat(64)),
            "--generation",
            "1",
            "--produced-at",
            "2026-08-26T00:00:00Z",
            "--output-dir",
            temp.path().join("out").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not a frozen checkpoint"));
}

#[cfg(unix)]
#[test]
fn rejects_a_symlink_to_a_database_with_live_wal_sidecars() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let target_dir = temp.path().join("target");
    let link_dir = temp.path().join("link");
    fs::create_dir(&target_dir).unwrap();
    fs::create_dir(&link_dir).unwrap();
    let target = target_dir.join("data.db");
    fs::write(&target, []).unwrap();
    fs::write(target_dir.join("data.db-wal"), []).unwrap();
    let database = link_dir.join("checkpoint.db");
    symlink(&target, &database).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_grant-patch-data-authority-snapshot"))
        .args([
            "--database",
            database.to_str().unwrap(),
            "--source-revision",
            "a",
            "--source-ledger-digest",
            &format!("sha256:{}", "a".repeat(64)),
            "--generation",
            "1",
            "--produced-at",
            "2026-08-26T00:00:00Z",
            "--output-dir",
            temp.path().join("out").to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not a frozen checkpoint"), "{stderr}");
    assert!(stderr.contains(target_dir.to_str().unwrap()), "{stderr}");
}
