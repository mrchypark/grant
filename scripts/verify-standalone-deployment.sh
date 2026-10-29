#!/bin/sh
set -eu
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
command -v kubectl >/dev/null
command -v yq >/dev/null
command -v jq >/dev/null
task_dir=$(mktemp -d "${TMPDIR:-/tmp}/grant-standalone.XXXXXX")
trap 'rm -rf "$task_dir"' EXIT HUP INT TERM
kubectl kustomize "$root/deploy/overlays/standalone" > "$task_dir/rendered.yaml"
yq -o=json -I=0 '.' "$task_dir/rendered.yaml" | jq -s '.' > "$task_dir/rendered.json"
jq -e '
  . as $docs |
  [.[] | select(.kind == "StatefulSet")] as $sets |
  [.[] | select(.kind == "ConfigMap" and .data.GRANT_RHIZA_CONFIG_JSON)] as $configs |
  $sets[0] as $s | $s.spec.template.spec as $p | $configs[0].data as $c |
  ($sets | length) == 1 and ($configs | length) == 1 and
  ($docs | length) == 4 and
  all($docs[]; (.kind == "Service" or .kind == "ConfigMap" or .kind == "StatefulSet")) and
  $s.spec.replicas == 1 and $s.spec.updateStrategy.type == "OnDelete" and
  $s.spec.persistentVolumeClaimRetentionPolicy == {whenDeleted:"Retain",whenScaled:"Retain"} and
  $s.spec.volumeClaimTemplates[0].metadata.name == "data" and
  $s.spec.volumeClaimTemplates[0].spec.accessModes == ["ReadWriteOnce"] and
  $p.terminationGracePeriodSeconds >= 45 and
  $p.nodeSelector["kubernetes.io/arch"] == "amd64" and
  $p.securityContext.runAsUser == 10001 and
  ($p.initContainers | length) == 1 and ($p.containers | length) == 1 and
  $p.initContainers[0].command == ["/usr/local/bin/grant","restore-local-storage"] and
  $p.initContainers[0].volumeMounts == $p.containers[0].volumeMounts and
  $p.containers[0].command == ["/usr/local/bin/grant"] and
  all(($p.initContainers + $p.containers)[];
    .image == "ghcr.io/mrchypark/grant:v0.4.0" and .imagePullPolicy == "IfNotPresent" and
    .envFrom == $p.containers[0].envFrom and
    .securityContext.readOnlyRootFilesystem == true and
    any(.volumeMounts[]; .name == "data" and .mountPath == "/var/lib/grant")) and
  $p.containers[0].readinessProbe.httpGet.path == "/readyz" and
  $p.containers[0].startupProbe.httpGet.path == "/healthz" and
  all($docs[] | select(.kind == "Service"); .spec.selector == $s.spec.selector.matchLabels and
    all(.spec.ports[]; .port == 8080 and .targetPort == "http")) and
  $c.GRANT_RUNTIME_MODE == "embedded" and $c.GRANT_LOCAL_NODE_ROLE == "voter" and
  $c.RHIZA_BIND_ADDR == "" and $c.GRANT_PEER_ADDR == "" and
  $c.RHIZA_OBJSTORE_DURABILITY == "before-ack" and
  ($c.GRANT_RHIZA_CONFIG_JSON | fromjson) ==
    {Local:true,NodeID:"grant-standalone",ClusterID:"grant-standalone-v1",Members:[],PeerAddr:""} and
  $c.GRANT_NODE_ID == "grant-standalone" and $c.GRANT_CLUSTER_ID == "grant-standalone-v1"
' "$task_dir/rendered.json" >/dev/null
# Compile this test through Go overlay; never write into the dirty app package.
command -v go >/dev/null
jq '[.[] | select(.kind == "ConfigMap" and .data.GRANT_RHIZA_CONFIG_JSON)][0].data' "$task_dir/rendered.json" > "$task_dir/config.json"
cp "$root/scripts/testdata/standalone/config_test.go" "$task_dir/config_test.go"
printf '\nconst standaloneConfigFixture = "%s/config.json"\n' "$task_dir" >> "$task_dir/config_test.go"
jq -n --arg virtual "$root/internal/grant/standalone_rendered_contract_test.go" --arg source "$task_dir/config_test.go" '{Replace:{($virtual):$source}}' > "$task_dir/overlay.json"
(cd "$root" && go test -overlay "$task_dir/overlay.json" ./internal/grant -run '^TestStandaloneRenderedConfig$' -count=1 -v)
(cd "$root" && go test ./cmd/grant -run '^TestRestoreLocalStorageEntrypoint$' -count=1 -v -timeout 2m)
printf '%s\n' 'Standalone render, ConfigFromEnv and restore command contract passed'
