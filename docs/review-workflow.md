# Reviewing changes

The pull request **Files changed** tab supports private file progress, changes since your previous review, and suggestions that a source-branch writer can apply together.

## File progress and previous reviews

Use **Mark as viewed** beside a file. Progress belongs to your account; other reviewers have their own marks. A mark identifies both sides of the file's diff, including modes and rename source. Unchanged files stay viewed after unrelated pushes. Changed, renamed, or newly deleted files need review again. Only the bounded files displayed by the diff can be marked. Up to 1,000 recent file marks are retained per reviewer and pull request.

**Changes since my last review** compares the exact trees of your most recently submitted review and the current head, including force pushes. It never accepts a caller-selected baseline commit. With no previous review, the page explicitly shows the first-review state and the full diff. You must still be able to read the source repository. Inline commenting is disabled in this comparison; return to **All changes** to anchor a new comment in the pull request diff.

## Suggested replacements

On a right-side diff line, post one standalone fenced block:

````text
```suggestion
replacement text
```
````

The block replaces the commented line. It may contain several replacement lines. An empty block deletes the selected line. To replace surrounding lines, use `suggestion:-N+M` (for example `suggestion:-1+2` includes one line before and two after the anchor). Each offset is at most 100. Only one suggestion block per comment is supported; replies and left-side comments cannot contain applicable suggestions.

Choose the checkboxes beside submitted suggestions, then **Apply selected suggestions**. A batch contains at most 20 suggestions/files and at most 1 MiB of combined input/output text. Overlapping ranges, binary/oversized files, symlinks, submodules, and LFS pointers are refused. The commit preserves file modes, untouched bytes, and line endings, and uses the applying user's noreply identity.

Application requires write access to the source repository, including a fork. Base-repository write access does not grant fork write access. The source branch's protection rules apply without administrator bypass. Archived or mirrored source repositories are read-only. The exact reviewed head and comment-body digest must still match; a changed branch, edited comment, resolved/outdated thread, or removed comment refuses the whole batch. There is no automatic rebase. Refresh and review new suggestions instead.

All selected changes land in one compare-and-swap commit. Application updates the pull request head and outdated comments before redirecting; if that secondary update fails after the commit is accepted, the normal push listener can retry it. A short database writer fence retains current account, repository, comment, and policy authority through publication. Cancellation of the HTTP request does not abandon admitted work; at most two batches run per instance, with bounded time and shutdown draining.

## REST API

Under `/api/v1/repos/{owner}/{repo}/pulls/{number}`:

- `GET /review-progress` returns the current `head_sha` and personal `{path, fingerprint, viewed}` entries.
- `PUT /review-progress` accepts `{path, expected_head, fingerprint, viewed}`.
- `GET /review-changes` returns `first_review`, `baseline`, `head_sha`, bounded file patches and truncation status.
- `POST /suggestions/apply` accepts `{expected_head, suggestions: [{comment_id, body_digest}]}` and returns `commit_sha`.

Suggestion comments require `expected_head` when composed. Comment responses include `body_digest`; applying a batch requires a write-scoped token with push rights and a credential that reaches the source repository. Private progress never accepts another reviewer identifier. Existing origin checks protect browser forms.

Migration 0096 adds only private progress records; existing reviews and comments retain their meaning.


## Resolve conflicts in the browser

On a conflicted pull request, choose **Resolve conflicts**. Expand **Source branch** and
**Target branch** to inspect either side, then edit **Resolved content** for every file.
Remove Git conflict markers before choosing **Commit resolution to source branch**.
The result is one merge commit with the original source and target commits as parents.
Automatically merged changes from both branches remain intact; the target branch does
not move. Normal checks, CODEOWNERS, security gates and approvals still govern merging
the pull request.

The editor supports regular UTF-8 text with matching paths and modes: up to 20 conflicted
files, 256 KiB per file and 1 MiB of combined preview text or submitted resolutions.
Binary files, symlinks, submodules, LFS pointers, rename/delete or mode conflicts and
larger changes require a local Git checkout. Source-file line endings and executable
modes are preserved. An empty resolution creates an empty file; it does not delete it.

You need current write access to the source repository, including a fork. Source-branch
protection has no administrator bypass. Both branch names and exact commit snapshots
are rechecked before publication; a stale preview is refused without rebasing your draft.
Refused browser drafts remain visible for copying before you reload. A target branch
that advances after the final check may need another update, but its newer commits are
never overwritten: publication changes only the source branch using compare-and-swap.

The matching API uses `GET` and `POST /api/v1/repos/{owner}/{repo}/pulls/{number}/conflicts`.
Both require a write-scoped token with Push permission and independent source-repository
access. GET returns `head_sha`, `base_sha` and bounded `files` containing `path`, `source`,
`target` and `merged` text. POST accepts `{expected_head, expected_base, resolutions:
[{path, content}]}` and returns `commit_sha`. Every conflict path must occur exactly once;
extra paths, unresolved marker lines and stale snapshots are refused. The same guarded,
cancellation-owned publication path serves browser and API. This feature adds no migration.
