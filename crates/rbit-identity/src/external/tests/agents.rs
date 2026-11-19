//! Delegated identities cannot acquire or use interactive provider credentials.
use super::{CompletionError, OPEN, Port, finish, identity};

#[test]
fn agent_provider_signin_and_linking_are_refused_before_credential_effects() {
    let linked = Port { linked: Some(5), agent: true, ..Port::default() };
    assert!(matches!(finish(&linked, &OPEN, identity(None, false), None), Err(CompletionError::Forbidden)));
    assert_eq!(linked.log(), ["touch"]);
    let linking = Port { agent: true, ..Port::default() };
    assert!(matches!(finish(&linking, &OPEN, identity(None, false), Some(5)), Err(CompletionError::Forbidden)));
    assert_eq!(linking.log(), Vec::<String>::new());
}
