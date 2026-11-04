#!/bin/sh
set -eu
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
for tool in kubectl yq jq go; do command -v "$tool" >/dev/null; done
task_dir=$(mktemp -d "${TMPDIR:-/tmp}/grant-nopvc.XXXXXX")
trap 'rm -rf "$task_dir"' EXIT
trap 'exit 130' HUP INT TERM
kubectl kustomize "$root/deploy/overlays/standalone-nopvc" > "$task_dir/rendered.yaml"
yq -o=json -I=0 '.' "$task_dir/rendered.yaml" | jq -s '.' > "$task_dir/rendered.json"
jq -e '
 . as $docs |
 [.[] | select(.kind == "StatefulSet")] as $sets |
 [.[] | select(.kind == "ConfigMap")] as $configs |
 $sets[0] as $s | $s.spec.template.spec as $p | $configs[0].data as $c |
 ($docs | length) == 4 and ($sets | length) == 1 and ($configs | length) == 1 and
 all($docs[]; .kind == "Service" or .kind == "ConfigMap" or .kind == "StatefulSet") and
 $s.spec.replicas == 1 and $s.spec.updateStrategy.type == "OnDelete" and
 ($s.spec | has("volumeClaimTemplates") | not) and
 ($s.spec | has("persistentVolumeClaimRetentionPolicy") | not) and
 ($p.volumes | length) == 2 and
 all($p.volumes[]; has("emptyDir") and (has("persistentVolumeClaim") | not) and (has("hostPath") | not)) and
 ([$p.volumes[] | select(.name == "data")][0].emptyDir == {sizeLimit:"10Gi"}) and
 all(["node.kubernetes.io/not-ready", "node.kubernetes.io/unreachable"][];
   . as $key | any($p.tolerations[];
    .key == $key and .operator == "Exists" and .effect == "NoExecute" and
    (has("tolerationSeconds") | not))) and
 $p.automountServiceAccountToken == false and
 $p.securityContext.runAsUser == 10001 and $p.securityContext.runAsGroup == 10001 and
 $p.securityContext.runAsNonRoot == true and $p.securityContext.fsGroup == 10001 and
 $p.nodeSelector["kubernetes.io/arch"] == "amd64" and $p.terminationGracePeriodSeconds == 60 and
 ($p.initContainers | length) == 1 and ($p.containers | length) == 1 and
 $p.initContainers[0].command == ["/usr/local/bin/grant","restore-local-storage"] and
 $p.containers[0].command == ["/usr/local/bin/grant"] and
 $p.initContainers[0].volumeMounts == $p.containers[0].volumeMounts and
 $p.initContainers[0].envFrom == $p.containers[0].envFrom and
 ($p.containers[0] | has("env") | not) and ($p.initContainers[0] | has("env") | not) and
 ($p.containers[0].envFrom | length) == 3 and
 $p.containers[0].envFrom[0].secretRef.name == "grant-secrets" and
 $p.containers[0].envFrom[1].secretRef.name == "grant-rhiza-storage" and
 $p.containers[0].envFrom[2].configMapRef.name == $configs[0].metadata.name and
 all(($p.initContainers + $p.containers)[];
   .image == "ghcr.io/mrchypark/grant:v0.7.0" and .imagePullPolicy == "IfNotPresent" and
   .securityContext.readOnlyRootFilesystem == true and
   .securityContext.allowPrivilegeEscalation == false and .securityContext.capabilities.drop == ["ALL"] and
   any(.volumeMounts[]; .name == "data" and .mountPath == "/var/lib/grant")) and
 $p.containers[0].readinessProbe.httpGet.path == "/readyz" and
 $p.containers[0].startupProbe.httpGet.path == "/healthz" and
 all($docs[] | select(.kind == "Service"); .spec.selector == $s.spec.selector.matchLabels) and
 $c.RHIZA_OBJSTORE_PROVIDER == "s3" and $c.RHIZA_OBJSTORE_DIR == "" and $c.RHIZA_OBJSTORE_INSECURE == "false" and
 ($c | has("RHIZA_OBJSTORE_DURABILITY") | not) and
 ($c.GRANT_RHIZA_CONFIG_JSON | fromjson) ==
 {Local:true,NodeID:"grant-standalone",ClusterID:"grant-standalone-v1",Members:[],PeerAddr:""}
' "$task_dir/rendered.json" >/dev/null
jq '[.[] | select(.kind == "ConfigMap")][0].data' "$task_dir/rendered.json" > "$task_dir/config.json"
cp "$root/scripts/testdata/standalone-nopvc/config_test.go" "$task_dir/config_test.go"
if [ -n "${STORAGE_ENV_FILE:-}" ]; then
 [ -s "$STORAGE_ENV_FILE" ] || { echo 'STORAGE_ENV_FILE must be nonempty' >&2; exit 1; }
 cp "$STORAGE_ENV_FILE" "$task_dir/secret.json"
else
 : > "$task_dir/secret.json"
fi
printf '\nconst noPVCSecretFixture = "%s/secret.json"\n' "$task_dir" >> "$task_dir/config_test.go"
printf '\nconst noPVCConfigFixture = "%s/config.json"\n' "$task_dir" >> "$task_dir/config_test.go"
jq -n --arg virtual "$root/internal/grant/nopvc_rendered_contract_test.go" --arg source "$task_dir/config_test.go" '{Replace:{($virtual):$source}}' > "$task_dir/overlay.json"
(cd "$root" && go test -overlay "$task_dir/overlay.json" ./internal/grant -run '^TestNoPVC(RemoteRenderedConfig|ProvidedConfig)$' -count=1 -v -timeout 2m)
printf '%s\n' 'NoPVC render and synthetic remote-S3 ConfigFromEnv passed; no Kubernetes/cloud/fencing qualification'
