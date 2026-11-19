# Manage agents in the browser

Open **Settings → Agents** to create an automation identity. Each agent has a permanent
handle, a display name, explicit capabilities, and repository or organization scopes.
The creating human owns the agent; agents cannot sign in interactively, administer
repositories, approve reviews, or issue their own credentials.

Enter one `owner/repository` or organization name per line in the read-only or write
fields. There may be at most 32 scopes in total. Organization-wide scopes require
current organization ownership and cover existing and future repositories. Effective
access always follows the owner's current permissions and each organization's policy.
Unavailable scopes are omitted from the editable fields; saving removes them.

Agent changes and new credentials require a recent proof of identity. Enter your current
password when asked. Editing uses the displayed revision, so a concurrent change is
refused instead of overwritten. Reload the page to review the latest state before trying
again.

Generate a credential from the agent's management page. The default lifetime is
30 days; the normal expiry choices, including **No expiration**, are available. A
read-only credential cannot perform writes even when the agent has write capabilities.
Copy the secret immediately: it is shown in that response only, is not stored in browser
history by the application, and never appears again after a reload. Existing credential
lists show only names, hints, scopes and expiry information.

**Rotate credential** atomically replaces the selected credential and shows the replacement
secret once. The old credential stops authenticating when rotation succeeds. Rotation
also works when an agent already has the maximum ten credentials. **Revoke credential**
ends one credential; **Disable and revoke all credentials** disables the agent and
revokes all its credentials. Re-enabling an agent does not restore revoked credentials.

Organization owners can open **Organization settings → Manage agent policy** to allow or
block agents. Blocking applies immediately to existing credentials and repository-specific
scopes as well as organization-wide scopes. A site administrator must also be a current
organization owner to manage this policy.
