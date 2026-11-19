import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { checkAccessPolicyRetirement } from "../rewrite/access-policy-retirement.mjs";
import { authoredSources } from "../rewrite/check.mjs";

test("the retired policy adapter and its rules cannot return", () => {
  assert.match(checkAccessPolicyRetirement([["crates/rbit-auth/src/policy.rs", "//! anything\n"]])[0], /retired policy adapter/u);
  for (const source of ["pub fn repo_access(viewer: Option<&User>) -> RepoAccess { todo!() }", "fn grant_holder() {}", "pub struct Grants { pub team: Option<Permission> }"]) {
    assert.match(checkAccessPolicyRetirement([["crates/rbit-services/src/repositories.rs", source]])[0], /belong in rbit_access::grants/u, source);
  }
  const compat = "pub type Grants = RepositoryGrants<Permission>;\npub fn repo_access(viewer: Option<&User>) -> RepoAccess { todo!() }\n";
  assert.deepEqual(checkAccessPolicyRetirement([["crates/rbit-core/src/access.rs", compat]]), []);
  assert.match(checkAccessPolicyRetirement([["crates/rbit-core/src/access.rs", "fn grants_apply() {}"]])[0], /projects records only/u);
  assert.deepEqual(checkAccessPolicyRetirement([["crates/rbit-access/src/grants.rs", "pub fn grants_apply() {}\npub struct RepositoryGrants<P> {}"]]), []);
  const runGrants = "crates/rbit-actions/src/trust.rs";
  assert.deepEqual(checkAccessPolicyRetirement([[runGrants, "pub struct Grants { pub write_token: bool }"]]), [], "run grants are not repository grants");
  assert.match(checkAccessPolicyRetirement([[runGrants, "fn repo_access() {}"]])[0], /belong in rbit_access::grants/u);
});

test("fact types are built only by the record adapters", () => {
  const facts = "let viewer = ViewerAccessFacts { site_admin: false, ..ViewerAccessFacts::default() };";
  assert.match(checkAccessPolicyRetirement([["crates/rbit-services/src/lfs.rs", facts]])[0], /build access facts/u);
  assert.deepEqual(checkAccessPolicyRetirement([["crates/rbit-storage/src/sqlite/wiki_mutations/authorization.rs", facts]]), []);
});

test("SQL readability is written once in readable_by!", () => {
  const sql = `"WHERE (r.visibility = 'public' OR r.owner_id = ?1)"`;
  assert.match(checkAccessPolicyRetirement([["crates/rbit-storage/src/sqlite/stars.rs", sql]])[0], /readable_by!/u);
  assert.deepEqual(checkAccessPolicyRetirement([["crates/rbit-storage/src/sqlite/repositories.rs", sql]]), []);
  const counts = `"SELECT COUNT(*) FROM repositories WHERE visibility = 'public'"`;
  assert.deepEqual(checkAccessPolicyRetirement([["crates/rbit-storage/src/sqlite/admin.rs", counts]]), [], "counting is not a readability filter");
});

test("delegated SQL has one explicit owner below the shared readability predicate", () => {
  const owner = "crates/rbit-storage/src/sqlite/agent_accounts/queries.rs";
  const helper = `macro_rules! repository_permission_sql { () => { "COALESCE(member_permission, 'write') <> 'none'" }; }`;
  assert.deepEqual(checkAccessPolicyRetirement([[owner, helper]]), []);
  for (const path of [
    "crates/rbit-storage/src/sqlite/agent_accounts/other.rs",
    "crates/rbit-storage/src/sqlite/other/queries.rs",
    "crates/rbit-storage/src/sqlite/repositories.rs",
    "crates/rbit-services/src/agents.rs",
    "crates/rbit-access/src/grants.rs",
  ]) {
    assert.match(checkAccessPolicyRetirement([[path, helper]])[0], /defined only/u, path);
  }
  const copiedAcl = `"COALESCE(member_permission, 'write') <> 'none'"`;
  for (const path of ["crates/rbit-storage/src/sqlite/agent_accounts/other.rs", "crates/rbit-storage/src/sqlite/stars.rs"]) {
    assert.match(checkAccessPolicyRetirement([[path, copiedAcl]])[0], /readable_by!/u, path);
  }
  assert.match(checkAccessPolicyRetirement([[owner, copiedAcl]])[0], /must define repository_permission_sql!/u);
  assert.match(checkAccessPolicyRetirement([[owner, `${helper} ${helper}`]])[0], /exactly once/u);
  assert.match(checkAccessPolicyRetirement([[owner, `${helper} readable_by!("?1", "0")`]])[0], /cannot recurse/u);
  assert.deepEqual(checkAccessPolicyRetirement([["crates/rbit-storage/src/sqlite/agent_accounts/scopes.rs",
    'const SQL: &str = concat!("SELECT ", super::queries::repository_permission_sql!("?1", "?2"));']]), []);
});

test("the current tree has no retired access path", () => {
  const { files } = authoredSources(fileURLToPath(new URL("../../", import.meta.url)));
  assert.equal(files.filter(([path]) => path === "crates/rbit-storage/src/sqlite/agent_accounts/queries.rs").length, 1,
    "the canonical delegated SQL owner must exist exactly once in the source tree");
  assert.deepEqual(checkAccessPolicyRetirement(files), []);
});
