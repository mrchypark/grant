use anyhow::Context;
use infra::{
    artifacts::ArtifactStore,
    config::{AppConfig, LocalNodeRole, RuntimeMode},
    database::Database,
    runtime::HiqliteRuntime,
    service::AppService,
};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs;
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
    service
        .migrate_schema()
        .await
        .context("migrate schema")?;
    if should_initialize_service(&config) {
        service.initialize().await.context("initialize service")?;
    } else {
        tracing::info!(
            runtime_mode = config.hiqlite.runtime_mode.as_str(),
            local_node_role = local_node_role_str(&config.hiqlite.local_node_role),
            "skipping service initialization on split learner"
        );
    }
    spawn_k8s_reconcile_task(service.clone(), &config);

    let router = api::build_router(service);
    let listener = tokio::net::TcpListener::bind(&config.server.bind_addr)
        .await
        .with_context(|| format!("bind {}", config.server.bind_addr))?;
    tracing::info!("certd listening on {}", config.server.bind_addr);
    axum::serve(listener, router).await?;
    Ok(())
}

fn spawn_k8s_reconcile_task(service: Arc<AppService>, config: &AppConfig) {
    if !matches!(config.hiqlite.runtime_mode, RuntimeMode::Embedded) {
        return;
    }
    if !matches!(config.hiqlite.local_node_role, LocalNodeRole::Voter) {
        return;
    }
    if !is_k8s_node_id_source() {
        return;
    }

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(15)).await;
        match async {
            // After the voter has rejoined the cluster, push schema updates
            // through the write path so leader-backed DDL exists cluster-wide.
            service.migrate_schema().await.context("reconcile migrate schema")?;
            service.initialize().await.context("reconcile initialize service")
        }
        .await
        {
            Ok(()) => tracing::info!("k8s voter reconcile completed"),
            Err(err) => tracing::warn!(error = %err, "k8s voter reconcile failed"),
        }
    });
}

async fn build_runtime(config: &AppConfig) -> anyhow::Result<HiqliteRuntime> {
    match config.hiqlite.runtime_mode {
        RuntimeMode::Embedded => {
            clear_hiqlite_restore_lock_if_needed().await?;
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
            clear_hiqlite_restore_lock_if_needed().await?;
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

async fn clear_hiqlite_restore_lock_if_needed() -> anyhow::Result<()> {
    if std::env::var("HQL_BACKUP_RESTORE")
        .ok()
        .filter(|value| !value.is_empty())
        .is_none()
    {
        return Ok(());
    }

    let data_dir = std::env::var("HQL_DATA_DIR").unwrap_or_else(|_| "/var/lib/certd/hiqlite".to_string());
    let lock_path = Path::new(&data_dir).join("state_machine").join("lock");

    match fs::remove_file(&lock_path).await {
        Ok(()) => {
            tracing::info!(path = %lock_path.display(), "removed stale hiqlite restore lock file");
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) if err.kind() == std::io::ErrorKind::IsADirectory => {
            fs::remove_dir_all(&lock_path)
                .await
                .with_context(|| format!("remove stale hiqlite restore lock dir {}", lock_path.display()))?;
            tracing::info!(path = %lock_path.display(), "removed stale hiqlite restore lock dir");
        }
        Err(err) => {
            return Err(err)
                .with_context(|| format!("remove stale hiqlite restore lock {}", lock_path.display()));
        }
    }

    Ok(())
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

fn should_initialize_service(config: &AppConfig) -> bool {
    match (&config.hiqlite.runtime_mode, &config.hiqlite.local_node_role) {
        (RuntimeMode::Split, LocalNodeRole::Learner) => false,
        (RuntimeMode::Embedded, LocalNodeRole::Learner) => false,
        (RuntimeMode::Embedded, LocalNodeRole::Voter) if is_k8s_node_id_source() => {
            is_bootstrap_voter_pod() && needs_bootstrap_initialization()
        }
        _ => true,
    }
}

fn local_node_role_str(role: &LocalNodeRole) -> &'static str {
    match role {
        LocalNodeRole::Voter => "voter",
        LocalNodeRole::Learner => "learner",
    }
}

fn is_k8s_node_id_source() -> bool {
    std::env::var("HQL_NODE_ID_FROM")
        .map(|value| value.eq_ignore_ascii_case("k8s"))
        .unwrap_or(false)
}

fn is_bootstrap_voter_pod() -> bool {
    std::env::var("HOSTNAME")
        .map(|value| value.ends_with("-0"))
        .unwrap_or(false)
}

fn needs_bootstrap_initialization() -> bool {
    let data_dir = std::env::var("HQL_DATA_DIR").unwrap_or_else(|_| "/var/lib/certd/hiqlite".to_string());
    let state_machine_dir = Path::new(&data_dir).join("state_machine");
    !state_machine_dir.exists()
}
