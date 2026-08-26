use crate::operation_catalog::{DecisionAdapter, OperationDescriptor, RegisteredOperation};
use infra::database::AuthoritySnapshotV2;
use std::collections::{HashMap, HashSet};

pub(crate) const PLANT_GET_CONTRACT_DIGEST: &str =
    "sha256:7ca72b56a8f32ab247d4d8dc936b30cedf2f9995c1ad8cc5b2f2703dc20f488d";

pub(crate) fn lookup(id: &str) -> Option<RegisteredOperation> {
    match id {
        "plant.get" => {
            let descriptor = OperationDescriptor {
                id: "plant.get",
                version: 1,
                contract_digest: PLANT_GET_CONTRACT_DIGEST,
                provider_id: "patch-data",
                action_id: "patch-data.plant.read",
                resource_type: "plant",
            };
            Some(RegisteredOperation {
                descriptor,
                adapter: DecisionAdapter::new(descriptor),
            })
        }
        _ => None,
    }
}

pub(crate) fn validate_authority_snapshot(snapshot: &AuthoritySnapshotV2) -> anyhow::Result<()> {
    if snapshot.source_id != "patch-data" {
        anyhow::bail!("Patch Data adapter sourceId mismatch");
    }
    let principals = snapshot
        .principals
        .iter()
        .map(|principal| (principal.id.as_str(), principal))
        .collect::<HashMap<_, _>>();
    let active_memberships = snapshot
        .memberships
        .iter()
        .filter(|membership| membership.lifecycle == "active")
        .map(|membership| {
            (
                membership.principal_id.as_str(),
                membership.organization_id.as_str(),
            )
        })
        .collect::<HashSet<_>>();

    for principal in &snapshot.principals {
        if !matches!(
            principal.kind.as_str(),
            "user" | "viewer" | "temporary" | "service"
        ) || principal.external_subject
            != format!("patch-data:{}:{}", principal.kind, principal.id)
        {
            anyhow::bail!("invalid Patch Data principal semantics");
        }
    }
    for membership in &snapshot.memberships {
        let valid_role = match membership.principal_type.as_str() {
            "user" => matches!(membership.role.as_str(), "owner" | "member"),
            "viewer" => membership.role == "viewer",
            "temporary" => membership.role == "temporary",
            _ => false,
        };
        let expected_source = if membership.role == "owner" {
            "organizations.owner".to_owned()
        } else {
            format!("{}:{}", membership.principal_type, membership.principal_id)
        };
        if !valid_role || membership.source_refs != [expected_source] {
            anyhow::bail!("invalid Patch Data membership semantics");
        }
    }
    for binding in &snapshot.effective_bindings {
        if binding.provider_id != "patch-data"
            || binding.action_id != "patch-data.plant.read"
            || binding.resource_type != "plant"
            || !matches!(
                binding.derivation.as_str(),
                "direct"
                    | "organization-owner"
                    | "organization-member"
                    | "descendant"
                    | "ancestor"
                    | "plant-permission"
                    | "temporary-assignment"
                    | "work-order-assignment"
            )
        {
            anyhow::bail!("invalid Patch Data binding semantics");
        }
        let principal = principals
            .get(binding.principal_id.as_str())
            .ok_or_else(|| anyhow::anyhow!("Patch Data binding principal missing"))?;
        if principal.kind == "service" {
            if binding.derivation != "direct" {
                anyhow::bail!("Patch Data service binding must be direct");
            }
        } else if !active_memberships.contains(&(
            principal.id.as_str(),
            binding.application_scope.organization_id.as_str(),
        )) {
            anyhow::bail!("Patch Data human binding requires active membership");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use infra::database::AuthoritySnapshotV2;

    #[test]
    fn plant_get_policy_tuple_is_owned_by_patch_data_catalog() {
        let registered = lookup("plant.get").expect("P0 operation");
        assert_eq!(registered.descriptor.provider_id, "patch-data");
        assert_eq!(registered.descriptor.action_id, "patch-data.plant.read");
        assert_eq!(registered.descriptor.resource_type, "plant");
        assert_eq!(registered.descriptor.version, 1);
        assert_eq!(
            registered.descriptor.contract_digest,
            PLANT_GET_CONTRACT_DIGEST
        );
    }

    fn authority_snapshot() -> AuthoritySnapshotV2 {
        serde_json::from_value(serde_json::json!({
            "schemaVersion": 2,
            "sourceId": "patch-data",
            "generation": 1,
            "sourceRevision": "revision",
            "sourceLedgerDigest": format!("sha256:{}", "a".repeat(64)),
            "sourceWatermark": format!("sha256:{}", "b".repeat(64)),
            "producedAt": "2026-08-26T00:00:00Z",
            "principals": [{"id":"user-a","kind":"user","active":true,"externalSubject":"patch-data:user:user-a"}],
            "organizations": [{"id":"org-a","active":true}],
            "memberships": [{"id":"user-user-a-org-a","principalId":"user-a","organizationId":"org-a","role":"member","principalType":"user","lifecycle":"active","validUntil":null,"sourceRefs":["user:user-a"]}],
            "applications": [],
            "effectiveBindings": [{"principalId":"user-a","applicationScope":{"kind":"wildcard","applicationId":null,"organizationId":"org-a"},"providerId":"patch-data","actionId":"patch-data.plant.read","resourceType":"plant","resourceId":"plant-a","effect":"allow","sourceRuleIds":["user-user-a-org-a"],"derivation":"descendant"}],
            "snapshotDigest": format!("sha256:{}", "0".repeat(64))
        }))
        .unwrap()
    }

    #[test]
    fn patch_data_adapter_owns_source_specific_authority_semantics() {
        let valid = authority_snapshot();
        validate_authority_snapshot(&valid).unwrap();

        let mut wrong_role = valid.clone();
        wrong_role.memberships[0].role = "administrator".to_owned();
        assert!(validate_authority_snapshot(&wrong_role).is_err());

        let mut wrong_derivation = valid.clone();
        wrong_derivation.effective_bindings[0].derivation = "provider-specific-magic".to_owned();
        assert!(validate_authority_snapshot(&wrong_derivation).is_err());

        let mut wrong_subject = valid;
        wrong_subject.principals[0].external_subject = "another-source:user:user-a".to_owned();
        assert!(validate_authority_snapshot(&wrong_subject).is_err());
    }
}
