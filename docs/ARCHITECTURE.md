# rbit architecture

This page describes the source layout at version 0.3.0. The server is a single Rust
process with embedded web assets and local persistence. Git is a runtime dependency for
transport and maintenance. Actions jobs execute in the separate `rbit-runner` program.
The [deployment guide](deployment.md) covers the operational setup; the
[rewrite plan](design/ground-up-rewrite-plan.md) and its
[progress record](design/rewrite/progress.md) preserve the design and acceptance history.

## Request and job flow

```text
Browser / REST client ──> rbit-web (Axum, Askama) ──> feature services ──> SQLite / redb / Git
Git over HTTP ────────> rbit-web ──> rbit-transport ──> git process
Git over SSH ─────────> rbit-ssh ───> rbit-transport ──> git process
Workflow event ───────> rbit-actions ──> database job queue
                                               │
                                      rbit-runner (separate process)
```

The HTTP and SSH adapters authenticate a caller and resolve current access before
serving a repository operation. Feature code makes the policy decision; stores retain
atomic uniqueness, version and race checks. Actions uses the server as its control
plane: the runner claims a job, performs its steps, and returns logs and results. The
server does not execute workflow steps. Actions routes, workers and UI are absent while
the instance switch is off; see the [Actions guide](actions.md).

## Workspace

The workspace is arranged around small domain crates and the concrete adapters that
compose them. New feature crates coexist with compatibility paths in the older crates;
the [rewrite acceptance ledger](design/rewrite/feature-ledger.json) records which
behaviors have moved. A crate's `src/lib.rs` documents its current boundary.

| Area | Crates and responsibility |
| --- | --- |
| Shared contracts and host | `rbit-contracts` owns stable IDs, timestamps and names; `rbit-kernel` owns bounded work admission and lifecycle primitives; `rbit-platform` owns filesystem operations. |
| Feature policy | `rbit-identity-models`, `rbit-identity`, `rbit-access`, `rbit-repositories`, `rbit-collaboration`, `rbit-actions`, `rbit-wikis` and `rbit-notifications` own their domain records, decisions or applications behind explicit ports. |
| Workflow and presentation | `rbit-workflow` parses and plans workflow files and shares runner protocol types; `rbit-localization` holds compiled messages and locale negotiation; `rbit-presentation` owns rendering decisions independent of HTTP. |
| Concrete data and Git | `rbit-storage` owns SQLite and redb access; `rbit-git` reads and maintains bare Git repositories; `rbit-transport` streams Git protocol exchanges through controlled `git` processes. |
| Authentication and delivery | `rbit-auth` provides cryptographic and compatibility adapters; `rbit-mail` renders and delivers mail. |
| Entry points | `rbit-web` owns the Axum routes, middleware and Askama templates; `rbit-ssh` serves Git over SSH; `rbit-services` composes feature applications and existing service adapters; `rbit-server` supplies the `rbit` server and client CLI; `rbit-runner` executes Actions jobs. `rbit-e2e` tests complete flows. |

The feature crates do not provide a dynamic plugin system or separate network
microservices. The server wires them explicitly. For example, `rbit-web/src/routes/mod.rs`
merges page, API, Git, LFS and optional Actions routes; `rbit-server/src/serve/` owns
startup and shutdown. `rbit-workflow` is shared by server and runner so both compile
against the same workflow and protocol types.

## Persistence and Git

One data directory contains the instance's durable state:

| Path | Purpose |
| --- | --- |
| `rbit.db` | SQLite stores accounts, repository metadata, issues, pull requests, Actions state and other relational records. Embedded migrations evolve the schema. |
| `secret.key` | Key for credentials sealed in SQLite. A database backup requires this key to restore those credentials. |
| `repositories/` | Bare Git repositories; Git objects stay in Git's native format. |
| `attachments/`, `lfs/` | Release attachments and content-addressed LFS data. |
| `rbit.redb` | Sessions and rebuildable derived data; see the backup guide for recovery behavior. |
| `actions/` | Runner logs, artifacts, caches and the separate Actions secrets key when Actions is enabled. |
| `tmp/` | Scratch work for merges, edits, imports and uploads. |

SQLite is the authority for relational state. Git refs and objects remain in bare
repositories, where normal Git tools can inspect and maintain them. redb contains
sessions and derived caches. Cross-store operations and native Git updates use feature
specific protocols and recovery checks; a validated name or model alone does not grant
filesystem or repository access. See [backup and restore](deployment.md#backups) for
exact inclusion, key handling and restore order.

Git reads use `rbit-git` and gitoxide; smart HTTP and SSH share `rbit-transport` for
pack exchange. The transport controls child process lifetime and streams pack data.
Browser edits, merges and protected pushes use their own guarded update paths. The
access decision and the ref that may be updated are rechecked at the write boundary.

## HTTP, presentation and localization

Axum routes in `rbit-web` serve HTML, REST API, Git smart HTTP, LFS and streamed files.
Askama renders the HTML. HTMX adds partial navigation and form updates, while the page
routes retain server-rendered responses. Embedded assets use content hashes. The REST
API publishes an OpenAPI document at `/api/v1/openapi.json`.

The presentation layer renders untrusted Markdown through a shared sanitizer; wiki,
repository document, issue and pull request views do not treat repository text as
trusted HTML. Locale negotiation and compiled message catalogs live in
`rbit-localization`; the HTTP adapter owns preference validation and response language.
Client-side themes and the installable PWA are part of the web layer.

Issue and pull conversation sidebars read relationships in one additional pooled
transaction with two statements: a live source/access lookup and one bounded join of
visible edges and endpoints. Progress derives from those visible rows, so hidden work
does not enter counts. The cost is fixed per page, not per relationship. Project boards
batch their visible cards in one relationship query within the existing page snapshot.
`crates/rbit-web/tests/work_counts.rs` records exact pooled connection, SQL statement
and Git-open counts through the real router and must change with intentional page work.

## Security boundaries

Authentication includes sessions, scoped personal access tokens, passkeys, TOTP and
configured external sign-in. Repository authorization combines the current actor and
repository grants; callers load current grants rather than treating a cached model as a
capability. Mutating page requests pass origin and CSRF checks. Background password work
has bounded admission, and the owner of admitted work retains capacity until it ends.

Actions has an additional boundary: workflows are untrusted input, and the runner is a
separate process. Its container backend isolates job execution through Docker or Podman;
the host backend runs with the runner user's privileges and is restricted to trusted
repository work. Runner credentials and per-task tokens have separate scopes and
lifetimes. [Actions](actions.md) documents the supported workflow dialect, runner
configuration and trust model.

## Testing and change control

Request work is measured by `rbit-web/tests/work_counts.rs` as exact SQL acquisitions,
statements and derived-cache writes for fixed anonymous and signed-in page fixtures.
The signed-in Files changed fixture uses ten acquisitions and ten statements. Three
serve review workflow state: one source-repository lookup, one current-grants query,
and one private progress read capped at 1,000 rows. Opening the source independently
keeps base-repository access from granting authority over a fork. These reads occur
once per page, not per changed file. Progress fingerprints use the exact comparison
already prepared for display, so a concurrent base push cannot attach a new preimage
to an older diff. Mutations recompute the current fingerprint and reject stale markers;
the workflow adds no database or cache writes on this read path. The anonymous fixture remains at six acquisitions and statements.

Unit and integration tests live with their crates; `rbit-e2e` exercises live Git, API,
SSH, Actions and browser-visible flows. `.github/workflows/ci.yml` runs the hosted quality
gates, including Linux and Windows tests and the stable `CI required` aggregate. The
[contribution guide](../CONTRIBUTING.md) gives the current local commands and release
gate. Historical milestone contracts under `design/` explain decisions made while
features were built; their old branch plans and progress counts are not current status.
