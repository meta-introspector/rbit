# Issue hierarchy and dependencies

Issues and pull requests can have one parent, direct subissues, and advisory blocking
relationships. Open an issue's **Details → Relationships → Add relationship**, choose
**Subissue** or **Blocked by**, and enter its number. Leave Repository blank for the
current repository, or enter another repository name in the same organization. Add a
subissue from its parent's page; the child then shows a reciprocal parent link. Removal
controls are available from either endpoint when you can write to both repositories.

Project cards show **Blocked** when the open issue has an open blocker you can access.
They also show completed/total direct subissues. A completed issue or merged pull request
counts as complete; “not planned” counts in the total but not completed. Closing or
reopening work updates these projections on the next request, including issue closure
through merged pull request keywords. Closing a parent does not close its children.
Blocking is advisory: it does not prevent closure, moving a card, or merging a pull request.

Only relationships whose two repositories you can currently read appear in lists,
progress or blocked indicators. These are visible-work rollups, not organization-wide
counts. An inaccessible parent or blocker is not named or counted. Write access to both
repositories is required even if you authored an issue. Archived repositories are read-only.
Repository-limited tokens and workflow task tokens cannot access these cross-repository
resources; use an unrestricted personal token with read or write scope as appropriate.
Anonymous callers can read relationships between public repositories.

Both relationship kinds represent a dependency: parent → child and blocked issue →
blocker. The combined directed graph must be acyclic. A parent may also be explicitly
blocked by its child (a redundant same-direction edge), but a child cannot depend on its
parent: that would create a cycle. Each issue has at most 100 incident relationships;
cycle checks visit at most 1,000 distinct issues. Requests that cannot safely establish
acyclicity within that bound are refused. Cycle, existing-parent and capacity conflicts
use the same response, without revealing hidden edges or counts.

Deleting an issue or repository removes its relationships. Transferring a repository to
a different owner removes its cross-repository relationships and preserves relationships
inside the transferred repository. Renames follow the current repository and account
names. Relationships do not silently reappear if the repository is later transferred back.

## REST API

Use `/api/v1/repos/{owner}/{repo}/issues/{number}/relations` for issues and pull requests:

- `GET` returns `items`, `progress: {completed, total, blocked}` and `can_edit`. Items have
  stable `id`, current owner/repository/number/title/state, `issue_kind`, `relationship`
  (`parent`, `child`, `blocked_by`, `blocking`) and `can_remove`.
- `POST` accepts `{"kind":"child","repository":"backend","number":42}` or
  `{"kind":"blocked_by","number":42}`. It returns `201` with the edge `id`. Adding the
  same edge again is idempotent and returns its existing id.
- `DELETE .../relations/{edge}` removes that exact edge and returns `204`. The edge must
  touch the requested issue. An old id cannot remove an edge that was removed and re-added.

Mutations require write scope and live write access to both endpoints. Missing or hidden
endpoints return `404`, insufficient authority `403`, and graph conflicts `409`. Project
API cards include the same permission-filtered `progress` object. The served
`/api/v1/openapi.json` document describes the complete shapes.

## Upgrade

Migration `0099_issue_relations.sql` adds the strict relationship table, indexes and a
repository-transfer cleanup trigger. Existing issues begin without relationships. Take
a pre-upgrade database backup: restoring that backup is the rollback path; older binaries
must not be pointed at the upgraded schema.
