# CI/CD Pipeline and the Kubernetes Cluster

The CI/CD Pipeline deploys to the Kubernetes Cluster on every
merge to `main`. A successful pipeline produces a versioned
container image, runs the platform's integration suite against an
ephemeral namespace, then promotes the image to the Kubernetes
Cluster's production namespace through a rolling update.

The CI/CD Pipeline manages its own state in a small Postgres
instance — pipeline runs, gate decisions, and the manifest of
which image was deployed when. That state survives a pipeline
restart and is what the rollback procedure reads from.

To roll back a deployment on the Kubernetes Cluster, the operator
runs `pipeline rollback <run-id>`. The CI/CD Pipeline looks up the
previous successful run for the same service, fetches the manifest
that pipeline produced, and re-applies it to the Kubernetes
Cluster's production namespace. The rollback uses the same rolling
update strategy as a forward deploy, so there is no downtime
window. If the previous run is older than the current image's
retention window, the rollback fails fast and the runbook escalates
to the platform team.
