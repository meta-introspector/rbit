//! Toolkit environment for a claimed job, including scoped OIDC request credentials.
use super::{BTreeMap, BackendKind, JobSpec, Loaded, View};

// Actions: third-party actions.
/// Adds what actions read to `task_env`: where `actions/setup-*` keep the tools they
/// download (the runner's own tool cache on the host, kept between jobs; the task's `temp`
/// in a container, which the job may write) and, when the `ACTIONS_STEP_DEBUG` secret or
/// variable is `true`, `RUNNER_DEBUG`. Returns whether debug logging is on.
pub(super) fn action_env(
    task_env: &mut BTreeMap<String, String>,
    loaded: &Loaded,
    backend: BackendKind,
    view: &View,
    spec: &JobSpec,
) -> Result<bool, String> {
    if let Some(url) = &spec.oidc_request_url {
        task_env.insert("ACTIONS_ID_TOKEN_REQUEST_URL".to_owned(), url.clone());
        task_env.insert("ACTIONS_ID_TOKEN_REQUEST_TOKEN".to_owned(), spec.token.expose().to_owned());
    }
    let tool_cache = match backend {
        BackendKind::Host => {
            let dir = loaded.work_dir.join("tool-cache");
            std::fs::create_dir_all(&dir).map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
            dir.display().to_string()
        }
        BackendKind::Container => format!("{}/_tool", view.temp),
    };
    for name in ["RUNNER_TOOL_CACHE", "AGENT_TOOLSDIRECTORY"] {
        task_env.insert(name.to_owned(), tool_cache.clone());
    }
    let step_debug = spec
        .secrets
        .get("ACTIONS_STEP_DEBUG")
        .map(|secret| secret.expose().to_owned())
        .or_else(|| spec.contexts.vars.get("ACTIONS_STEP_DEBUG").cloned())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("true"));
    if step_debug {
        task_env.insert("RUNNER_DEBUG".to_owned(), "1".to_owned());
    }
    Ok(step_debug)
}

#[cfg(test)]
mod tests {
    use crate::job::tests::{execute, host_job, step};
    use rbit_workflow::protocol::TaskResult;

    #[tokio::test]
    async fn oidc_toolkit_environment_is_scoped_and_request_token_is_masked() {
        if crate::process::find_program("sh").is_none() {
            return;
        }
        for allowed in [false, true] {
            let mut command =
                step("printf 'url=%s token=%s\\n' \"${ACTIONS_ID_TOKEN_REQUEST_URL:-none}\" \"${ACTIONS_ID_TOKEN_REQUEST_TOKEN:-none}\"");
            command.shell = Some("sh".to_owned());
            let mut spec = host_job(vec![command]);
            spec.oidc_request_url = allowed.then(|| "https://git.example.com/api/actions/oidc/token?api-version=1".to_owned());
            spec.template.permissions = Some(rbit_workflow::spec::TokenPermissions {
                contents: rbit_workflow::spec::Access::Read,
                issues: rbit_workflow::spec::Access::None,
                pull_requests: rbit_workflow::spec::Access::None,
                id_token: allowed,
            });
            let (result, reporter, _work) = execute(spec).await;
            let text = reporter.lines().join("\n");
            assert_eq!(result, TaskResult::Success, "{text}");
            assert!(
                text.contains(if allowed {
                    "url=https://git.example.com/api/actions/oidc/token?api-version=1 token=***"
                } else {
                    "url=none token=none"
                }),
                "{text}"
            );
            assert!(!text.contains(crate::testing::TOKEN));
        }
    }
}
