# Workflow cloud identities (OIDC)

Actions jobs can request a short-lived, signed identity and exchange it with a cloud
provider for credentials. This avoids storing a long-lived cloud key in repository
secrets. rbit issues the identity; you configure the provider, trust policy and resource
permissions separately. This feature is independent of OIDC user sign-in.

## Enable identity for a job

Enable Actions on the instance and repository, and configure the server's public URL
with HTTPS. The issuer is that configured URL followed by `/api/actions/oidc`; request
headers cannot change it. Discovery and public keys must be reachable by your provider:

```text
https://git.example.com/api/actions/oidc/.well-known/openid-configuration
https://git.example.com/api/actions/oidc/jwks
```

Update rbit-runner alongside the server before enabling identity permission; older runners
refuse the new job fields. Declare `id-token: write` explicitly in the workflow or job. It permits requesting an
identity, not writing repository contents or accessing cloud resources. Other required
permissions must also be listed because a `permissions` map replaces defaults:

```yaml
name: Deploy
on:
  push:
    branches: [main]
jobs:
  deploy:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      id-token: write
    environment: production
    env:
      ROLE_ARN: arn:aws:iam::123456789012:role/rbit-deploy
    steps:
      - name: Assume the deployment role
        shell: bash
        run: |
          set -euo pipefail
          umask 077
          identity_file=$(mktemp)
          trap 'rm -f "$identity_file"' EXIT
          python3 - "$identity_file" <<'PYTHON'
          import json, os, sys, urllib.parse, urllib.request
          url = os.environ["ACTIONS_ID_TOKEN_REQUEST_URL"]
          url += "&" + urllib.parse.urlencode({"audience": "sts.amazonaws.com"})
          request = urllib.request.Request(url, headers={
              "Authorization": "Bearer " + os.environ["ACTIONS_ID_TOKEN_REQUEST_TOKEN"]
          })
          with urllib.request.urlopen(request, timeout=30) as response:
              identity = json.load(response)["value"]
          with open(sys.argv[1], "w", encoding="utf-8") as output:
              output.write(identity)
          PYTHON
          aws sts assume-role-with-web-identity \
            --role-arn "$ROLE_ARN" \
            --role-session-name "rbit-$GITHUB_RUN_ID" \
            --web-identity-token "file://$identity_file" \
            --query 'AssumedRoleUser.Arn' --output text
```

This example requires Bash, Python 3 and the AWS CLI in the job image, plus an IAM role
configured as described below. Replace `ROLE_ARN` with your role. It completes the exchange
and prints only the assumed role ARN; it does not persist cloud credentials for subsequent
steps. The task credential stays out of process arguments, and the JWT is written only
to a temporary file restricted to its owner and removed when the step exits. Do not log
either credential or upload that file as an artifact. The runner supplies the request
URL (including `?api-version=1`) and task credential only to eligible jobs. Personal access tokens, page sessions and runner
registration credentials cannot request identities. Default workflow permissions never
grant OIDC; `id-token: none` disables it and `id-token: read` is invalid. Explicit
`write-all` includes identity permission. Reusable workflows can reduce the caller's
ceiling, never elevate it; permission must survive the entire call chain.

Only trusted, non-fork, non-pull-request runs qualify, including trusted scheduled runs.
Even a same-repository pull request cannot mint a token. Every request rechecks the
running task, current task credential, runner status and heartbeat, run cancellation,
repository/owner Actions policy, archival state and ownership tenure. Original and
rerun actors must still be active writers, except an actorless original scheduled run.
Environment branch rules and required approvals are checked again, including reviewers'
current access and self-review restrictions. Removing access stops new issuance.

## Trust the stable subject and exact audience

The provider must validate signature, issuer, expiration and the exact audience, then
restrict the subject to the intended repository and ref or environment. Names and numeric
database IDs are descriptive claims, not safe identity anchors. The stable subject is:

```text
repo:<repository_uid>:owner:<repository_owner_uid>:ref:refs/heads/main
repo:<repository_uid>:owner:<repository_owner_uid>:environment:production
```

Both UIDs are 32 hexadecimal characters. Renaming preserves them; transferring a repository
rotates its repository UID, so old cloud trust cannot silently follow a new owner. Deleting
and recreating an account or repository also creates a new identity. Identities are allocated
lazily when Actions first creates a run for that owner and repository; migration and ordinary
account/repository creation do not populate the identity tables. Runs capture those UIDs
in the same insert and cannot issue after a transfer. Runs created before migration 0098
have no captured identity and must be replaced by a new run.

A job with an environment uses the environment subject instead of its ref. Restrict its
allowed branches in rbit, and add a cloud `ref` condition where supported if needed.
Scope bytes other than ASCII letters, digits, `-_.~/` are percent-encoded; for example,
`production:eu` becomes `production%3Aeu`. The entire subject is limited to 255 bytes.
Use the exact subject from a verified token from your intended trusted job when setting
up trust; decoding a JWT alone does not verify it.

### AWS IAM

AWS supports ES256 OIDC identities and issuer paths. Create an IAM OIDC provider for
`https://git.example.com/api/actions/oidc`, with `sts.amazonaws.com` as its audience.
Use an exact trust policy on the intended role, replacing all placeholders:

```json
{
  "Version": "2012-10-17",
  "Statement": [{
    "Effect": "Allow",
    "Principal": {
      "Federated": "arn:aws:iam::<ACCOUNT_ID>:oidc-provider/git.example.com/api/actions/oidc"
    },
    "Action": "sts:AssumeRoleWithWebIdentity",
    "Condition": {
      "StringEquals": {
        "git.example.com/api/actions/oidc:aud": "sts.amazonaws.com",
        "git.example.com/api/actions/oidc:sub": "repo:<REPOSITORY_UID>:owner:<OWNER_UID>:environment:production"
      }
    }
  }]
}
```

Attach only the resource permissions that deployment needs to that role. The trust policy
alone grants no AWS resource permissions. Follow AWS's [provider setup](https://docs.aws.amazon.com/IAM/latest/UserGuide/id_roles_providers_create_oidc.html)
and [OIDC role trust guidance](https://docs.aws.amazon.com/IAM/latest/UserGuide/id_roles_create_for-idp_oidc.html)
for the token exchange and account-specific configuration.

### Google Cloud Workload Identity Federation

Configure an OIDC workload identity pool provider with the same issuer. Google supports
ES256. Request the audience your provider accepts; its default is the canonical provider
URL, including the project **number**:

```text
https://iam.googleapis.com/projects/PROJECT_NUMBER/locations/global/workloadIdentityPools/POOL_ID/providers/PROVIDER_ID
```

Map `google.subject=assertion.sub` and set an attribute condition matching the exact
subject, for example:

```text
assertion.sub == 'repo:<REPOSITORY_UID>:owner:<OWNER_UID>:environment:production'
```

Google limits `google.subject` to 127 characters, so use a short ref/environment whose
full subject fits that bound. Configure the provider's audience explicitly if using
something other than its default. Grant only the intended federated principal access
to resources or service-account impersonation. See Google's [OIDC provider setup](https://docs.cloud.google.com/iam/docs/workload-identity-federation-with-other-providers)
and [attribute mapping limits](https://docs.cloud.google.com/iam/docs/workload-identity-federation).

These are provider configuration examples, not automated provisioning or a claim that
every GitHub cloud-login action works unchanged. rbit uses its own issuer, claims and
ES256 signing; check an action's issuer and algorithm assumptions before using it.

## Claims and lifetime

The JWT header contains `alg: ES256`, `typ: JWT` and the published key's `kid`.

| Claims | Meaning |
| --- | --- |
| `iss`, `sub`, `aud` | Configured issuer, stable scoped subject and requested audience |
| `iat`, `nbf`, `exp`, `jti` | Unix-second issuance/not-before/expiry times and a random token identifier |
| `repository_uid`, `repository_owner_uid` | Stable identity values used in the subject |
| `repository`, `repository_owner` | Current `owner/name` and owner name; descriptive only |
| `repository_id`, `repository_owner_id` | Numeric database IDs represented as strings; descriptive only |
| `ref`, `sha`, `event_name` | Full run ref, commit and triggering event |
| `workflow_ref`, `workflow_sha` | Entrypoint `owner/repository/path@ref` and its run commit |
| `run_id`, `run_number`, `run_attempt`, `job_id`, `task_id` | String-valued execution identifiers |
| `environment` | Environment name, present for an environment job |
| `actor_id`, `trigger_actor_id` | Original and current-attempt actor IDs as strings, omitted when absent |

Workflow claims identify the **entrypoint** workflow, including when it calls reusable
workflows. They do not attest the called workflow's file identity; there are no
`job_workflow_ref`/`job_workflow_sha` claims to trust in this release.

Tokens last at most five minutes, shortened to the remaining task timeout. Cancellation,
access changes or runner disablement prevent new tokens but cannot revoke an already
issued JWT or an exchanged cloud session. Configure cloud session duration separately.

## HTTP contract and operations

The [standalone OpenAPI document](actions-oidc.openapi.json) describes the three GET
endpoints. They sit outside `/api/v1` and remain available when the general REST API is
disabled, provided Actions is enabled. Discovery and JWKS are public and cacheable for
300 seconds; token responses and errors use `Cache-Control: no-store`.

`GET /api/actions/oidc/token?api-version=1&audience=...` accepts the current task bearer
credential and returns `{"value":"<signed JWT>"}`. Both query parameters are optional;
the default audience is the issuer, which is usually not the cloud's expected audience.
Only API version `1` is accepted. An audience must be 1–512 UTF-8 bytes without controls
or surrounding whitespace. Unknown parameters are rejected.

Missing authorization returns 401; an unknown/revoked task credential returns 404; a
live policy refusal or the limit of 60 successful issuances per task per UTC minute
returns 403. Invalid query values return 422. IP throttling returns 429. Request
timeouts return 503. Request another token only when needed, and never log either the
task credential or the resulting JWT.

Signing keys rotate automatically in 30-day Unix-epoch periods. JWKS publishes the
current and next key, plus the previous key when present. Refresh provider caches on
an unknown `kid`. Keep the server clock accurate; clock rollback cannot overwrite
published future key material.

Private signing keys are sealed in SQLite with the existing Actions secrets key.
Back up the database and separately secure a matching copy of
`<data_dir>/actions/secrets.key`, or the configured `[actions] secrets_key_file`.
Restrict its filesystem permissions. Losing that key prevents signing with stored
keys; rbit does not silently replace it. Preserve the issuer URL and matching key/data
backup during recovery. Migration 0098 adds identity, signing-key and issuance tables;
take a pre-upgrade backup for rollback instead of running an older binary on the
migrated database.
