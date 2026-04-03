use anyhow::Context;
use infra::{
    artifacts::ArtifactStore,
    config::{AppConfig, RuntimeMode},
    database::Database,
    runtime::HiqliteRuntime,
    service::AppService,
};
use std::sync::Arc;
use tracing_subscriber::{EnvFilter, fmt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let config = AppConfig::from_env()?;
    let runtime = build_runtime(&config).await?;
    let db = Database::new(
        runtime,
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

async fn build_runtime(config: &AppConfig) -> anyhow::Result<HiqliteRuntime> {
    match config.hiqlite.runtime_mode {
        RuntimeMode::Embedded => {
            let mut node_config = hiqlite::NodeConfig::from_env();
            node_config.learner_only = matches!(
                config.hiqlite.local_node_role,
                infra::config::LocalNodeRole::Learner
            );
            let local = hiqlite::start_node(node_config)
                .await
                .context("start embedded hiqlite node")?;
            Ok(HiqliteRuntime::embedded(
                local,
                config.hiqlite.default_read_consistency.clone(),
            ))
        }
        RuntimeMode::Remote => Ok(HiqliteRuntime::remote(
            build_remote_client(config).await?,
            config.hiqlite.default_read_consistency.clone(),
        )),
        RuntimeMode::Split => {
            let mut node_config = hiqlite::NodeConfig::from_env();
            node_config.learner_only = matches!(
                config.hiqlite.local_node_role,
                infra::config::LocalNodeRole::Learner
            );
            let local = hiqlite::start_node(node_config)
                .await
                .context("start split-mode local hiqlite node")?;
            let remote = build_remote_client(config).await?;
            Ok(HiqliteRuntime::split(
                local,
                remote,
                config.hiqlite.default_read_consistency.clone(),
            ))
        }
    }
}

async fn build_remote_client(config: &AppConfig) -> anyhow::Result<hiqlite::Client> {
    let api_secret = config
        .hiqlite
        .remote_api_secret
        .clone()
        .context("CERTD_REMOTE_API_SECRET is required for remote or split runtime")?;
    if config.hiqlite.remote_nodes.is_empty() {
        anyhow::bail!("CERTD_REMOTE_NODES is required for remote or split runtime");
    }
    hiqlite::Client::remote(
        config.hiqlite.remote_nodes.clone(),
        config.hiqlite.remote_tls,
        config.hiqlite.remote_tls_no_verify,
        api_secret,
        config.hiqlite.remote_with_proxy,
    )
    .await
    .context("create remote hiqlite client")
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,aws_config=warn,aws_smithy_runtime=warn"));
    fmt().with_env_filter(filter).with_target(false).init();
}
