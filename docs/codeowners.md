# Code owners

Add `.github/CODEOWNERS`, `CODEOWNERS`, or `docs/CODEOWNERS` to a repository's base
branch. The first existing file in that order applies. Pull requests use the base
branch's version, so a proposed ownership change cannot select its own reviewers.

```
* @maintainer
/src/ @acme/core
/docs/ @writer
/docs/generated/
```

The last matching line applies. Owners on one line are alternatives: one eligible
owner's approval covers that group. Every changed owned path needs a matching
approval; renames check both old and new paths. Include the CODEOWNERS file itself
in an ownership rule (the wildcard above does this) so policy changes also need
review. A line without owners clears earlier ownership for its matches. Unmatched paths require no owner approval.

Use `@username` or `@organization/team`. Owners must be active users with current
write access, and the pull request author cannot approve their own changes. Teams
must belong to the repository's organization. Matching is case-sensitive. `*` matches
within a path component, `**` crosses directories, `?` matches one byte, `/` anchors a
pattern at the root, and a trailing `/` matches a directory and its contents. Comments
start with `#`. Negation, character classes, escaped whitespace and email owners are
not supported; invalid files fail closed when owner approval is required.

Opening a ready pull request, marking it ready, changing its base, or pushing to its
head or base requests eligible owners using the ordinary review inbox and notification
flow. Timeline entries identify automatic requests as CODEOWNERS automation.
Existing requests are deduplicated. Earlier manual requests are preserved.

In **Repository settings → Branch protection**, enable **Require a pull request**
and **Require code owner reviews**. The REST protection API accepts
`require_code_owner_reviews: true`. Existing rules default to false after migration
0092. Owner approvals follow the rule's stale-approval setting. Direct merges and the
merge queue evaluate current ownership and access; administrator bypass remains an
explicit audited action for rules that permit it. Refusals carry
`merge_requires_code_owners` and an actionable explanation.

Ownership evaluation is bounded: 64 KiB per file, 256 rules, 32 owners per rule,
100 members per team, 100 referenced owner identities, 500 expanded candidates,
100 automatic reviewers, 1,000 changed paths of at most 4,096 bytes each, and a total
pattern-matching budget. Non-UTF-8 paths cannot be certified. Incomplete path lists
or exceeded evaluation limits block required owner reviews. Split an oversized pull request or simplify the ownership file. Invalid
or oversized files do not send automatic review requests. Missing files and unowned
paths impose no owner requirement. Administrators should establish CODEOWNERS on the
base branch before enabling enforcement.
