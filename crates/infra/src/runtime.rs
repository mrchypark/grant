use crate::config::{ReadConsistency, RuntimeMode};
use anyhow::{Result, anyhow};
use domain::HealthStatus;
use hiqlite::{Client, Row};
use std::borrow::Cow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Readiness {
    pub read_ready: bool,
    pub write_ready: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadRoute {
    Local,
    Strong,
}

#[derive(Debug, Clone)]
pub struct RuntimeBackupItem {
    pub name: String,
    pub last_modified: i64,
    pub size: Option<u64>,
}

pub fn resolve_read_route(
    consistency: ReadConsistency,
    readiness: Readiness,
) -> Result<ReadRoute> {
    match consistency {
        ReadConsistency::Local => {
            if readiness.read_ready {
                Ok(ReadRoute::Local)
            } else if readiness.write_ready {
                Ok(ReadRoute::Strong)
            } else {
                Err(anyhow!("no readable hiqlite route available"))
            }
        }
        ReadConsistency::Strong => {
            if readiness.write_ready {
                Ok(ReadRoute::Strong)
            } else {
                Err(anyhow!("strong read requested but quorum write path is not ready"))
            }
        }
        ReadConsistency::PreferStrong => {
            if readiness.write_ready {
                Ok(ReadRoute::Strong)
            } else if readiness.read_ready {
                Ok(ReadRoute::Local)
            } else {
                Err(anyhow!("no readable hiqlite route available"))
            }
        }
    }
}

#[derive(Clone)]
pub struct HiqliteRuntime {
    local: Option<Client>,
    remote: Option<Client>,
    runtime_mode: RuntimeMode,
    default_read_consistency: ReadConsistency,
}

impl HiqliteRuntime {
    pub fn embedded(local: Client, default_read_consistency: ReadConsistency) -> Self {
        Self {
            local: Some(local),
            remote: None,
            runtime_mode: RuntimeMode::Embedded,
            default_read_consistency,
        }
    }

    pub fn remote(remote: Client, default_read_consistency: ReadConsistency) -> Self {
        Self {
            local: None,
            remote: Some(remote),
            runtime_mode: RuntimeMode::Remote,
            default_read_consistency,
        }
    }

    pub fn split(
        local: Client,
        remote: Client,
        default_read_consistency: ReadConsistency,
    ) -> Self {
        Self {
            local: Some(local),
            remote: Some(remote),
            runtime_mode: RuntimeMode::Split,
            default_read_consistency,
        }
    }

    pub fn default_read_consistency(&self) -> ReadConsistency {
        self.default_read_consistency.clone()
    }

    pub fn runtime_mode(&self) -> &RuntimeMode {
        &self.runtime_mode
    }

    pub async fn health_status(&self) -> HealthStatus {
        let readiness = self.readiness().await;
        let status = if readiness.read_ready && readiness.write_ready {
            "ok"
        } else if readiness.read_ready || readiness.write_ready {
            "degraded"
        } else {
            "down"
        };
        HealthStatus::new(
            status,
            readiness.read_ready,
            readiness.write_ready,
            self.runtime_mode.as_str(),
            self.default_read_consistency.as_str(),
        )
    }

    pub async fn readiness(&self) -> Readiness {
        let read_ready = if let Some(client) = self.local_read_client() {
            client.is_healthy_db().await.is_ok()
        } else {
            false
        };
        let write_ready = if let Some(client) = self.write_client() {
            client.is_healthy_db().await.is_ok()
        } else {
            false
        };
        Readiness {
            read_ready,
            write_ready,
        }
    }

    pub async fn execute<S>(&self, sql: S, params: Vec<hiqlite::Param>) -> Result<usize>
    where
        S: Into<Cow<'static, str>>,
    {
        self.write_client()
            .ok_or_else(|| anyhow!("no write-capable hiqlite client configured"))?
            .execute(sql, params)
            .await
            .map_err(Into::into)
    }

    pub async fn batch<S>(&self, sql: S) -> Result<Vec<Result<usize>>>
    where
        S: Into<Cow<'static, str>>,
    {
        self.write_client()
            .ok_or_else(|| anyhow!("no write-capable hiqlite client configured"))?
            .batch(sql)
            .await
            .map(|rows| rows.into_iter().map(|row| row.map_err(Into::into)).collect())
            .map_err(Into::into)
    }

    pub async fn query_rows<T, S>(
        &self,
        consistency: ReadConsistency,
        stmt: S,
        params: Vec<hiqlite::Param>,
    ) -> Result<Vec<T>>
    where
        T: for<'a, 'r> From<&'a mut Row<'r>> + Send + 'static,
        S: Into<Cow<'static, str>>,
    {
        let stmt = stmt.into();
        let route = resolve_read_route(consistency, self.readiness().await)?;
        match route {
            ReadRoute::Local => self
                .local_read_client()
                .ok_or_else(|| anyhow!("no local hiqlite read client configured"))?
                .query_map(stmt, params)
                .await
                .map_err(Into::into),
            ReadRoute::Strong => self
                .write_client()
                .ok_or_else(|| anyhow!("no strong hiqlite read client configured"))?
                .query_consistent_map(stmt, params)
                .await
                .map_err(Into::into),
        }
    }

    pub async fn backup(&self) -> Result<()> {
        self.write_client()
            .ok_or_else(|| anyhow!("no write-capable hiqlite client configured"))?
            .backup()
            .await
            .map_err(Into::into)
    }

    pub async fn backup_list_local(&self) -> Result<Vec<RuntimeBackupItem>> {
        self.local
            .as_ref()
            .ok_or_else(|| anyhow!("local hiqlite client required for local backup listing"))?
            .backup_list_local()
            .await
            .map(|items| {
                items
                    .into_iter()
                    .map(|item| RuntimeBackupItem {
                        name: item.name,
                        last_modified: item.last_modified,
                        size: item.size,
                    })
                    .collect()
            })
            .map_err(Into::into)
    }

    pub async fn backup_list_s3(&self) -> Result<Vec<RuntimeBackupItem>> {
        self.local
            .as_ref()
            .or(self.remote.as_ref())
            .ok_or_else(|| anyhow!("hiqlite client required for s3 backup listing"))?
            .backup_list_s3()
            .await
            .map(|items| {
                items
                    .into_iter()
                    .map(|item| RuntimeBackupItem {
                        name: item.name,
                        last_modified: item.last_modified,
                        size: item.size,
                    })
                    .collect()
            })
            .map_err(Into::into)
    }

    fn local_read_client(&self) -> Option<&Client> {
        self.local.as_ref().or(self.remote.as_ref())
    }

    fn write_client(&self) -> Option<&Client> {
        self.remote.as_ref().or(self.local.as_ref())
    }
}

impl RuntimeMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            RuntimeMode::Embedded => "embedded",
            RuntimeMode::Remote => "remote",
            RuntimeMode::Split => "split",
        }
    }
}

impl ReadConsistency {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReadConsistency::Local => "local",
            ReadConsistency::Strong => "strong",
            ReadConsistency::PreferStrong => "prefer-strong",
        }
    }
}
