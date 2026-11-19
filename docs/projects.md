# Organization projects

Organization members can plan issues and pull requests across their organization's
repositories at `/organizations/{organization}/projects`. Organization owners and site administrators
create projects, rename them, configure columns and delete projects. Members can add,
move and remove cards for repositories they can currently read. Outsiders cannot see
boards, even if some linked repositories are public.

Create a project with a name and 2–12 columns, one name per line. Columns are ordered
positions: renaming a column keeps its cards in that position. Move cards before removing
occupied trailing columns. The last column is the completed-work column. Add a card by
repository name and issue or pull request number; adding the same card again preserves
its position. Removing a card or deleting a project never deletes the original thread.

Completed issues and merged pull requests appear in the last column automatically.
This reads the existing issue lifecycle, including issues closed by a merged pull
request's `Fixes #number` references, rather than copying asynchronous events into a
second state machine. Reopened issues return to their saved position. Issues closed as
not planned retain their saved position. Automatically completed cards cannot be moved;
reopen the underlying issue first. Manual card movement does not close an issue.

Use repository, title, status-column, priority, iteration and overdue filters in either
the board or table layout. Pages contain at most 100 visible cards. Sort by issue ID,
priority (urgent first, unset last) or target date (earliest first, unset last); ties use
issue ID. Next page retains the effective filters and sort. Live edits can move cards
between pages, so refresh from the beginning when reconciling a changing project. Hidden cards contribute neither
rows nor pagination cursors. Repository transfers out of the organization stop showing
those cards. Their saved placements remain in the board and count toward its card cap;
transferring the repository back restores them. Deleting the project removes all saved
placements. An organization can have 100 boards and a board can contain 10,000 cards.

All mutations recheck current identity, membership and repository access inside the
same database write transaction. Card moves/removals and project settings/deletion
require the current revision; stale requests return a conflict so concurrent changes
are not silently overwritten. Board revisions also change when cards change.

## Planning and saved views

Choose **Edit planning** on a card to set its priority, target date and iteration. These
fields belong to that project: adding the same issue to another project starts with no
priority, date or iteration. Editing them does not change the issue, close it or override
a dependency. Removing and re-adding a card clears its planning fields. Dates are calendar
days (`YYYY-MM-DD`); an open issue is overdue when its target date precedes today in UTC.
Closing an issue clears its overdue indicator; reopening recomputes it from the retained
date. Table rows and board cards show only progress and blockers the viewer can read.

Owners and site administrators can create up to 200 named iterations per project, with
inclusive start/end dates. Dates may overlap; iterations do not roll over automatically.
Archive an iteration to keep existing assignments while preventing new ones. Delete it
only after unassigning its cards; saved views selecting it are also removed. The dates
organize work and do not constrain individual target dates.

**Saved views** stores the current filters, layout and sort. Each member can keep 50
personal views per project. Only their creator can see or edit them, including when
another member manages the board. Owners and site administrators can also manage up to
50 shared views per project. Sharing scope is fixed at creation; create a new view to
change it. Saving an existing view replaces its name and criteria with the current
filters. Pagination is never saved.

Repository-filtered views use stable repository IDs, follow renames and disappear when
the repository is no longer readable or leaves the organization. They become visible
again if access returns. Hidden views retain their quota slots; deleting their repository
removes them. Shared views still return only each reader's visible cards and progress.
Column filters reference positions like cards; if a selected trailing column was removed,
clear the filters and replace the affected saved view with valid criteria.

## REST API

The OpenAPI document includes these endpoints under `/api/v1`:

| Method | Path | Input |
| --- | --- | --- |
| GET | `/orgs/{org}/projects` | List boards |
| POST | `/orgs/{org}/projects` | JSON `title`, `columns` array |
| GET | `/orgs/{org}/projects/{id}` | Query `q`, `repository`, `column`, `priority`, `iteration`, `overdue`, `layout`, `sort`, `cursor` or `view` |
| PATCH | `/orgs/{org}/projects/{id}` | JSON `title`, `columns`, `revision` |
| DELETE | `/orgs/{org}/projects/{id}` | Query `revision` |
| POST | `/orgs/{org}/projects/{id}/cards` | JSON `repository`, `number` |
| PATCH | `/orgs/{org}/projects/{id}/cards/{issue}` | JSON `column` (zero-based), `revision` |
| DELETE | `/orgs/{org}/projects/{id}/cards/{issue}` | Query `revision` |
| PUT | `/orgs/{org}/projects/{id}/cards/{issue}/planning` | JSON `revision`, `priority`, nullable `iteration_id`, nullable `target_date` |
| GET / POST | `/orgs/{org}/projects/{id}/iterations` | List / create with `title`, `starts_on`, `ends_on`, optional `archived` |
| PATCH / DELETE | `/orgs/{org}/projects/{id}/iterations/{iteration}` | Replace with current JSON `revision` / delete with query `revision` |
| GET / POST | `/orgs/{org}/projects/{id}/views` | List / create with `title`, optional `shared`, `filter` criteria |
| PATCH / DELETE | `/orgs/{org}/projects/{id}/views/{view}` | Replace with current JSON `revision` / delete with query `revision` |

The card path uses the global `issue_id` returned by the board, while adding a card uses
its repository-local number. A board response contains `board`, `cards`, `next` (the next
`after` cursor or null for ID sorting), opaque `next_cursor`, effective `filter`,
`iterations`, visible `views`, and `can_manage`. Use `cursor` for any sort; the cursor is
bound to the board and effective criteria. The legacy `after` input works only for ID
sorting. A saved view request accepts `view` and pagination only. For iteration filtering,
`0` selects unassigned cards; omit it for all iterations. Priority values are `none`,
`urgent`, `high`, `normal` and `low`; layouts are `board` and `table`; sorts are `id`,
`priority` and `target_date`. Set `overdue=true` for overdue work only. Cards contain the effective `column`, their
current `revision`, and `automatic` when completion comes from the thread lifecycle.

Use an unrestricted personal access token: read scope for reads, write for card
and personal-view mutations, and admin for board configuration, iterations and shared
views. The response flags `can_manage` and view `can_edit` also respect the token scope. Repository-limited tokens and workflow
tokens cannot access organization projects. API authorization does not use cookies.
Browser forms retain the normal Origin checks.

## Upgrade

Migration `0101_project_planning` adds project planning, iteration and saved-view tables
and indexes. Existing cards start without planning fields; no issue state is changed.
Back up before upgrading. Older binaries cannot open the newer migration set; rollback
requires restoring the pre-upgrade database backup.


Migration `0093_project_boards` adds two SQLite tables and indexes. Existing repositories
and issues are unchanged. Normal database backups include projects and cards. Take a
pre-upgrade backup before applying migrations; older binaries cannot run against a
newer migration set. Restore the pre-upgrade backup for rollback.

Cards show visible direct-subissue progress and an advisory blocked indicator. See [issue hierarchy and dependencies](issue-relations.md) for permissions, lifecycle behavior and limits.
