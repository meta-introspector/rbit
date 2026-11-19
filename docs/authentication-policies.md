# Security policies and sign-in providers

Site administrators can choose **Site administration → Security → Required** to
require an authenticator app (TOTP) or passkey for every account. Organization
owners can make the same choice in **Organization settings → Security** for their
member accounts, including owners. Both policies are optional by default. A site
requirement takes precedence over an optional organization policy; membership in
any organization that requires MFA is sufficient to require enrollment.

These are account enrollment requirements. Existing browser sessions are directed
to **Settings → Security** until the user enrolls. Password changes, enrollment and
sign-out remain reachable. Personal access tokens (including existing tokens) and
user Git/SSH access are refused until enrollment. Public anonymous access and
repository-scoped deploy keys retain their existing permissions. The last required factor cannot be removed through Security settings. An account
without a factor (including after an administrative reset) must enroll again. Recovery codes still work for the normal second sign-in step.

Set up your own authenticator app or passkey before enabling a policy. Changes
are persisted and take effect immediately, without a restart. Deployment config
`[security] require_two_factor = false` supplies the initial site default; the live
administrator choice overrides it. Existing installations remain optional after
upgrade. Migration 0090 adds organization policy storage without changing users
or memberships.

## Configure Google, Discord, GitHub or organization SSO

Open **Site administration → Security → Configure sign-in providers** (also available
in **Configuration → Sign-in providers**). Choose a Google, Discord, GitHub or OIDC
preset, enter the application's client ID and secret, and register the exact callback
`{server.public_url}/login/{provider_id}/callback` in the provider's application.
Keep provider IDs stable after accounts link identities.

Presets begin disabled. Clear **Disable provider** when credentials are ready,
save the section, then restart using the administration restart page. Check the
sign-in page before closing your existing administrator session. Check **Disable
provider**, save and restart to hide its sign-in/link buttons and refuse callbacks
without deleting its settings or linked identities. Keep another working sign-in
method for users before disabling their only provider. Secrets continue to use
the encrypted configuration storage and redacted editor.

- Google uses OIDC discovery at `https://accounts.google.com`, with `openid email profile`.
- Discord uses its OAuth2 authorization-code flow with PKCE and `identify email`,
  retrieving the immutable user ID from `/users/@me`. The account's `mfa_enabled`
  flag does not prove that this authorization used MFA and is never treated as proof.
- GitHub uses the existing OAuth flow; GitHub Enterprise remains configurable.
- Generic OIDC supports providers such as Keycloak, Authentik and Entra ID through
  their issuer/discovery URL. Existing ID-token signature, issuer, audience, nonce,
  expiry, PKCE/state, endpoint address guards and linked-identity rules still apply.

Email linking and provider MFA trust remain opt-in. Enable `trust_email` only when
the provider controls and verifies its users' addresses. `trust_mfa` is OIDC-only
and accepts an authenticated `amr` claim containing `mfa`; it does not replace local
enrollment required by a site or organization policy.

Provider references: [Google OIDC](https://developers.google.com/identity/openid-connect/openid-connect),
[Discord OAuth2](https://docs.discord.com/developers/topics/oauth2),
[Discord user API](https://docs.discord.com/developers/resources/user#get-current-user).

## Repository settings navigation

Repository settings and their subpages share a persistent section sidebar. On
small screens it becomes a horizontally scrollable tab row. Nested forms retain
the active section, and Actions appears only when enabled for the instance.
The danger-zone link jumps to the general settings' destructive operations.
