# Agent identities

Create an agent under your account when automation needs its own identity and
credentials. Issues, comments, pull requests and other actions use the agent's
account, so activity remains distinguishable from its human owner. The owner
controls the agent's capabilities, repository and organization scopes, and
credentials. See [the settings workflow](agents-ui.md) for browser management.

An agent is non-interactive: it cannot sign in with a password, create a browser
session, link an identity provider, use passkeys, recover a password, or register
an SSH key. Use its personal access token for the REST API, Git HTTPS and LFS.
It cannot create other agents or tokens, administer repositories or organizations,
approve Actions jobs, or submit approving/request-changes PR reviews.
Comment-only reviews are available with pull-request capability.

## Delegated access

Every agent needs `read` and at least one explicit scope. Capabilities are:

| Capability | Enables |
| --- | --- |
| `read` | Read supported repository content and metadata inside its scopes |
| `push` | Push Git changes and perform supported server-side branch edits |
| `issues` | Create/update issues and issue comments |
| `pull_requests` | Create/update pull requests and PR comments/review suggestions |
| `statuses` | Publish commit statuses |
| `dispatch` | Dispatch permitted Actions workflows |
| `read_logs` | Read permitted Actions logs and artifacts |
| `merge` | Merge pull requests subject to normal repository rules |

Read/write scope ceilings and token scope apply in addition to capabilities.
An agent with a read-only repository scope cannot comment or otherwise mutate
that repository, even when it has the matching capability. Suggestions that
change a branch require both pull-request capability on the base repository and
push capability on the head repository. Protection, checks, merge requirements
and Actions environment approvals continue to apply.

A repository scope reaches one repository. An organization scope reaches its
current and future repositories while the human remains an organization member.
Only a current organization owner can grant an organization-wide scope. Effective
access is always bounded by the human's current rights and never exceeds write.
An agent has no baseline access to public repositories outside its scopes.
Organization owners can disallow agents for the organization; this also blocks
explicit individual-repository grants to its repositories.

Suspending the human, disabling the agent, removing a grant, reducing capability,
revoking a credential, or requiring the human to enroll in MFA removes the
corresponding access. Disabling an agent permanently revokes all of its tokens;
re-enabling requires new credentials. Parent password recovery/credential-epoch
changes also permanently retire child credentials. Deleting the parent leaves
the old agent account permanently non-interactive; it cannot become a human.

## Manage with the REST API

Management endpoints require an unlimited **human** PAT with `admin` scope and
operate only on agents owned by that human. Being a site administrator does not
permit managing another human's agents through these endpoints. Use your
server's base URL and keep token values in an appropriate secret store.

Create an agent with `POST /api/v1/user/agents`:

```json
{
  "slug": "review-bot",
  "name": "Review assistant",
  "capabilities": ["read", "pull_requests"],
  "scopes": [
    {"kind": "organization", "id": 42, "access": "write"}
  ]
}
```

The response includes `id`, stable `login`, `type: "Bot"`, `owner_id`, `revision`,
and scopes. Agent handles cannot be renamed. List owned agents with
`GET /api/v1/user/agents`, or fetch one at `/user/agents/{id}`. Each human may
have up to 50 agents, each with at most 32 distinct scope targets.

Update with `PATCH /api/v1/user/agents/{id}`:

```json
{
  "revision": 1,
  "enabled": true,
  "agent": {
    "slug": "review-bot",
    "name": "Review assistant",
    "capabilities": ["read", "pull_requests"],
    "scopes": [{"kind": "repository", "id": 17, "access": "write"}]
  }
}
```

Use the current revision. A stale revision returns `409` with code
`agent_changed`; reload and review changes before retrying. Disable independently
of old scope validity using `POST /user/agents/{id}/disable` with
`{"revision": 2}`. Unavailable or unowned agents return `404`.

## Issue and rotate credentials

`POST /api/v1/user/agents/{id}/credentials`:

```json
{"revision": 1, "name": "Review worker", "scope": "write", "expires_in_days": 30}
```

Choose `read` or `write`. Expiration choices are 7, 30, 90, 365 days or `null`
for no expiration; omission defaults to 30 days. A credential issued through the
API may not outlive the human PAT that issues it. The one-time response includes
the plaintext `token` and uses `Cache-Control: no-store`. Store it immediately;
listing credentials returns metadata only. Each agent may have ten credentials.

The returned credential `id` is a stable management identifier, separate from
its underlying personal access token ID. It is never reused after deletion.
Use that ID for `DELETE /user/agents/{id}/credentials/{credential}` or
`POST /user/agents/{id}/credentials/{credential}/rotate`. Rotation accepts the
same JSON as creation and atomically replaces the old credential, including at
the ten-credential limit. On a refused rotation the old credential remains valid;
on success it is revoked and the response contains the new one-time secret.
A stale rotation/revocation cannot target a later credential.

Agent `GET /api/v1/user` identifies itself as `type: "Bot"` with
`agent_owner_id`. An agent PAT cannot call the management endpoints.

Organization policy uses `GET`/`PUT /api/v1/orgs/{org}/agent-policy`, with
`{"allow_agents": false}` to deny agent access. Only a current organization
owner may manage this policy. The served OpenAPI document describes the complete
request and response schemas.

## Upgrade and rollback

Migrations 0103–0105 add agent identities, current-scope dispatch admission and
stable credential management identifiers. Existing human accounts and ordinary
access tokens retain their behavior. Migration 0105 preserves existing agent
credential links while assigning non-reused management IDs. Take a database backup
before upgrading; rolling back these migrations requires restoring that backup.

Migration 0106 keeps single-repository agent access checks indexed by agent and
repository, avoiding enumeration of other repositories in organization scopes.
Effective permissions and live revocation rules remain the same. Take a database
backup before applying 0106; rollback requires restoring the pre-upgrade backup.
