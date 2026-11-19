# Independent releases and updates

rbit can host release downloads and update metadata on its own instance. The browser's
existing Releases page serves notes and attached archives. Public repositories also serve
`/{owner}/{repository}/releases/latest.json`, a small JSON document containing only
`tag_name`, `draft` and `prerelease`. It selects the latest published stable release before
prereleases, never a draft. A repository with only prereleases returns that prerelease;
the update checker ignores it.

This feed is available even when the REST API is disabled. It always checks anonymous
repository access, including when a browser sends an owner's session. Private repositories
and instances requiring sign-in do not expose public metadata. Responses use `no-store`
so a cache cannot keep serving metadata after repository visibility changes.

## Configure update notices

The default source is rbit.sh. A deployment can choose another public HTTPS source:

```toml
[updates]
check_for_updates = true
report_version = true
feed_url = "https://git.example.com/platform/rbit/releases/latest.json"
release_url_prefix = "https://git.example.com/platform/rbit/releases/tag/"
```

The corresponding environment settings are `RBIT_UPDATES__FEED_URL` and
`RBIT_UPDATES__RELEASE_URL_PREFIX`. Restart after changing configuration. The prefix must
end with `/`; both URLs must use HTTPS without credentials, query strings or fragments.
Only a validated semantic-version tag is appended to the configured prefix. URLs received
inside feed responses are ignored. The source is administrator-controlled deployment
configuration, so choose a trusted endpoint with public release metadata.

The checker sends no credentials, cookies, repository data, running version or instance
identifier. It refuses redirects, limits responses to 64 KiB, and times out after ten
seconds. Successful checks are cached in memory for 24 hours; failures back off and keep
the last successful notice. Disabled checks create no HTTP client or worker. Notices never
download or install updates automatically.

Existing deployments can retain the previous GitHub source explicitly:

```toml
[updates]
feed_url = "https://api.github.com/repos/GitRbit/rbit/releases/latest"
release_url_prefix = "https://github.com/GitRbit/rbit/releases/tag/"
```

No database migration is required. Before rolling back to a binary that predates these
settings, remove the two new keys from deployment files, environment overrides and any
saved administrator configuration.

## Native publication

`.rbit/workflows/release.yml` is a manual workflow. Dispatch it on an existing `vX.Y.Z`
tag whose version matches both binaries and the changelog. `publish=false` is the default:
it runs formatting, source rules, structural contracts, frontend tests, strict Clippy,
workspace tests and doctests, advisory/license checks, the release build and upgrade
rehearsal. It checks the exact HEAD/tag and clean tracked source before and after the
build, then packages the binaries with their licenses, documentation and deployment files.

The initial native workflow supports the Linux architecture of its runner (`x86_64` or
`aarch64`). Use a runner whose glibc and other runtime libraries match your supported
deployment baseline, and state that baseline in the release notes. It does not claim to
build macOS, Windows or a multi-architecture image. The legacy GitHub workflow remains
available for those builds. Native checkout and artifact actions use rbit's built-in
implementations; archive publication does not use GitHub or `gh`.

Before enabling publication, configure the repository's **release** environment:

- Allow only release tags, such as `refs/tags/v*`, and require a reviewer with self-review
  disabled. Keep publication credentials and keys in this environment.
- Protect release tags with an immutable tag rule. Client-side tag checks detect changes
  before upload and publication, but are not an atomic lock across REST requests;
  immutable tag policy supplies that guarantee. Do not remove the rule during delivery.
- Set `RELEASE_PUBLIC_KEY` as an environment variable containing the trusted Ed25519
  public key in PEM format. Store its matching private key as the environment secret
  `RELEASE_SIGNING_KEY`. Distribute the public key and its fingerprint through a separate
  trusted channel before a release. Key rotation requires updating that trust explicitly.
- For protected tags, set the environment secret `RELEASE_TOKEN` to a personal access token
  with **Admin** scope from a repository administrator. **Write** scope caps the owner's
  effective access at Write. The fallback workflow task token can publish only unprotected
  releases where it has write access. Enable the native REST API for publication.

Dispatch with `publish=true` when ready. This dispatch builds and checks its own candidate
again; the publication job takes that run's artifacts after approval. Signing creates
`RELEASE.json` with the tag and exact commit, a canonical `SHA256SUMS` manifest covering
that metadata and all archives, and its raw Ed25519 signature `SHA256SUMS.sig`. Signing
requires a fresh directory: existing manifest files or symlinks are never overwritten.
No private key is added to the archive or artifact.

The publisher verifies local signatures and checksums before sending a credential. It
addresses the exact `refs/tags/...` reference, creates a draft, uploads and reads back every
asset, then publishes. Interrupted uploads remain drafts. A retry verifies existing assets
before resuming; mismatched metadata, missing published assets, unexpected files or different
bytes cause a refusal, never an overwrite. An unchanged complete published release is an
idempotent success. Keep tokens in environment variables, never arguments or checked-in files.

## Verify a download

Download all archives named in `SHA256SUMS`, plus `RELEASE.json`, `SHA256SUMS` and
`SHA256SUMS.sig`, into an otherwise empty directory. Obtain `trusted-release-key.pem`
through the separately trusted channel, outside that directory. With Node.js 24 and this
repository's verification script:

```bash
node scripts/native-release/cli.mjs verify ./downloaded-release ./trusted-release-key.pem
```

The verifier rejects an invalid signature, changed bytes, unlisted files, unsafe names,
symlinks or mismatched release metadata. Check that the printed tag and commit are the
release you intended to install. The script uses Node's standard
[Ed25519 signing and verification](https://nodejs.org/download/release/v24.22.2/docs/api/crypto.html),
with no additional cryptography package. Older mirrored releases may only have unsigned
checksums; they do not gain signature trust retroactively.

## Container publication

Set `RELEASE_REGISTRY` (a hostname, optionally with a port) and `RELEASE_NAMESPACE` as
variables in the **release** environment. Set `RELEASE_REGISTRY_USER` and
`RELEASE_REGISTRY_PASSWORD` there as secrets for that registry. The Linux publication
runner needs Docker with Buildx and a compatible daemon. Explicitly setting
`containers=true` along with `publish=true` publishes both `rbit` and `rbit-runner` images.

The publisher verifies signed archives again and extracts only the exact binary members
to fixed files, without allowing archive paths to write to disk. It uses the Dockerfiles'
`BINARY=prebuilt` stages, so the images contain the verified release binaries. Credentials
are passed to Docker through standard input and an isolated temporary Docker configuration
is removed afterwards. Existing local binary files are not overwritten.

Version tags retain prerelease labels; SemVer build metadata uses `_` instead of `+` in
container tags. Only stable versions also update `latest`. A publication covers its native
Linux architecture; do not publish a second architecture over the same tag expecting Docker
to assemble a multi-architecture manifest. Configure registry visibility and version-tag
immutability according to your delivery policy. Container publication follows archive
publication; if it fails, the signed archives remain available and the workflow reports
failure. No registry or release is contacted by the local unit-test fixtures.
