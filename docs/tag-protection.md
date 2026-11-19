# Tag protection

Repository administrators configure **Settings → Tag protection** with tag names or
patterns such as `v*` or `releases/**`. `*` matches within one path segment, `**` crosses
segments and `?` matches one character. Rules are opt-in and apply to existing tags too.
There can be at most 100 rules per repository. Pull mirrors cannot enable tag protection: their refs are controlled by the upstream repository.

A matching tag is immutable: neither an administrator nor a writer may move or delete
it through Git HTTP or SSH. Only a repository administrator may create a matching tag,
change its release, or add or remove release attachments. For protected release changes
through the API, use a repository administrator's personal access token with **Admin**
scope. Deploy keys and Actions task tokens cannot publish protected tags. Other tags
retain existing permissions.
A repository administrator must explicitly remove the matching rules before rewriting
or deleting a protected tag. Rule changes are audited.

The API exposes `GET` and `POST /api/v1/repos/{owner}/{repo}/tag_protections` and
`DELETE /api/v1/repos/{owner}/{repo}/tag_protections/{id}`. These require an administrator
of the repository and a token with admin scope. Creation takes `{"pattern":"v*"}`;
listing returns all rules, at most 100. Git refusals identify `protected_tag_immutable`
or `protected_tag_publisher` and the matching rule.

Migration 0094 adds an initially empty `tag_protections` table. Existing repository
permissions are unchanged until an administrator adds a rule. Back up before upgrading;
restore the pre-upgrade backup to roll back the database schema.
