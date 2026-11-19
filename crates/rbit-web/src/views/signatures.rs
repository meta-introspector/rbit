//! Signing trust settings and explicit verification state labels.
use super::{layout::Page, repo::RepoHeader};
use askama::Template;
use askama_web::WebTemplate;
use rbit_localization::{Locale, RepoBrowseLabel};
use rbit_services::signatures::SignatureState;
#[derive(serde::Deserialize)]
pub struct KeyInput {
    pub name: String,
    pub kind: String,
    pub public_key: String,
}
impl std::fmt::Debug for KeyInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("KeyInput").field("name", &self.name).field("kind", &self.kind).field("public_key", &"[redacted]").finish()
    }
}
#[derive(Debug, Template, WebTemplate)]
#[template(path = "signatures/keys.html")]
pub struct SigningKeysPage {
    pub page: Page,
    pub header: RepoHeader,
    pub keys: Vec<rbit_services::signatures::SigningKey>,
}
#[derive(Debug, Template, WebTemplate)]
#[template(path = "signatures/page.html")]
pub struct SignaturePage {
    pub page: Page,
    pub header: RepoHeader,
    pub object_id: String,
    pub kind: String,
    pub signature: SignatureFragment,
}
#[derive(Debug, Template, WebTemplate)]
#[template(path = "signatures/fragment.html")]
pub struct SignatureFragment {
    pub locale: Locale,
    pub state: SignatureState,
    pub url: String,
}
impl SignatureFragment {
    pub fn label(&self) -> RepoBrowseLabel {
        match self.state {
            SignatureState::Unsigned => RepoBrowseLabel::SignatureUnsigned,
            SignatureState::Verified => RepoBrowseLabel::SignatureVerified,
            SignatureState::Unknown => RepoBrowseLabel::SignatureUnknown,
            SignatureState::Invalid => RepoBrowseLabel::SignatureInvalid,
            SignatureState::Unavailable => RepoBrowseLabel::SignatureUnavailable,
        }
    }
}
