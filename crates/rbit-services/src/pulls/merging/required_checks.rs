//! Required status contexts after the push trigger decides no workflow will run.

/// Whether `context` is the shape of an rbit Actions job status for a push:
/// `<workflow> / <job> (push)`.
fn is_push_actions_context(context: &str) -> bool {
    context.contains(" / ") && context.ends_with(" (push)")
}

/// `required` without the checks of push runs when the push left every workflow out.
pub(super) fn required_after(required: Vec<String>, left_out: bool) -> Vec<String> {
    if !left_out {
        return required;
    }
    required.into_iter().filter(|context| !is_push_actions_context(context)).collect()
}

#[cfg(test)]
mod tests {
    use super::required_after;

    #[test]
    fn a_push_left_out_drops_only_its_own_checks() {
        let required: Vec<String> =
            ["CI / test (push)", "CI / test (pull_request)", "ci/external", "Jenkins / build (nightly)"].map(String::from).to_vec();
        assert_eq!(required_after(required.clone(), false), required, "a push that ran something keeps every check");
        assert_eq!(
            required_after(required, true),
            ["CI / test (pull_request)", "ci/external", "Jenkins / build (nightly)"],
            "pull request runs and other systems still report"
        );
    }
}
