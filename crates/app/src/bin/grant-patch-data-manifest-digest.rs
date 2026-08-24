use anyhow::{Context, bail};
use infra::database::{PatchDataAuthorityImport, patch_data_manifest_digest};
use serde_json::Value;
use std::io::{self, Read};

fn main() -> anyhow::Result<()> {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .context("read Patch Data authority manifest from stdin")?;
    let mut value: Value = serde_json::from_str(&input).context("parse manifest JSON")?;
    let Some(object) = value.as_object_mut() else {
        bail!("manifest must be a JSON object");
    };
    // manifestDigest is excluded from its own canonical digest. Supplying it is
    // optional for this tool so a producer can generate the value before POST.
    object.insert("manifestDigest".to_owned(), Value::String("0".repeat(64)));
    let manifest: PatchDataAuthorityImport =
        serde_json::from_value(value).context("validate manifest wire shape")?;
    println!("{}", patch_data_manifest_digest(&manifest));
    Ok(())
}
