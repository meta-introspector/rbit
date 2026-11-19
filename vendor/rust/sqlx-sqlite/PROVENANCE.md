# sqlx-sqlite 0.9.0 initialization compatibility patch

Source: published crates.io `sqlx-sqlite` 0.9.0, SQLx upstream revision
`003b698e99e024f3621b8043a2426fde5b741171`, subdirectory `sqlx-sqlite`.
The published archive SHA-256 is
`488e99c397a62007e4229aec669a179816339afc6d2620ca6fa420dbee2e982c`,
matching the original workspace lockfile. `UPSTREAM-SHA256.json` records all
56 original package files, excluding the package-local lock and Cargo cache marker.
The normalized and original manifests and upstream VCS marker remain present.

The published subcrate licenses contain relative references to the upstream root.
They are replaced here with the complete MIT and Apache-2.0 license texts from
`sqlx` 0.9.0, whose upstream VCS revision is identical. Original reference-file
hashes remain in `UPSTREAM-SHA256.json`.

The Rust-source changes are the `initialization` module and its registration,
plus the call at `src/connection/handle.rs`: publish native initialization success
in a process-wide Rust `OnceLock` before calling
`sqlite3_open_v2`. Every connection path in this driver uses that boundary, including
raw SQLx connections and pools. Preserve any native failure code as `SqliteError`;
a failed attempt releases a cold-path mutex so the next caller can retry.
Attempts are serialized only until success. Unit tests cover concurrent first
attempts, once-success publication, native error preservation and retry recovery.
No mutexes are held during normal connection use and tests remain concurrent.
Calling `sqlite3_shutdown()` after initialization is unsupported by this driver.

Observed failure: on macOS ARM64, the exact crashed executable UUID and return
address identify a call through a null SQLite `mutex.xMutexInit` pointer during
`sqlite3_initialize`. The bundled SQLite 0.37.0 dependency's mutex setup publishes
`xMutexAlloc` after a writer memory barrier but reads that marker without an
acquire barrier before calling `xMutexInit`. A concurrent first-use publication
race is the causal inference; the null callback and call location are observed.
The patch prevents concurrent driver initialization without claiming to repair
or fully diagnose SQLite's native memory model.

Revalidate with the fresh-process concurrent-first-open storage regression,
backup/restore integration tests, full workspace tests and strict Clippy.
Revisit this small patch on SQLx or SQLite upgrades; remove it only when equivalent
upstream initialization ordering has been established. This crate is excluded
from first-party workspace membership and retains normal license coverage.
