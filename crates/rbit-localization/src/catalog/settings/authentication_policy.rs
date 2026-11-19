//! MFA enrollment policies and identity provider setup.

labels! {
    /// Authentication policy and provider setup labels.
    PolicyMessage, "settings.authentication-policy" {
        CallbackUrl = "callback-url" => "Callback URL:",
        Security = "security" => "Security",
        TwoFactor = "two-factor" => "Two-factor authentication",
        RequireFactor = "require-factor" => "Require an authenticator app or passkey",
        Optional = "optional" => "Optional",
        Required = "required" => "Required",
        EnrollmentHelp = "enrollment-help" => "Users must enroll an authenticator app or passkey in their Security settings before continuing authenticated activity. Existing sessions, API tokens and Git access are checked. Sign-in, enrollment and sign-out stay available.",
        OwnerHelp = "owner-help" => "Set up your own authenticator app or passkey before enabling this requirement. Recovery codes remain available if a device is lost.",
        Save = "save" => "Save security policy",
        SiteIntro = "site-intro" => "Optional security requirements for every account on this site. Changes take effect immediately.",
        Providers = "providers" => "SSO and identity providers",
        ProvidersHelp = "providers-help" => "Set up Google, Discord, GitHub or an OpenID Connect provider. Disable a provider without deleting linked identities.",
        ConfigureProviders = "configure-providers" => "Configure sign-in providers",
        OrgSettings = "org-settings" => "Organization settings",
        OrgIntro = "org-intro" => "Optional MFA enrollment requirement for every member account, including owners. Changes take effect immediately.",
        SiteRequired = "site-required" => "This site already requires MFA. Making the organization policy optional will not override the site requirement.",
        OrgHelp = "org-help" => "Choose whether members must set up an authenticator app or passkey.",
        ManagePolicy = "manage-policy" => "Manage security policy",
        EnrollmentRequired = "enrollment-required" => "Your site or organization requires two-factor authentication. Add a passkey or set up an authenticator app below to continue.",
        AddGoogle = "add-google" => "Add Google",
        AddDiscord = "add-discord" => "Add Discord",
        AddGithub = "add-github" => "Add GitHub",
        AddOidc = "add-oidc" => "Add OIDC / SSO",
    }
}
