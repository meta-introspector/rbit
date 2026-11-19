//! New authentication controls preserve the English/Spanish catalog contract.
use rbit_localization::{Locale, MessageFamily, settings::authentication_policy::PolicyMessage};

#[test]
fn every_authentication_policy_label_has_a_spanish_entry() {
    let mut ids = std::collections::BTreeSet::new();
    for &message in PolicyMessage::ALL {
        assert!(ids.insert(message.id()));
        for locale in [Locale::English, Locale::Spanish] {
            let text = message.text(&locale);
            assert_ne!(text.as_str(), "");
            assert!(!text.fallback(), "{message:?} in {locale:?}");
        }
    }
}
