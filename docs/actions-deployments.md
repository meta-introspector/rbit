# Deployment history, promotion and rollback

Open **Deployments** from a repository's Actions page. Anyone who can read its Actions
can see environment jobs in progress, completed attempts and promotion/rollback requests.
The page reports workflow outcomes and links reported by a runner. An observed environment
link is a recorded URL; rbit does not probe it or claim that the environment is healthy.

Completed job attempts are immutable. Rerunning an ordinary Actions job creates another
attempt; its earlier SHA, environment, outcome and observed URL remain in history. A late
runner update cannot write a URL into a newer attempt. History survives ordinary workflow
run/log/artifact retention, while links to removed runs disappear. Deleting a repository
removes its deployment history too.

## Configure a trusted deployment workflow

A repository administrator first creates source and target environments in
**Settings → Actions → Environments**, including their reviewers, self-review restrictions,
wait timers and branch/tag policies. On Deployments, the administrator then saves a mapping
from a source environment, such as `staging`, to a target, such as `production`, and a
workflow path. The environments must differ and already exist.

The workflow must support `workflow_dispatch` and declare these string inputs:

- `deployment_sha`: the full immutable SHA of the selected successful source attempt.
- `deployment_environment`: the configured target environment.
- `deployment_operation`: `promote` or `rollback`.
- `artifact_id` and `artifact_digest`, when requests will select an artifact (both optional
  string inputs). The digest is the recorded archive's SHA-256 as 64 hexadecimal digits.

For example, this current-default-branch workflow receives the selected identity and
passes through the target environment gate. Add the actual deployment steps appropriate
for your infrastructure; this example only reports the requested identity.

```yaml
name: Deploy selected source
on:
  workflow_dispatch:
    inputs:
      deployment_sha: {type: string, required: true}
      deployment_environment: {type: string, required: true}
      deployment_operation: {type: string, required: true}
      artifact_id: {type: string}
      artifact_digest: {type: string}
jobs:
  deploy:
    runs-on: ubuntu-latest
    environment: ${{ inputs.deployment_environment }}
    env:
      DEPLOYMENT_SHA: ${{ inputs.deployment_sha }}
      DEPLOYMENT_OPERATION: ${{ inputs.deployment_operation }}
    steps:
      - run: printf 'Requested %s of %s\n' "$DEPLOYMENT_OPERATION" "$DEPLOYMENT_SHA"
```

At least one planned job must name the selected target environment, and named environment
jobs must use that target. The new run follows the current environment's approval and wait
timer rules and the ordinary runner claim/branch policy checks. Noncanceling concurrency
queues normally. A deployment request that would cancel another workflow run through its
concurrency group is refused with `409`; wait for that run or cancel it explicitly before
requesting deployment. A refused request leaves other runs untouched. Reusable workflows must come from this same current default-branch commit; external, fork
or historical reusable workflow definitions are refused. Jobs must be fully planned from
dispatch inputs; job plans deferred until `needs` outputs are available are unsupported.
Workflow authors remain responsible for deployment steps, artifact verification and their
own infrastructure.

## Select a source

A current repository writer selects one completed attempt and the configured target.
Promotion requires a successful trusted attempt in the configured source environment.
Rollback requires a successful trusted attempt in the target environment. Fork and pull
request attempts cannot supply deployment identity. A source commit must still be
available in the repository.

Optionally provide an artifact ID and its recorded digest together. It must be an
unexpired artifact from the exact selected source run attempt, produced by a successful
job of that attempt. An artifact from another SHA/run/attempt or a mismatched digest is
refused. Expired or removed artifacts cannot be selected.

The server reads and plans the administrator-selected workflow from the repository's
**current own default branch**, then checks current writer access, configuration identity
and generation, source/artifact provenance, workflow availability and the exact default
branch name/tip again under the final publication fence. It never dispatches a historical
source workflow or a fork workflow. A moved default branch or changed configuration
refuses the request so the writer can review it again.

A completed deployment records its logical deployed source separately from its workflow
commit. For example, promoting source B with current workflow A produces a production
attempt for logical source B; rolling back that attempt still selects B, rather than A.
The original producing attempt and artifact ID/digest are retained as provenance even when
run/artifact retention removes their live records. Explicit selection of a removed or
expired artifact is refused. A SHA-only rollback can still select B and keeps the original
artifact provenance without claiming unavailable artifact bytes were selected again.

The selected source SHA is an explicit input; the new run's own SHA identifies the
current workflow definition. The dashboard separates those identities and records the
actor, operation, target, exact source attempt, optional artifact identity/digest and new
run together. A requested SHA alone is not proof that an external deployment completed;
review the workflow outcome and your infrastructure's own observations.

Intent-bound deployment runs cannot use generic Actions rerun controls. Submit a new
promotion or rollback request instead, so rbit evaluates the current trusted workflow and
current authority/gates again. Ordinary workflow reruns retain their existing behavior.

## REST API

- `GET /api/v1/repos/{owner}/{repo}/deployments?before={attempt_id}` returns a bounded page
  of completed attempts, current unfinished environment jobs, configured workflows and
  recent requests. `next_before` pages through older completed attempts.
- `PUT /api/v1/repos/{owner}/{repo}/deployments/workflows/{environment}` accepts
  `source_environment` and `workflow`. It requires admin token scope and current repository
  administration, and returns stable configuration `id` and `generation` values.
- `POST /api/v1/repos/{owner}/{repo}/deployments/requests` accepts `environment`,
  `operation`, `source_attempt`, `configuration_id`, `configuration_generation`, and
  optional paired `artifact_id`/`artifact_digest`. It requires write token scope, the
  Dispatch right and current repository write access. Success is `201` with the new run
  identity. Changed snapshots/configuration return `409`, invalid provenance/input `422`.

These endpoints and pages exist only when Actions is enabled for the instance; reads
also require the repository's Actions switch and normal repository visibility.

## Upgrade

Migration **0102** adds immutable completed attempt history, deployment workflow mappings
and request records. Take a pre-upgrade database backup; restore that backup to roll back
the schema. The migration backfills currently stored completed jobs. Older attempts that
had already been overwritten by reruns or deleted by retention cannot be reconstructed.
It enables no workflow mapping or automatic deployment. History is small metadata retained
beyond run retention; administrators should account for it in database capacity and backups.
