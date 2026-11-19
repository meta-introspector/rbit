//! Authentication policy labels fall back to English until translated.
use crate::catalog::{fallback::MessageWriter, settings::authentication_policy::PolicyMessage};

entries! { Fr:
    PolicyMessage => MessageWriter |text| {}
}
