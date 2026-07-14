use anyhow::Context;
use infra::{
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
    let service = Arc::new(AppService::new(config.clone(), db));
    if should_migrate_schema(&config.hiqlite.local_node_role) {
        service.migrate_schema().await.context("migrate schema")?;
    } else {
        tracing::info!(
            local_node_role = local_node_role_str(&config.hiqlite.local_node_role),
            "skipping schema migration on learner"
        );
    }
    if should_initialize_service(&config) {
        service.bootstrap().await.context("bootstrap service")?;
    } else {
        tracing::info!(
            runtime_mode = config.hiqlite.runtime_mode.as_str(),
            local_node_role = local_node_role_str(&config.hiqlite.local_node_role),
            "skipping service bootstrap on this node"
        );
    }
    spawn_k8s_reconcile_task(service.clone(), &config);

    let router = if matches!(config.hiqlite.local_node_role, LocalNodeRole::Learner) {
        api::build_learner_router(service)
    } else {
        api::build_router(service)
    };
    let listener = tokio::net::TcpListener::bind(&config.server.bind_addr)
        .await
        .with_context(|| format!("bind {}", config.server.bind_addr))?;
    tracing::info!("grant listening on {}", config.server.bind_addr);
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
            service
                .migrate_schema()
                .await
                .context("reconcile migrate schema")?;
            service
                .bootstrap()
                .await
                .context("reconcile bootstrap service")
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

    let data_dir =
        std::env::var("HQL_DATA_DIR").unwrap_or_else(|_| "/var/lib/grant/hiqlite".to_string());
    let lock_path = Path::new(&data_dir).join("state_machine").join("lock");

    match fs::remove_file(&lock_path).await {
        Ok(()) => {
            tracing::info!(path = %lock_path.display(), "removed stale hiqlite restore lock file");
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) if err.kind() == std::io::ErrorKind::IsADirectory => {
            fs::remove_dir_all(&lock_path).await.with_context(|| {
                format!(
                    "remove stale hiqlite restore lock dir {}",
                    lock_path.display()
                )
            })?;
            tracing::info!(path = %lock_path.display(), "removed stale hiqlite restore lock dir");
        }
        Err(err) => {
            return Err(err).with_context(|| {
                format!("remove stale hiqlite restore lock {}", lock_path.display())
            });
        }
    }

    Ok(())
}

async fn build_remote_client(config: &AppConfig) -> anyhow::Result<hiqlite::Client> {
    let api_secret = config
        .hiqlite
        .remote_api_secret
        .clone()
        .context("GRANT_REMOTE_API_SECRET is required for remote or split runtime")?;
    if config.hiqlite.remote_nodes.is_empty() {
        anyhow::bail!("GRANT_REMOTE_NODES is required for remote or split runtime");
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
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    fmt().with_env_filter(filter).with_target(false).init();
}

fn should_initialize_service(config: &AppConfig) -> bool {
    should_initialize_for_node(
        &config.hiqlite.runtime_mode,
        &config.hiqlite.local_node_role,
        is_k8s_node_id_source(),
        is_bootstrap_voter_pod(),
        needs_bootstrap_initialization(),
    )
}

fn should_migrate_schema(role: &LocalNodeRole) -> bool {
    matches!(role, LocalNodeRole::Voter)
}

fn should_initialize_for_node(
    runtime_mode: &RuntimeMode,
    role: &LocalNodeRole,
    uses_k8s_node_ids: bool,
    is_bootstrap_voter: bool,
    needs_bootstrap: bool,
) -> bool {
    if matches!(role, LocalNodeRole::Learner) {
        return false;
    }
    if matches!(runtime_mode, RuntimeMode::Embedded) && uses_k8s_node_ids {
        return is_bootstrap_voter && needs_bootstrap;
    }
    true
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
    let data_dir =
        std::env::var("HQL_DATA_DIR").unwrap_or_else(|_| "/var/lib/grant/hiqlite".to_string());
    let state_machine_dir = Path::new(&data_dir).join("state_machine");
    !state_machine_dir.exists()
}

#[cfg(test)]
mod tests {
    use super::{should_initialize_for_node, should_migrate_schema};
    use infra::config::{LocalNodeRole, RuntimeMode};

    #[test]
    fn learners_never_migrate_or_bootstrap() {
        assert!(!should_migrate_schema(&LocalNodeRole::Learner));
        for runtime_mode in [
            RuntimeMode::Embedded,
            RuntimeMode::Remote,
            RuntimeMode::Split,
        ] {
            assert!(!should_initialize_for_node(
                &runtime_mode,
                &LocalNodeRole::Learner,
                false,
                true,
                true,
            ));
        }
    }

    #[test]
    fn voters_migrate_and_keep_bootstrap_ownership_rules() {
        assert!(should_migrate_schema(&LocalNodeRole::Voter));
        assert!(should_initialize_for_node(
            &RuntimeMode::Remote,
            &LocalNodeRole::Voter,
            false,
            false,
            false,
        ));
        assert!(should_initialize_for_node(
            &RuntimeMode::Embedded,
            &LocalNodeRole::Voter,
            true,
            true,
            true,
        ));
        assert!(!should_initialize_for_node(
            &RuntimeMode::Embedded,
            &LocalNodeRole::Voter,
            true,
            false,
            true,
        ));
    }
}
