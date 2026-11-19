//! Repository protection settings cards.

use super::{RepoHeader, SettingsCard, rules_url};

/// Cards this workstream adds to the repository settings page ("Branch protection",
/// icon `shield`).
pub fn repo_settings_cards(header: &RepoHeader) -> Vec<SettingsCard> {
    vec![
        SettingsCard {
            url: format!("{}/tags", header.settings_url),
            icon: "shield",
            title: rbit_localization::settings::cards::CardMessage::TagProtection,
            description: rbit_localization::settings::cards::CardMessage::TagProtectionText,
        },
        SettingsCard {
            url: rules_url(header),
            icon: "shield",
            title: rbit_localization::settings::cards::CardMessage::Protection,
            description: rbit_localization::settings::cards::CardMessage::ProtectionText,
        },
    ]
}
