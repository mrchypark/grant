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

Use the same approach as the existing chaos harness: query each voter pod and inspect
the hiqlite state exposed by the app.

One practical option is to reuse the existing script:

```bash
KUBE_CONTEXT=gke_ied-cluster \
NAMESPACE=certd-chaos \
CERTD_MASTER_KEY=<master-key> \
bash /Users/cypark/Documents/work/account/scripts/gke-chaos.sh
```

The script already resolves leader and follower pods before the `leader-delete` and
`follower-delete` phases. If you want a manual check, use a local port-forward and read
the hiqlite state from the voter `/healthz` or metrics path that the current harness
uses.

## Step 2: patch the template with the pod name

Copy the template and replace the placeholder:

```bash
sed 's/REPLACE_WITH_LEADER_POD/certd-voter-0/' \
  /Users/cypark/Documents/work/account/deploy/chaos-mesh/pod/leader-delete-template.yaml \
  >/tmp/leader-delete.yaml
```

For a follower experiment, do the same with
[pod/follower-delete-template.yaml](/Users/cypark/Documents/work/account/deploy/chaos-mesh/pod/follower-delete-template.yaml).

## Step 3: apply and watch recovery

```bash
kubectl --context gke_ied-cluster apply -f /tmp/leader-delete.yaml
kubectl --context gke_ied-cluster -n certd-chaos get pod -w
```

## Step 4: validate application behavior

Expected behavior:

- `leader-delete`: a short failover window is possible, then recovery
- `follower-delete`: usually less visible disruption
- `learner` local reads should continue
- `learner` `?consistency=strong` reads should still work unless quorum or routing is
  impacted by a separate fault

Recommended checks after the fault:

- `/api/v1/me`
- `/api/v1/plants`
- `/api/v1/plants/<facility-code>`
- `learner` read with and without `?consistency=strong`

## Cleanup

Delete the experiment after the test:

```bash
kubectl --context gke_ied-cluster -n certd-chaos delete podchaos certd-voter-leader-delete
```
