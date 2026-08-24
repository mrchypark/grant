use std::{
    io::Write,
    process::{Command, Stdio},
};

const MANIFEST: &str = r#"{
  "manifestVersion": "grant.patch-data-authority.v1",
  "generation": "generation-a",
  "ledgerDigest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "expectedPreviousGeneration": null,
  "users": [{"userId":"user-a","externalUserId":"external-a","email":null,"displayName":"User A","active":true}],
  "organizations": [{"organizationId":"org-a","name":"Org A","active":true}],
  "memberships": [{"membershipId":"membership-a","organizationId":"org-a","userId":"user-a","role":"member","status":"active"}],
  "hierarchyEdges": [],
  "plantBindings": [{"bindingId":"binding-a","subjectType":"user","subjectId":"user-a","subjectOrganizationId":"org-a","plantId":"plant-a","state":"active"}],
  "applicationAssignments": []
}"#;

#[test]
fn computes_the_api_canonical_digest_without_a_placeholder_field() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_grant-patch-data-manifest-digest"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start digest tool");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(MANIFEST.as_bytes())
        .expect("write manifest");
    let output = child.wait_with_output().expect("digest output");
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("UTF-8 digest"),
        "f108240f743a942b70b2e6654e2c2f9362cea21f0ac970551fc5e45295b315b0\n"
    );
}

#[test]
fn rejects_unknown_manifest_fields() {
    let malformed = MANIFEST.replacen("{", "{\"unexpected\":true,", 1);
    let mut child = Command::new(env!("CARGO_BIN_EXE_grant-patch-data-manifest-digest"))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start digest tool");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(malformed.as_bytes())
        .expect("write manifest");
    let output = child.wait_with_output().expect("digest output");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("unknown field"),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}
