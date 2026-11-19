# Repository Security hub

The **Security** tab collects static-analysis findings from external CI or rbit Actions.
Only repository writers and administrators can read findings, including in public
repositories. Read-only merge boxes omit private security evidence as well. Administrators configure the scanner and dismiss/reopen findings. This
hub also supports [opt-in native secret and npm dependency scans](security-automation.md), with separately scoped evidence and reviewable dependency update proposals.

Back up before upgrading: migration **0097** adds security settings, scan attempts,
findings and occurrences. All are included in the regular SQLite backup.

## Configure and upload

An administrator opens Security, chooses the exact SARIF `tool.driver.name`, and
optionally configures a trusted default-branch workflow path. The optional merge gate
and minimum severity are configured on the same page. Saving settings changes the
configuration generation: older scan evidence cannot satisfy the new policy.

External CI uses an unrestricted personal token with **admin** scope belonging to a
current repository administrator. A writer token cannot attest clean scan evidence.
Keep the credential in trusted CI secrets, never in a repository or untrusted PR job.

1. Start an attempt with `POST /api/v1/repos/OWNER/REPO/security/scans` and JSON
   `{"commit_sha":"FULL_IMMUTABLE_COMMIT_ID"}`. The commit must already exist in this
   repository; PR heads are available through their managed pull refs. The response
   includes the server-generated scan `id` and state `pending`.
2. Run the configured scanner against those exact bytes. Always initiate the attempt
   **before** scanning, including when retrying a scan.
3. Send the raw SARIF JSON body to
   `POST /api/v1/repos/OWNER/REPO/security/scans/ID/sarif`, authenticated as the same
   administrator or live Actions task that started the attempt. A scan is finalized
   once, as `complete` or `incomplete`. A rejected report leaves its attempt pending.

The API uses ordinary JSON errors and a **1 MiB** body limit. For example, after your
CI has set `API`, `TOKEN` and the immutable `SHA`:

```sh
scan_id=$(curl --fail-with-body --silent --show-error \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  --data "{\"commit_sha\":\"$SHA\"}" "$API/security/scans" | jq -er .id)
# Run the scanner here against SHA, producing report.sarif.
curl --fail-with-body --silent --show-error \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/sarif+json' \
  --data-binary @report.sarif "$API/security/scans/$scan_id/sarif"
```

A late completion cannot supersede a newer attempt: the newest **started** attempt for
that exact commit and current scanner configuration is authoritative, even when pending.
Finalized attempts cannot be replayed or overwritten. The newest 100 attempts are
retained per repository, and at most 10,000 deduplicated findings are retained. Undismissed findings with no retained occurrences are pruned; dismissal history remains. A limit
refusal never creates a clean report; the pending attempt remains visible.

## Supported SARIF

The importer accepts a deliberately bounded subset of
[SARIF 2.1.0](https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/sarif-v2.1.0-errata01-os-complete.html):

- Exactly one run and one named driver per report, at most 1,000 results and rules.
- An explicit `results` array (empty means no findings), and 1–16 explicit `invocations`.
  Every invocation must say `executionSuccessful: true`, without error-level tool
  execution/configuration notifications, for the report to count as complete. Omitted
  success flags, failed invocations and error notifications cannot pass the gate.
- Each result needs a `ruleId`, bounded plain-text message, exactly one physical
  location with a repository-relative URI and positive `startLine`. Artifact indexes
  and `%SRCROOT%` locations are supported. Absolute URLs, external-property references,
  traversal, ambiguous paths and unsupported result kinds are refused. No SARIF URI
  is fetched or executed.
- Security severity scores use the usual boundaries: critical ≥9, high ≥7, medium ≥4,
  low below4. Without a score, SARIF error/warning/note/none map to high/medium/low/info.
  Scores may be on the result or its matching rule. Suppressions in an uploaded report
  do not dismiss repository findings.

Identity is scoped by scanner, rule and normalized path. Supplied `fingerprints` or
`partialFingerprints` preserve identity across moved lines; without them, identity also
includes the line, any supplied column/end positions, and message, so location/message changes can appear as new findings.
Distinct locations are not collapsed merely because they share a rule. Identical
results deduplicate; conflicting occurrences sharing one fingerprint are refused.

## Trusted rbit Actions uploads

Set `permissions: {contents: write}` and expose `secrets.RBIT_TOKEN` (or
`secrets.GITHUB_TOKEN`) to the trusted scanner step. The two scan POST endpoints accept
that task token. They still require all of the following at initiation and completion:

- The task is live, writes its own repository and belongs to a trusted non-fork run.
- The run is not a pull-request run, has no pull association, and executes the configured
  workflow at the **current immutable default-branch tip**.
- The workflow exists there, and its actor still has write permission. Scheduled runs
  without an actor may qualify. Reusable-workflow jobs do not qualify in this release.
- The repository and Actions remain enabled for writes. A stopped task, moved default
  tip or changed security configuration invalidates the upload.

Use a trusted default-branch `workflow_dispatch`, scheduled or push workflow to scan
explicit PR head/base SHAs. The scan target is supplied separately from the workflow's
execution SHA, so that trusted workflow can produce both reports required by the gate.
Do not execute untrusted PR code in a step carrying this credential: the trusted scanner
and its workflow are the attestation authority. Fork/PR task tokens cannot substitute
clean evidence, even after a fork run is approved.

## Triage and merge gate

Each finding links to its exact commit, file and line. The page labels pending/incomplete
reports, scans invalidated by settings changes, and commits different from the current default branch.
Administrators can dismiss as **false positive**, **accepted risk** or **used in tests**,
with a mandatory audit note; reopening restores its blocking effect. Dismissals carry
across scans with the same scanner fingerprint and are audited with actor and reason.

When enabled, the gate requires complete current-generation scans for **both the exact
current PR head and base tip**. It blocks new, undismissed findings at or above the chosen
severity; pre-existing base findings are not treated as newly introduced. Missing,
retained-away, stale, pending or incomplete evidence blocks merging. A changed head/base
needs fresh evidence for that revision. Direct merges and merge-queue landings recheck
the security evidence under a SQLite publication fence immediately before updating refs. The fence excludes scan starts, completions, settings changes and dismissal changes until the native ref update finishes. Admitted publication work survives a cancelled request; shutdown drains it and timeouts kill and reap the native child before releasing the fence.

Ordinary branch-protection bypass does **not** bypass this gate. An administrator may
change or disable it explicitly, which is audited. The gate is a check on trusted scan
evidence, not a guarantee that code is vulnerability-free.

API clients can read `/security/scans`, `/security/scans/ID/findings?offset=0` (50 per
page), and `/security/settings`. Administrators update settings with `PUT` and triage with
`POST /security/findings/ID/dismissal` using `{ "reason": "accepted_risk", "note": "…" }`;
`reason: "reopen"` reopens it. The served OpenAPI document lists every endpoint.
