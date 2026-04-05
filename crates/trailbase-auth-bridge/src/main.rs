use anyhow::{Context, Result};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("seed-user") => {
            let mut data_dir: Option<PathBuf> = None;
            let mut email: Option<String> = None;
            let mut password: Option<String> = None;

            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--data-dir" => data_dir = args.next().map(PathBuf::from),
                    "--email" => email = args.next(),
                    "--password" => password = args.next(),
                    other => anyhow::bail!("unknown seed-user argument: {other}"),
                }
            }

            let data_dir = data_dir.context("missing --data-dir")?;
            let email = email.context("missing --email")?;
            let password = password.context("missing --password")?;
            trailbase_auth_bridge::seed_user(data_dir, &email, &password).await
        }
        Some(other) => anyhow::bail!("unknown command: {other}"),
        None => {
            let config = trailbase_auth_bridge::BridgeConfig::from_env()?;
            trailbase_auth_bridge::run(config).await
        }
    }
}
