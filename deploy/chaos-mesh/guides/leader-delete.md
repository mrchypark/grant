# Leader Delete Procedure

Use this procedure when you want a real `leader-delete` experiment instead of killing
an arbitrary voter pod.

## Why this is not a static selector

`Chaos Mesh` selectors can target pods by label, field, or explicit pod list. They
cannot infer the current hiqlite leader from Raft state. In this project, leader
selection is dynamic and must be discovered at runtime.

That is why this repo provides a template YAML instead of a fake `leader-delete` label
selector.

## Step 1: identify the current leader

Query each voter pod and inspect the hiqlite state exposed by the app. For example,
port-forward each voter service in turn and check the health or metrics output:

```bash
kubectl --context gke_ied-cluster -n grant-chaos port-forward svc/grant-voter-0 18080:8080
curl -fsS http://127.0.0.1:18080/healthz
```

Repeat for `grant-voter-1` and `grant-voter-2`, then choose the pod currently
reported as leader.

## Step 2: patch the template with the pod name

Copy the template and replace the placeholder:

```bash
sed 's/REPLACE_WITH_LEADER_POD/grant-voter-0/' \
  deploy/chaos-mesh/pod/leader-delete-template.yaml \
  >/tmp/leader-delete.yaml
```

For a follower experiment, do the same with
[pod/follower-delete-template.yaml](../pod/follower-delete-template.yaml).

## Step 3: apply and watch recovery

```bash
kubectl --context gke_ied-cluster apply -f /tmp/leader-delete.yaml
kubectl --context gke_ied-cluster -n grant-chaos get pod -w
```

## Step 4: validate application behavior

Expected behavior:

- `leader-delete`: a short failover window is possible, then recovery
- `follower-delete`: usually less visible disruption

Recommended checks after the fault:

- `/api/v1/me`
- `/api/v1/providers`
- `/api/v1/authz/check`

## Cleanup

Delete the experiment after the test:

```bash
kubectl --context gke_ied-cluster -n grant-chaos delete podchaos grant-voter-leader-delete
```
