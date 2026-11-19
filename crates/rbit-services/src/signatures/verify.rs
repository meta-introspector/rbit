//! Verification against only the explicitly enrolled repository public keys.
use super::{SignatureState, process::execute};
use rbit_git::SignedObject;
use rbit_storage::sqlite::signing_keys::SigningKey;
use std::{path::Path, process::Stdio};
use tokio::process::Command;

pub(super) fn gpg(home: &Path) -> Command {
    let mut command = Command::new("gpg");
    command.current_dir(home);
    command
        .args(["--no-options", "--batch", "--no-tty", "--no-autostart", "--no-auto-key-retrieve", "--auto-key-locate", "clear"])
        .arg("--homedir")
        .arg(".");
    command
}

pub(super) async fn check(object: &SignedObject, keys: &[SigningKey]) -> SignatureState {
    let Some(signature) = &object.signature else { return SignatureState::Unsigned };
    let Ok(dir) = tempfile::tempdir() else { return SignatureState::Unavailable };
    let signature_path = dir.path().join("signature");
    let payload_path = dir.path().join("payload");
    if tokio::fs::write(&signature_path, signature).await.is_err() || tokio::fs::write(&payload_path, &object.payload).await.is_err() {
        return SignatureState::Unavailable;
    }
    if signature.starts_with(b"-----BEGIN SSH SIGNATURE-----") {
        ssh(dir.path(), &payload_path, signature, keys).await
    } else if signature.starts_with(b"-----BEGIN PGP SIGNATURE-----") {
        pgp(dir.path(), keys).await
    } else {
        SignatureState::Unavailable
    }
}

async fn ssh(root: &Path, payload_path: &Path, signature: &[u8], keys: &[SigningKey]) -> SignatureState {
    let Ok(sig) = ssh_key::SshSig::from_pem(signature) else { return SignatureState::Invalid };
    if sig.namespace() != "git" {
        return SignatureState::Invalid;
    }
    let trusted = keys
        .iter()
        .filter(|key| key.kind == "ssh")
        .filter_map(|key| ssh_key::PublicKey::from_openssh(&key.public_key).ok())
        .any(|key| key.key_data() == sig.public_key());
    if !trusted {
        return SignatureState::Unknown;
    }
    let mut allowed = String::new();
    for key in keys.iter().filter(|key| key.kind == "ssh") {
        allowed.push_str("rbit ");
        allowed.push_str(&key.public_key);
        allowed.push('\n');
    }
    let allowed_path = root.join("allowed_signers");
    if tokio::fs::write(&allowed_path, allowed).await.is_err() {
        return SignatureState::Unavailable;
    }
    let Ok(input) = std::fs::File::open(payload_path) else { return SignatureState::Unavailable };
    let mut command = Command::new("ssh-keygen");
    command.current_dir(root);
    command
        .args(["-Y", "verify", "-n", "git", "-I", "rbit", "-f"])
        .arg("allowed_signers")
        .arg("-s")
        .arg("signature")
        .stdin(Stdio::from(input));
    match execute(&mut command).await {
        Ok(output) if output.status.success() => SignatureState::Verified,
        Ok(output) => ssh_failure(&output.stderr),
        Err(_) => SignatureState::Unavailable,
    }
}

fn ssh_failure(stderr: &[u8]) -> SignatureState {
    // Unknown options, unsupported tools and local I/O failures do not disprove a signature.
    if String::from_utf8_lossy(stderr).contains("Signature verification failed:") {
        SignatureState::Invalid
    } else {
        SignatureState::Unavailable
    }
}

async fn pgp(root: &Path, keys: &[SigningKey]) -> SignatureState {
    let material = keys.iter().filter(|key| key.kind == "openpgp").map(|key| key.public_key.as_str()).collect::<Vec<_>>().join("\n");
    if material.is_empty() {
        return SignatureState::Unknown;
    }
    let keyfile = root.join("keys.asc");
    if tokio::fs::write(&keyfile, material).await.is_err() {
        return SignatureState::Unavailable;
    }
    let imported = execute(gpg(root).args(["--import"]).arg("keys.asc").stdin(Stdio::null())).await;
    if !imported.is_ok_and(|output| output.status.success()) {
        return SignatureState::Unavailable;
    }
    let output = execute(gpg(root).args(["--status-fd", "1", "--verify"]).arg("signature").arg("payload").stdin(Stdio::null())).await;
    let Ok(output) = output else { return SignatureState::Unavailable };
    let status = String::from_utf8_lossy(&output.stdout);
    if status.contains("[GNUPG:] NO_PUBKEY ") {
        return SignatureState::Unknown;
    }
    if ["BADSIG", "ERRSIG", "REVKEYSIG", "EXPKEYSIG", "EXPSIG"].iter().any(|code| status.contains(&format!("[GNUPG:] {code} "))) {
        return SignatureState::Invalid;
    }
    if output.status.success() && status.lines().any(|line| line.starts_with("[GNUPG:] VALIDSIG ")) {
        SignatureState::Verified
    } else {
        SignatureState::Invalid
    }
}

/// Parse public material with an isolated keyring before publishing trust.
pub(super) async fn valid_pgp_key(material: String) -> Result<bool, std::io::Error> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("key.asc");
    tokio::fs::write(&path, material).await?;
    let output =
        execute(gpg(dir.path()).args(["--with-colons", "--import-options", "show-only", "--import"]).arg("key.asc").stdin(Stdio::null()))
            .await?;
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(output.status.success()
        && text.lines().any(|line| line.starts_with("pub:"))
        && !text.lines().any(|line| line.starts_with("sec:") || line.starts_with("ssb:")))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signatures_gpg_uses_owned_cwd_and_relative_home_on_every_platform() {
        let root = Path::new("owned-verifier-directory");
        let command = gpg(root);
        assert_eq!(command.as_std().get_current_dir(), Some(root));
        let args: Vec<_> = command.as_std().get_args().collect();
        assert_eq!(&args[args.len() - 2..], [std::ffi::OsStr::new("--homedir"), std::ffi::OsStr::new(".")]);
    }
    #[test]
    fn signatures_unsupported_verifiers_do_not_invalidate_signatures() {
        assert_eq!(ssh_failure(b"unknown option -- Y\nusage: ssh-keygen"), SignatureState::Unavailable);
        assert_eq!(ssh_failure(b"Could not open allowed signers file"), SignatureState::Unavailable);
        assert_eq!(ssh_failure(b"Signature verification failed: incorrect signature"), SignatureState::Invalid);
    }
}
