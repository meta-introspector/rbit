# Automatic security scans and dependency updates

The repository **Security** page offers opt-in native secret and npm dependency scans.
Only writers can read findings, inventory and update history; administrators configure
scanning, request scans and create updates. Back up before migration **0100**, which adds
scanner channels, durable jobs, fingerprint keys and update mappings to SQLite.

## Enable scanning

Open **Automatic scans → Scan settings**. Enable either scanner, save, and choose
**Scan current default branch**. Pushes and pull-request updates request fresh evidence;
a bounded reconciliation pass also finds missed events. Unchanged revisions are rescanned
after 12 hours. Each repository reconciliation considers its newest 100 open pull requests;
older requests can use the explicit scan endpoint for their exact head and base.

Secret scanning runs without network access. Dependency scanning stays off until an
administrator explicitly consents to sending **public npm package names and versions** to
`api.osv.dev` and `registry.npmjs.org`. No source files, secret values, repository names or
private-registry credentials are sent. Private registries are unsupported. DNS resolves
through the existing public-address policy; redirects and environment proxies are disabled.
Responses, requests and elapsed time are bounded. Provider failures are incomplete scans,
never evidence that no vulnerabilities exist.

Native secret, native npm and external SARIF evidence occupy separate channels. Required
channels each need current-generation complete scans of the exact PR head and base.
Native evidence expires after 24 hours, including when Git has not changed. A clean external
upload cannot replace a missing native result. Settings changes invalidate native evidence;
stopped, failed, unsupported, expired and retained-away evidence cannot satisfy the gate.
Existing audited dismissals and the final publication fence apply to native findings too.

## What is inspected

Secret scanning reads UTF-8 regular Git blobs at an immutable commit. It does not check out
files, run scripts, follow symlinks, scan binary files, recurse into submodules or inspect Git
history. Initial detectors recognize the common `ghp_` classic and `github_pat_` fine-grained
personal-token shapes and standalone PEM/OpenSSH private-key headers. This is bounded
pattern detection, not provider validation or a guarantee that a credential is active.
Unsupported providers and token formats are not covered. Rotate or revoke a detected
credential with its provider: deleting it from a file does not revoke it.

Findings retain detector, file, line and a repository-keyed digest. Matched credential
values and source snippets are never stored in findings or emitted to logs. The per-repository
fingerprint key lives in SQLite and is included in backups. Moving a credential within the
same file preserves its identity; changing the credential changes the identity.

The text walk is capped at 10,000 entries, 64 MiB total data, 1 MiB per tree/blob and five
seconds of traversal. Exceeding coverage bounds produces an incomplete scan. At most
1,000 findings are stored in a report. Binary files, links and submodules are explicitly
outside text scope.

Dependency scanning accepts a root `package-lock.json` version 3, up to 2 MiB and 1,000
package entries, with exact versions resolved from the public npm registry. Workspace
links, aliases, Git/file dependencies, malformed input and other lock formats are
incomplete coverage. A root `package.json` without a supported lockfile is incomplete;
absence of both files means the root npm check is not applicable. Nested projects and
other ecosystems are not inspected in this release.

[OSV exact-version queries](https://google.github.io/osv.dev/post-v1-querybatch/) identify
advisories. Batches are limited to 100 packages; oversized, malformed or paginated results
are incomplete rather than silently truncated. Findings use **high** severity conservatively
because provider severity vectors are not normalized in this release. Advisory identities
are scoped to package installation location and advisory, so repeated installations remain
distinct. No advisory-linked URL is fetched.

## Dependency update proposals

Select a complete current default-branch npm scan, choose a direct package, and use
**Propose dependency update**. The server looks for a newer stable patch in the same
major/minor release line, checks that OSV reports no advisory for the candidate, and
validates registry metadata. It changes only `package.json` and `package-lock.json` in one
commit on a dedicated branch, then opens an ordinary pull request. No package manager,
package installation or repository lifecycle script runs on the server.

The resolver-free first release supports **exact-pinned direct leaf dependencies** only.
Multiple installed copies, transitive dependants, peer/optional/bundled dependencies,
workspaces, overrides, aliases, install scripts, deprecated versions and updates requiring
new dependency resolution need manual repair. Unsupported proposals return a readable
error; they do not pretend to fix the finding or weaken its scan state.

Update branches use create-only Git CAS with a final current-authority/configuration/base
check. Existing branches are never force-updated. Retry recovery accepts only the exact
proposed two-file tree, unchanged regular-file modes and one direct parent at the scanned base; a human-edited branch produces a conflict. Proposals are bounded
to 100 stored mappings per repository. A proposal may require manual completion if the
branch was committed but later pull-request creation failed. Retrying reconnects a single
exact matching pull request, including a closed one; ambiguous or edited pull requests need
manual review. Normal CI/review/merge requirements remain authoritative.

**Pull requests inherit repository visibility**, including public visibility. Their titles
and bodies contain generic package updates, never detected secret values or private finding
details. Creating automatic update PRs is a separate, default-off option with this disclosure
shown before saving. Updates never auto-merge. Automated proposals consider up to five
direct packages per completed default-branch scan; unsupported repairs remain manual.

## API and optional workflow

All paths are below `/api/v1/repos/OWNER/REPO/security/automation`:

| Method and path | Meaning |
| --- | --- |
| `GET`, `PUT /` | Read or replace opt-in settings |
| `POST /scans` | Queue native scans with `{"commit_sha":"FULL_SHA"}` |
| `GET /inventory?commit_sha=FULL_SHA` | Read the supported public npm inventory |
| `GET /updates` | Read proposal history |
| `POST /updates` | Propose a patch with `{"commit_sha":"FULL_SHA","package":"name"}` |

Human mutations require an unrestricted admin-scope token and current repository admin
access. Only `/scans` also accepts a live, writable task token from the configured trusted
current default-branch workflow. It requests server-owned work and cannot upload a clean
native result or create arbitrary pull requests. Fork/PR/reusable-workflow grants cannot
request trusted native scans. Specify the target commit separately from the workflow SHA.

[The optional workflow](examples/security-native.yml) uses default-branch manual/scheduled
triggers. Set its exact path in native scan settings and enable Actions. rbit checks the
workflow's immutable trust revision and current actor/token policy at admission. Built-in
automation also works without Actions. Disabled scanners allocate no scan jobs; a pending
job is revalidated before reading data and before committing evidence. Jobs survive restart
with expiring claims, bounded queue capacity and shutdown draining.

The [npm lockfile reference](https://docs.npmjs.com/cli/v11/configuring-npm/package-lock-json/)
and [GitHub credential reference](https://docs.github.com/en/organizations/managing-programmatic-access-to-your-organization/github-credential-types)
describe the input formats. This feature does not provide push protection, full-history
secret scanning, credential revocation or general dependency resolution.
