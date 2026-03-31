use anyhow::Context;
use infra::{artifacts::ArtifactStore, config::AppConfig, database::Database, service::AppService};
use std::sync::Arc;
use tracing_subscriber::{EnvFilter, fmt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let config = AppConfig::from_env()?;
    let node_config = hiqlite::NodeConfig::from_env();
    let client = hiqlite::start_node(node_config)
        .await
        .context("start hiqlite node")?;

    let db = Database::new(
        client,
        config.security.bootstrap_user_id.clone(),
        config.security.bootstrap_user_name.clone(),
    );
    let artifacts = ArtifactStore::new(&config.artifacts)
        .await
        .context("init artifact store")?;
    let service = Arc::new(AppService::new(config.clone(), db, artifacts));
    service.initialize().await.context("initialize service")?;

    let router = api::build_router(service);
    let listener = tokio::net::TcpListener::bind(&config.server.bind_addr)
        .await
        .with_context(|| format!("bind {}", config.server.bind_addr))?;
    tracing::info!("certd listening on {}", config.server.bind_addr);
    axum::serve(listener, router).await?;
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,aws_config=warn,aws_smithy_runtime=warn"));
    fmt().with_env_filter(filter).with_target(false).init();
}
