use infra::service::{AppService, ExposureReadAuthorizationDecision, OperationAuthorizationTarget};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OperationDescriptor {
    pub(crate) id: &'static str,
    pub(crate) version: u64,
    pub(crate) contract_digest: &'static str,
    pub(crate) provider_id: &'static str,
    pub(crate) action_id: &'static str,
    pub(crate) resource_type: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DecisionAdapter {
    provider_id: &'static str,
    action_id: &'static str,
    resource_type: &'static str,
}

impl DecisionAdapter {
    pub(crate) const fn new(descriptor: OperationDescriptor) -> Self {
        Self {
            provider_id: descriptor.provider_id,
            action_id: descriptor.action_id,
            resource_type: descriptor.resource_type,
        }
    }

    pub(crate) async fn decide(
        self,
        service: &AppService,
        bearer_token: &str,
        organization_id: &str,
        resource_id: &str,
    ) -> anyhow::Result<ExposureReadAuthorizationDecision> {
        service
            .check_operation_authorization(
                Some(bearer_token),
                organization_id,
                OperationAuthorizationTarget {
                    provider_id: self.provider_id,
                    action_id: self.action_id,
                    resource_type: self.resource_type,
                    resource_id,
                },
            )
            .await
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RegisteredOperation {
    pub(crate) descriptor: OperationDescriptor,
    pub(crate) adapter: DecisionAdapter,
}

pub(crate) fn lookup(id: &str) -> Option<RegisteredOperation> {
    super::patch_data_operations::lookup(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_existing_p0_operation_is_callable() {
        for provisional in [
            "patch-data.organizations.list",
            "plant.list",
            "files.list",
            "issues.create",
        ] {
            assert!(
                lookup(provisional).is_none(),
                "{provisional} became callable"
            );
        }
    }
}
