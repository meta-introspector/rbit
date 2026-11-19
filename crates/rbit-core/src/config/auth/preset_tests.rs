//! Provider setup presets.

use super::*;
#[test]
fn presets_are_disabled_drafts_and_keep_credentials_required_when_enabled() {
    for name in ["google", "github", "discord", "oidc"] {
        let mut provider = ProviderConfig::preset(name).unwrap();
        assert!(provider.disabled);
        assert!(AuthConfig { providers: vec![provider.clone()], ..Default::default() }.validate().is_ok());
        provider.disabled = false;
        assert!(AuthConfig { providers: vec![provider], ..Default::default() }.validate().is_err());
    }
    assert_eq!(ProviderConfig::preset("discord").unwrap().effective_scopes(), ["identify", "email"]);
    assert!(ProviderConfig::preset("unknown").is_none());
}
