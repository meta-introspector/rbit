//! Every agent management label has complete English and Spanish text.
use rbit_localization::{Locale, settings::agents::AgentsMessage};
#[test]
fn agent_labels_are_complete_in_english_and_spanish() {
    for message in AgentsMessage::ALL {
        for locale in [Locale::English, Locale::Spanish] {
            let text = message.text(&locale);
            assert!(!text.as_str().is_empty(), "{message:?}");
            assert!(!text.fallback(), "{message:?}");
        }
    }
}
