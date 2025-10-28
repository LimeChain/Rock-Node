# Rock Node Helm Chart

This chart deploys the Rock Node service together with the configuration,
persistent storage, and observability endpoints required to run the data
availability node inside Kubernetes.

## Installing

```bash
helm install rock-node charts/rock-node \
  --namespace rock-node \
  --create-namespace
```

Override image repository, tag, and any environment specific configuration as
needed:

```bash
helm upgrade --install rock-node charts/rock-node \
  --namespace rock-node \
  --set image.repository=ghcr.io/limechain/rock-node \
  --set image.tag=latest
```

To expose the service on fixed TCP ports reachable from other machines on your
LAN, switch the Service to `NodePort` and assign port numbers:
```bash
helm upgrade --install rock-node charts/rock-node \
  --namespace rock-node \
  --set service.type=NodePort \
  --set service.grpc.nodePort=23513 \
  --set service.metrics.nodePort=23514
```

This exposes:

- `http://<k3s-node-ip>:23513` (TCP) → Rock Node gRPC endpoint
- `http://<k3s-node-ip>:23514` → Observability `/metrics`, `/readyz`, `/livez`

## Argo CD Application

To have Argo CD reconcile this chart automatically, point the Application
manifest at the chart directory and enable automated sync:

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: rock-node
  namespace: argocd
spec:
  project: default
  source:
    repoURL: git@github.com:LimeChain/Rock-Node.git
    targetRevision: main
    path: charts/rock-node
  destination:
    server: https://kubernetes.default.svc
    namespace: rock-node
  syncPolicy:
    automated:
      prune: true
      selfHeal: true
    syncOptions:
      - CreateNamespace=true
```

When releasing, publish a container image for Rock Node and bump the chart’s
`appVersion`/`image.tag`. Pushing that change to the Git repository Argo CD
watches will trigger a sync and roll out the new version automatically.
