macro_rules! params {
    ($($value:expr),* $(,)?) => {
        vec![$(::hiqlite::Param::from($value)),*]
    };
}

use super::*;

impl Database {
    pub fn new(
        runtime: HiqliteRuntime,
        bootstrap_user_id: String,
        bootstrap_user_name: String,
    ) -> Self {
        Self {
            runtime,
            bootstrap_user_id,
            bootstrap_user_name,
        }
    }

    pub(super) async fn query_rows_with<T, S>(
        &self,
        consistency: ReadConsistency,
        stmt: S,
        params: Vec<hiqlite::Param>,
        label: &str,
    ) -> Result<Vec<T>>
    where
        T: for<'a, 'r> From<&'a mut Row<'r>> + Send + 'static,
        S: Into<Cow<'static, str>>,
    {
        self.runtime
            .query_rows::<T, S>(consistency, stmt, params)
            .await
            .with_context(|| format!("query rows: {label}"))
    }

    pub(super) async fn query_row_with<T, S>(
        &self,
        consistency: ReadConsistency,
        stmt: S,
        params: Vec<hiqlite::Param>,
        label: &str,
    ) -> Result<T>
    where
        T: for<'a, 'r> From<&'a mut Row<'r>> + Send + 'static,
        S: Into<Cow<'static, str>>,
    {
        let rows: Vec<T> = self
            .query_rows_with::<T, S>(consistency, stmt, params, label)
            .await?;
        if rows.is_empty() {
            return Err(anyhow!("no row returned: {label}"));
        }
        if rows.len() > 1 {
            return Err(anyhow!("multiple rows returned: {label}, expected 1"));
        }
        rows.into_iter()
            .next()
            .ok_or_else(|| anyhow!("no row returned: {label}"))
    }

    pub(super) async fn query_consistent_rows<T, S>(
        &self,
        stmt: S,
        params: Vec<hiqlite::Param>,
        label: &str,
    ) -> Result<Vec<T>>
    where
        T: for<'a, 'r> From<&'a mut Row<'r>> + Send + 'static,
        S: Into<Cow<'static, str>>,
    {
        self.query_rows_with(ReadConsistency::Strong, stmt, params, label)
            .await
    }

    pub(super) async fn query_default_rows<T, S>(
        &self,
        stmt: S,
        params: Vec<hiqlite::Param>,
        label: &str,
    ) -> Result<Vec<T>>
    where
        T: for<'a, 'r> From<&'a mut Row<'r>> + Send + 'static,
        S: Into<Cow<'static, str>>,
    {
        self.query_rows_with(self.runtime.default_read_consistency(), stmt, params, label)
            .await
    }

    pub(super) async fn query_count<S>(
        &self,
        consistency: ReadConsistency,
        stmt: S,
        params: Vec<hiqlite::Param>,
        label: &str,
    ) -> Result<i64>
    where
        S: Into<Cow<'static, str>>,
    {
        Ok(self
            .query_row_with::<CountRow, S>(consistency, stmt, params, label)
            .await?
            .count)
    }

    pub(super) async fn exists_row_with<S>(
        &self,
        consistency: ReadConsistency,
        stmt: S,
        params: Vec<hiqlite::Param>,
        label: &str,
    ) -> Result<bool>
    where
        S: Into<Cow<'static, str>>,
    {
        Ok(self.query_count(consistency, stmt, params, label).await? > 0)
    }

    pub(super) fn validation_read_consistency(&self) -> ReadConsistency {
        validation_read_consistency_for_mode(self.runtime.runtime_mode())
    }

    pub(crate) fn default_read_consistency_with_override(
        &self,
        override_consistency: Option<ReadConsistency>,
    ) -> ReadConsistency {
        override_consistency.unwrap_or_else(|| self.runtime.default_read_consistency())
    }

    pub(super) fn validation_read_consistency_with_override(
        &self,
        override_consistency: Option<ReadConsistency>,
    ) -> ReadConsistency {
        override_consistency.unwrap_or_else(|| self.validation_read_consistency())
    }

    pub async fn health_status(&self) -> HealthStatus {
        self.runtime.health_status().await
    }

    pub async fn readiness(&self) -> Readiness {
        self.runtime.readiness().await
    }
}

fn validation_read_consistency_for_mode(
    runtime_mode: &crate::config::RuntimeMode,
) -> ReadConsistency {
    match runtime_mode {
        crate::config::RuntimeMode::Embedded => ReadConsistency::Local,
        crate::config::RuntimeMode::Remote => ReadConsistency::Strong,
        crate::config::RuntimeMode::Split => ReadConsistency::Strong,
    }
}

pub(super) fn now_string() -> String {
    DateTime::<Utc>::from(std::time::SystemTime::now()).to_rfc3339()
}
