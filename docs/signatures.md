# Repository signing trust

Repository administrators enroll SSH or armored OpenPGP **public** keys in
**Settings → Trusted signing keys**. Trust is local to that repository. At most 16
keys may be enrolled; removing a key removes its trust from subsequent checks.
Key changes are audited. Migration 0095 adds the public-key table; take the usual
pre-upgrade backup. Keys travel with the database backup.

Commit pages load a signature badge separately so verification never delays the
initial code page. Without JavaScript, use **Check signature**. The tag list has
the same link: annotated tags verify the tag object; lightweight tags verify the
commit they reference. The verification page shows the exact object id and kind.

The states are:

- **unsigned**: the commit or annotated tag carries no supported signature.
- **verified**: a key explicitly trusted by this repository cryptographically
  signed these exact bytes, in Git's signing namespace for SSH signatures.
- **unknown**: the signing key is absent from repository trust, or trust changed
  while verification was running.
- **invalid**: verification failed, or OpenPGP reports an expired/revoked signature
  or key.
- **unavailable**: a verifier is missing, busy, unsupported, or exceeded its limits.

Verification does not establish that the commit author owns an rbit account,
that the code is safe, or that a release was approved. It is informational and
adds no merge or push requirement.

Install `ssh-keygen` for SSH verification and `gpg` for OpenPGP verification on the
server's trusted executable path. Official container images include both programs;
standalone binary installations need the host packages (`gpg` and `openssh-client`
on Debian). Missing programs do not break repository browsing.
The server never reads repository-provided verifier settings or executable paths.
GPG uses a fresh private home, no option files, no automatic key retrieval, and no
automatic agent startup. Only enrolled public keys enter its keyring. Enrollment
rejects secret packets even under public-key armor, compressed/encrypted packets,
and partial/indefinite packet lengths; export a standard armored public key.

Verification uses gitoxide's exact signed-data extraction on immutable object ids.
Objects above 1 MiB are refused. Two workers may run concurrently; each child has
a five-second deadline and 16 KiB limits on each output pipe. A disconnected waiter
does not release admission while its worker still owns a child. Shutdown closes
admission and drains those workers before stores close. There is no positive cache;
trust is reloaded before each check and rechecked before publishing a positive.

## REST API

- `GET /api/v1/repos/{owner}/{repo}/git/signatures/{id}` requires repository read
  access and a full commit or annotated-tag object id. It returns `object_id`,
  `kind`, and `state`.
- `GET` / `POST /api/v1/repos/{owner}/{repo}/signing_keys` require an administrator
  credential and repository administration. POST accepts `name`, `kind` (`ssh` or
  `openpgp`) and `public_key` (at most 64 KiB).
- `DELETE /api/v1/repos/{owner}/{repo}/signing_keys/{id}` revokes that repository's
  trust entry. Limited tokens cannot manage trust even if their nominal scope is admin.

Responses are not cacheable. Public verification never returns stored public-key
material. See [Git signature verification](https://git-scm.com/docs/git-verify-commit)
and [GnuPG import controls](https://www.gnupg.org/documentation/manuals/gnupg/GPG-Input-and-Output.html).
