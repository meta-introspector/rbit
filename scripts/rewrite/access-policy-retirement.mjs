/** Retired repository access paths must not return; see docs/design/rewrite/access-policy-acceptance.md. */

const COMPAT = "crates/rbit-core/src/access.rs";
// The grant rules are defined only in rbit-access; rbit-core keeps the record projection.
const POLICY = /\bfn\s+(?:repo_access|grant_holder|policy_grants|grants_apply)\b|\bstruct\s+(?:Grants|RepositoryGrants)\b/u;
// `rbit_actions::trust::Grants` is what a stored run is granted (secrets, write token), not
// repository access; that file may name the struct but not the grant rule functions.
const RUN_GRANTS = "crates/rbit-actions/src/trust.rs";
const POLICY_FNS = /\bfn\s+(?:repo_access|grant_holder|policy_grants|grants_apply)\b/u;
// Only adapters that project stored rows may build the policy's fact types directly.
const FACTS = /\b(?:ViewerAccessFacts|RepositoryAccessFacts)\s*\{/u;
const FACT_ADAPTERS = [COMPAT, "crates/rbit-storage/src/sqlite/wiki_mutations/authorization.rs"];
// Listings enter through readable_by!; its agent branch uses the canonical effective
// permission helper. The helper evaluates human-parent rights and cannot recurse
// through readable_by!, which would enter the same agent branch again.
const SQL_RULE = /visibility\s*=\s*'public'\s+OR|'write'\)\s*<>\s*'none'/u;
const SQL_OWNER = "crates/rbit-storage/src/sqlite/repositories.rs";
const AGENT_SQL_OWNER = "crates/rbit-storage/src/sqlite/agent_accounts/queries.rs";
const AGENT_SQL_DEFINITION = /\bmacro_rules!\s*repository_permission_sql\b/gu;

/** Comments are not code; strip line and block comments before matching. */
function code(source) {
  return source.replace(/\/\*[\s\S]*?\*\//gu, "").replace(/\/\/[^\n]*/gu, "");
}

/** Takes authored [path, source] pairs; returns violations naming each path and rule. */
export function checkAccessPolicyRetirement(sources) {
  const violations = [];
  for (const [path, source] of sources) {
    if (path === "crates/rbit-auth/src/policy.rs") violations.push(`${path}: the retired policy adapter returned; use rbit_access::grants`);
    if (!path.startsWith("crates/") || !path.endsWith(".rs")) continue;
    const text = code(source);
    const agentDefinitions = [...text.matchAll(AGENT_SQL_DEFINITION)].length;
    if (agentDefinitions > 0 && path !== AGENT_SQL_OWNER) {
      violations.push(`${path}: delegated SQL permission is defined only in ${AGENT_SQL_OWNER}`);
    }
    if (path === AGENT_SQL_OWNER) {
      if (agentDefinitions !== 1) violations.push(`${path}: the canonical delegated SQL module must define repository_permission_sql! exactly once`);
      if (/\breadable_by\s*!/u.test(text)) violations.push(`${path}: the delegated permission helper cannot recurse through readable_by!`);
    }
    if (path.startsWith("crates/rbit-access/")) continue;
    const tests = path.includes("/tests/");
    if (!tests && path !== COMPAT && (path === RUN_GRANTS ? POLICY_FNS : POLICY).test(text)) {
      violations.push(`${path}: repository grant rules belong in rbit_access::grants`);
    }
    if (path === COMPAT && /\bfn\s+(?:grant_holder|policy_grants|grants_apply)\b|\bstruct\s+(?:Grants|RepositoryGrants)\b/u.test(text)) {
      violations.push(`${path}: the compatibility path projects records only; grant rules belong in rbit_access::grants`);
    }
    if (!tests && !FACT_ADAPTERS.includes(path) && FACTS.test(text)) {
      violations.push(`${path}: build access facts through rbit_access::grants or rbit_core::access, not by hand`);
    }
    if (path.startsWith("crates/rbit-storage/src/") && path !== SQL_OWNER && path !== AGENT_SQL_OWNER && SQL_RULE.test(text)) {
      violations.push(`${path}: SQL readability must use the shared readable_by! predicate`);
    }
  }
  return violations;
}
