//! Spanish entries for authentication policies and provider setup.
use crate::catalog::{fallback::MessageWriter, settings::authentication_policy::PolicyMessage};

entries! { Es:
    PolicyMessage => MessageWriter |text| {
        PolicyMessage::CallbackUrl => text.literal("URL de devolución:"),
        PolicyMessage::Security => text.literal("Seguridad"),
        PolicyMessage::TwoFactor => text.literal("Autenticación de dos factores"),
        PolicyMessage::RequireFactor => text.literal("Requerir una aplicación de autenticación o una llave de acceso"),
        PolicyMessage::Optional => text.literal("Opcional"),
        PolicyMessage::Required => text.literal("Obligatoria"),
        PolicyMessage::EnrollmentHelp => text.literal("Los usuarios deben configurar una aplicación de autenticación o una llave de acceso en sus ajustes de Seguridad antes de continuar su actividad autenticada. Se comprueban las sesiones existentes, los tokens de API y el acceso Git. El inicio de sesión, la inscripción y el cierre de sesión siguen disponibles."),
        PolicyMessage::OwnerHelp => text.literal("Configura tu propia aplicación de autenticación o llave de acceso antes de activar este requisito. Los códigos de recuperación siguen disponibles si pierdes un dispositivo."),
        PolicyMessage::Save => text.literal("Guardar política de seguridad"),
        PolicyMessage::SiteIntro => text.literal("Requisitos de seguridad opcionales para todas las cuentas de este sitio. Los cambios se aplican inmediatamente."),
        PolicyMessage::Providers => text.literal("SSO y proveedores de identidad"),
        PolicyMessage::ProvidersHelp => text.literal("Configura Google, Discord, GitHub o un proveedor de OpenID Connect. Desactiva un proveedor sin borrar las identidades vinculadas."),
        PolicyMessage::ConfigureProviders => text.literal("Configurar proveedores de inicio de sesión"),
        PolicyMessage::OrgSettings => text.literal("Configuración de la organización"),
        PolicyMessage::OrgIntro => text.literal("Requisito opcional de inscripción en MFA para todas las cuentas miembro, incluidos los propietarios. Los cambios se aplican inmediatamente."),
        PolicyMessage::SiteRequired => text.literal("Este sitio ya requiere MFA. Hacer opcional la política de la organización no anula el requisito del sitio."),
        PolicyMessage::OrgHelp => text.literal("Elige si los miembros deben configurar una aplicación de autenticación o una llave de acceso."),
        PolicyMessage::ManagePolicy => text.literal("Gestionar política de seguridad"),
        PolicyMessage::EnrollmentRequired => text.literal("Tu sitio u organización requiere autenticación de dos factores. Añade una llave de acceso o configura una aplicación de autenticación a continuación para continuar."),
        PolicyMessage::AddGoogle => text.literal("Añadir Google"),
        PolicyMessage::AddDiscord => text.literal("Añadir Discord"),
        PolicyMessage::AddGithub => text.literal("Añadir GitHub"),
        PolicyMessage::AddOidc => text.literal("Añadir OIDC / SSO"),
    }
}
