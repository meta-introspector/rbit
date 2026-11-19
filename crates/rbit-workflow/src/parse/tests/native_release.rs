//! The shipping native release workflow must parse with publication disabled by default.
use super::support::parse_ok;
use crate::model::{InputKind, Permission};
use crate::spec::{BuiltinKind, StepAction};

#[test]
fn native_release_workflow_requires_opt_in_and_a_protected_publication_job() {
    let workflow = parse_ok(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.rbit/workflows/release.yml")));
    assert_eq!(workflow.warnings().len(), 0);
    assert!(workflow.on.push.is_none());
    let inputs = &workflow.on.workflow_dispatch.as_ref().unwrap().inputs;
    let mut names: Vec<_> = inputs.iter().map(|input| input.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["containers", "publish"]);
    assert!(inputs.iter().all(|input| input.kind == InputKind::Boolean && input.default.as_deref() == Some("false")));
    assert_eq!(workflow.permissions.as_ref().unwrap().contents, Permission::Read);
    let candidate = workflow.job("candidate").unwrap();
    let StepAction::Uses(checkout) = &candidate.steps[0].action else { panic!("candidate must start with builtin checkout") };
    assert_eq!(checkout.kind, BuiltinKind::Checkout);
    assert_eq!(checkout.with.get("fetch-tags").map(String::as_str), Some("true"));
    let publish = workflow.job("publish").unwrap();
    assert_eq!(publish.needs, ["candidate"]);
    assert_eq!(publish.condition.as_deref(), Some("${{ inputs.publish }}"));
    assert_eq!(publish.environment.as_ref().unwrap().name, "release");
    assert_eq!(publish.permissions.as_ref().unwrap().contents, Permission::Write);
}
