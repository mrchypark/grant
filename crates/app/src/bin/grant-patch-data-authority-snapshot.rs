use anyhow::{Context, bail};
use chrono::{DateTime, Utc};
use infra::database::{
    ApplicationScopeV2, AuthorityApplicationV2, AuthorityMembershipV2, AuthorityOrganizationV2,
    AuthorityPrincipalV2, AuthoritySnapshotV2, EffectiveBindingV2, snapshot_digest,
};
use rusqlite::{Connection, OpenFlags, Transaction};
use rustix::fs::{CWD, RenameFlags, renameat_with};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
};

const SOURCE_ID: &str = "patch-data";
const PROVIDER_ID: &str = "patch-data";
const ACTION_ID: &str = "patch-data.plant.read";

#[derive(Debug)]
struct Args {
    database: PathBuf,
    source_revision: String,
    source_ledger_digest: String,
    generation: u64,
    produced_at: String,
    output_dir: PathBuf,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Receipt<'a> {
    schema_version: u64,
    source_id: &'a str,
    generation: u64,
    snapshot_digest: &'a str,
    snapshot_artifact_sha256: String,
    source_revision: &'a str,
    source_ledger_digest: &'a str,
    source_watermark: &'a str,
    principal_count: usize,
    organization_count: usize,
    membership_count: usize,
    application_count: usize,
    effective_binding_count: usize,
}

fn main() -> anyhow::Result<()> {
    let args = parse_args()?;
    if args.output_dir.exists() {
        bail!("output directory already exists");
    }
    let produced_at =
        DateTime::parse_from_rfc3339(&args.produced_at).context("invalid --produced-at")?;
    if !args.produced_at.ends_with('Z')
        || produced_at
            .with_timezone(&Utc)
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
            != args.produced_at
    {
        bail!("--produced-at must be canonical UTC seconds");
    }
    require_digest(&args.source_ledger_digest, "--source-ledger-digest")?;
    let database = args
        .database
        .canonicalize()
        .context("canonicalize frozen PocketBase SQLite file")?;
    for suffix in ["-wal", "-shm"] {
        let mut sidecar = database.as_os_str().to_os_string();
        sidecar.push(suffix);
        let sidecar = PathBuf::from(sidecar);
        if sidecar.exists() {
            bail!(
                "database is not a frozen checkpoint: {} exists",
                sidecar.display()
            );
        }
    }
    let uri = format!("file:{}?immutable=1", database.display());
    let mut connection = Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .context("open frozen PocketBase SQLite file read-only")?;
    let transaction = connection
        .transaction()
        .context("begin frozen read transaction")?;
    let mut snapshot = export(&transaction, &args)?;
    transaction
        .commit()
        .context("finish frozen read transaction")?;
    snapshot.snapshot_digest = snapshot_digest(&snapshot)?;
    let snapshot_bytes = serde_json::to_vec(&snapshot)?;
    let snapshot_artifact_sha256 =
        format!("sha256:{}", hex::encode(Sha256::digest(&snapshot_bytes)));
    let receipt = Receipt {
        schema_version: snapshot.schema_version,
        source_id: &snapshot.source_id,
        generation: snapshot.generation,
        snapshot_digest: &snapshot.snapshot_digest,
        snapshot_artifact_sha256,
        source_revision: &snapshot.source_revision,
        source_ledger_digest: &snapshot.source_ledger_digest,
        source_watermark: &snapshot.source_watermark,
        principal_count: snapshot.principals.len(),
        organization_count: snapshot.organizations.len(),
        membership_count: snapshot.memberships.len(),
        application_count: snapshot.applications.len(),
        effective_binding_count: snapshot.effective_bindings.len(),
    };
    persist_bundle(
        &args.output_dir,
        &snapshot_bytes,
        &serde_json::to_vec(&receipt)?,
    )?;
    Ok(())
}

fn parse_args() -> anyhow::Result<Args> {
    let mut values = env::args().skip(1);
    let mut map = BTreeMap::new();
    while let Some(name) = values.next() {
        if !matches!(
            name.as_str(),
            "--database"
                | "--source-revision"
                | "--source-ledger-digest"
                | "--generation"
                | "--produced-at"
                | "--output-dir"
        ) {
            bail!("unknown argument {name}");
        }
        let value = values
            .next()
            .with_context(|| format!("missing value for {name}"))?;
        if map.insert(name.clone(), value).is_some() {
            bail!("duplicate argument {name}");
        }
    }
    let take = |name: &str| {
        map.get(name)
            .cloned()
            .with_context(|| format!("missing {name}"))
    };
    Ok(Args {
        database: PathBuf::from(take("--database")?),
        source_revision: take("--source-revision")?,
        source_ledger_digest: take("--source-ledger-digest")?,
        generation: take("--generation")?
            .parse()
            .context("invalid --generation")?,
        produced_at: take("--produced-at")?,
        output_dir: PathBuf::from(take("--output-dir")?),
    })
}

fn export(tx: &Transaction<'_>, args: &Args) -> anyhow::Result<AuthoritySnapshotV2> {
    if args.generation == 0
        || args.source_revision.trim() != args.source_revision
        || args.source_revision.is_empty()
    {
        bail!("invalid generation or source revision");
    }
    let mut principals = BTreeMap::<String, AuthorityPrincipalV2>::new();
    let mut organizations = BTreeMap::<String, AuthorityOrganizationV2>::new();
    let mut memberships = BTreeMap::<String, AuthorityMembershipV2>::new();
    let mut bindings = BTreeMap::<String, EffectiveBindingV2>::new();
    let mut watermark = Sha256::new();

    for (id, active) in rows2(tx, "SELECT id,'1' FROM organizations ORDER BY id")? {
        watermark_row(&mut watermark, &[&id, &active]);
        organizations.insert(
            id.clone(),
            AuthorityOrganizationV2 {
                id,
                active: active == "1",
            },
        );
    }
    let produced = DateTime::parse_from_rfc3339(&args.produced_at)?;
    for spec in [
        (
            "user",
            "SELECT id,deleted,organizations FROM users ORDER BY id",
        ),
        (
            "viewer",
            "SELECT id,deleted,organizations FROM viewers ORDER BY id",
        ),
        (
            "temporary",
            "SELECT id,expired,organization FROM temporary_accounts ORDER BY id",
        ),
        (
            "service",
            "SELECT id,CAST(enabled AS TEXT),'' FROM service_principals ORDER BY id",
        ),
    ] {
        for (id, state, orgs) in rows3(tx, spec.1)? {
            watermark_row(&mut watermark, &[&spec.0.to_owned(), &id, &state, &orgs]);
            let active = match spec.0 {
                "service" => state == "1",
                "temporary" => DateTime::parse_from_rfc3339(&state).is_ok_and(|at| at > produced),
                _ => state.is_empty(),
            };
            principals.insert(
                id.clone(),
                AuthorityPrincipalV2 {
                    id: id.clone(),
                    kind: spec.0.to_owned(),
                    active,
                    external_subject: format!("patch-data:{0}:{id}", spec.0),
                },
            );
            if spec.0 != "service" {
                for organization_id in relation_ids(&orgs)? {
                    let role = spec.0;
                    let membership_id = format!("{spec}-{id}-{organization_id}", spec = spec.0);
                    memberships.insert(
                        membership_id.clone(),
                        AuthorityMembershipV2 {
                            id: membership_id,
                            principal_id: id.clone(),
                            organization_id,
                            role: if role == "user" { "member" } else { role }.to_owned(),
                            principal_type: spec.0.to_owned(),
                            lifecycle: if active { "active" } else { "disabled" }.to_owned(),
                            valid_until: (spec.0 == "temporary").then(|| state.clone()),
                            source_refs: vec![format!("{0}:{id}", spec.0)],
                        },
                    );
                }
            }
        }
    }
    // Organization owners are explicit active memberships.
    for (org, owner) in rows2(
        tx,
        "SELECT id,owner FROM organizations WHERE owner<>'' ORDER BY id",
    )? {
        watermark_row(&mut watermark, &[&org, &owner]);
        let id = format!("owner-{owner}-{org}");
        memberships.insert(
            id.clone(),
            AuthorityMembershipV2 {
                id,
                principal_id: owner,
                organization_id: org,
                role: "owner".into(),
                principal_type: "user".into(),
                lifecycle: "active".into(),
                valid_until: None,
                source_refs: vec!["organizations.owner".into()],
            },
        );
    }
    let descendants = hierarchy(tx, &mut watermark)?;
    let plants = rows2(
        tx,
        "SELECT id,organization FROM plants WHERE isArchive=0 ORDER BY id",
    )?;
    for (plant, org) in &plants {
        watermark_row(&mut watermark, &[plant, org]);
    }
    // User organization scope expands through the materialized hierarchy.
    for membership in memberships
        .values()
        .filter(|m| m.lifecycle == "active" && m.principal_type == "user")
    {
        let mut scopes = descendants
            .get(&membership.organization_id)
            .cloned()
            .unwrap_or_default();
        scopes.insert(membership.organization_id.clone());
        for (plant, org) in &plants {
            if scopes.contains(org) {
                insert_binding(
                    &mut bindings,
                    &membership.principal_id,
                    &membership.organization_id,
                    plant,
                    "descendant",
                    &[&membership.id],
                );
            }
        }
    }
    for (plant, users, viewers, temporary) in rows4(
        tx,
        "SELECT plant,users,viewers,temporary FROM permissions ORDER BY plant",
    )? {
        watermark_row(&mut watermark, &[&plant, &users, &viewers, &temporary]);
        for (kind, raw) in [
            ("user", users),
            ("viewer", viewers),
            ("temporary", temporary),
        ] {
            for principal in relation_ids(&raw)? {
                if let Some(membership) = memberships.values().find(|m| {
                    m.principal_id == principal
                        && m.principal_type == kind
                        && m.lifecycle == "active"
                }) {
                    insert_binding(
                        &mut bindings,
                        &principal,
                        &membership.organization_id,
                        &plant,
                        if kind == "temporary" {
                            "temporary-assignment"
                        } else {
                            "plant-permission"
                        },
                        &["permissions"],
                    );
                }
            }
        }
    }
    let source_watermark = format!("sha256:{}", hex::encode(watermark.finalize()));
    Ok(AuthoritySnapshotV2 {
        schema_version: 2,
        source_id: SOURCE_ID.into(),
        generation: args.generation,
        source_revision: args.source_revision.clone(),
        source_ledger_digest: args.source_ledger_digest.clone(),
        source_watermark,
        produced_at: args.produced_at.clone(),
        principals: principals.into_values().collect(),
        organizations: organizations.into_values().collect(),
        memberships: memberships.into_values().collect(),
        applications: Vec::<AuthorityApplicationV2>::new(),
        effective_bindings: bindings.into_values().collect(),
        snapshot_digest: format!("sha256:{}", "0".repeat(64)),
    })
}

fn insert_binding(
    out: &mut BTreeMap<String, EffectiveBindingV2>,
    principal: &str,
    org: &str,
    plant: &str,
    derivation: &str,
    refs: &[&str],
) {
    let key = format!("{principal}\0{org}\0{plant}");
    out.entry(key).or_insert_with(|| EffectiveBindingV2 {
        principal_id: principal.into(),
        application_scope: ApplicationScopeV2 {
            kind: "wildcard".into(),
            application_id: None,
            organization_id: org.into(),
        },
        provider_id: PROVIDER_ID.into(),
        action_id: ACTION_ID.into(),
        resource_type: "plant".into(),
        resource_id: plant.into(),
        effect: "allow".into(),
        source_rule_ids: refs.iter().map(|v| (*v).to_owned()).collect(),
        derivation: derivation.into(),
    });
}

fn hierarchy(
    tx: &Transaction<'_>,
    watermark: &mut Sha256,
) -> anyhow::Result<BTreeMap<String, BTreeSet<String>>> {
    let mut out = BTreeMap::<String, BTreeSet<String>>::new();
    for (child, parent) in rows2(
        tx,
        "SELECT organization,parent FROM org_parent_map ORDER BY parent,organization",
    )? {
        watermark_row(watermark, &[&child, &parent]);
        if child != parent {
            out.entry(parent).or_default().insert(child);
        }
    }
    Ok(out)
}

fn relation_ids(raw: &str) -> anyhow::Result<Vec<String>> {
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    if raw.starts_with('[') {
        return serde_json::from_str(raw).context("decode PocketBase relation array");
    }
    Ok(vec![raw.to_owned()])
}

fn rows2(tx: &Transaction<'_>, sql: &'static str) -> anyhow::Result<Vec<(String, String)>> {
    let mut s = tx.prepare(sql)?;
    Ok(s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?)
}
fn rows3(tx: &Transaction<'_>, sql: &'static str) -> anyhow::Result<Vec<(String, String, String)>> {
    let mut s = tx.prepare(sql)?;
    Ok(s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?)
}
fn rows4(
    tx: &Transaction<'_>,
    sql: &'static str,
) -> anyhow::Result<Vec<(String, String, String, String)>> {
    let mut s = tx.prepare(sql)?;
    Ok(
        s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<Result<_, _>>()?,
    )
}
fn watermark_row(hash: &mut Sha256, values: &[&String]) {
    for value in values {
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value.as_bytes());
    }
}
fn require_digest(value: &str, name: &str) -> anyhow::Result<()> {
    if value.len() != 71
        || !value.starts_with("sha256:")
        || !value[7..]
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        bail!("invalid {name}");
    }
    Ok(())
}

fn persist_bundle(target: &Path, snapshot: &[u8], receipt: &[u8]) -> anyhow::Result<()> {
    persist_bundle_before_publish(target, snapshot, receipt, || Ok(()))
}

fn persist_bundle_before_publish(
    target: &Path,
    snapshot: &[u8],
    receipt: &[u8],
    before_publish: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let parent = target
        .parent()
        .context("output directory requires parent")?;
    let temp = tempfile::Builder::new()
        .prefix(".authority-snapshot-")
        .tempdir_in(parent)?;
    fs::write(temp.path().join("snapshot.json"), snapshot)?;
    fs::write(temp.path().join("receipt.json"), receipt)?;
    before_publish()?;
    renameat_with(CWD, temp.path(), CWD, target, RenameFlags::NOREPLACE)
        .context("atomically publish immutable snapshot bundle without replacement")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::persist_bundle_before_publish;
    use std::fs;

    #[test]
    fn concurrent_creator_is_never_overwritten_at_publication() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("bundle");

        let error = persist_bundle_before_publish(&target, b"snapshot", b"receipt", || {
            fs::create_dir(&target)?;
            Ok(())
        })
        .unwrap_err();

        assert!(error.to_string().contains("without replacement"));
        assert!(target.is_dir());
        assert!(fs::read_dir(&target).unwrap().next().is_none());
    }
}
