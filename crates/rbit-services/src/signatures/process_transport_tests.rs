//! Real Git signature verification through the exact bounded child/file-stdin boundary.
use super::*;
use std::path::Path;
#[tokio::test]
async fn signatures_real_git_payload_verifies_with_inherited_file_stdin() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let work = root.join("work");
    std::fs::create_dir(&work).unwrap();
    let key = root.join("key");
    let Ok(generated) =
        std::process::Command::new("ssh-keygen").current_dir(root).args(["-q", "-t", "ed25519", "-N", "", "-f", "key"]).output()
    else {
        return;
    };
    assert!(generated.status.success(), "keygen failed: {}", String::from_utf8_lossy(&generated.stderr));
    diagnostic_git(&work, &["init", "-q", "--initial-branch=main"]);
    std::fs::write(work.join("README.md"), b"signature transport fixture\n").unwrap();
    diagnostic_git(&work, &["add", "README.md"]);
    diagnostic_git(&work, &["commit", "-S", "-qm", "signed transport fixture"]);
    let id = diagnostic_git(&work, &["rev-parse", "HEAD"]);
    let object = rbit_git::RepositoryReader::new(work.join(".git")).signed_object(id).await.unwrap();
    let signature = root.join("signature");
    let payload = root.join("payload");
    let allowed = root.join("allowed_signers");
    let public = std::fs::read_to_string(key.with_extension("pub")).unwrap();
    let public = public.split_whitespace().take(2).collect::<Vec<_>>().join(" ");
    std::fs::write(&signature, object.signature.unwrap()).unwrap();
    std::fs::write(&payload, object.payload).unwrap();
    std::fs::write(&allowed, format!("rbit {public}\n")).unwrap();
    let mut command = Command::new("ssh-keygen");
    command
        .current_dir(root)
        .args(["-Y", "verify", "-n", "git", "-I", "rbit", "-f"])
        .arg("allowed_signers")
        .arg("-s")
        .arg("signature")
        .stdin(Stdio::from(std::fs::File::open(&payload).unwrap()));
    let output = execute(&mut command).await.unwrap();
    assert!(
        output.status.success(),
        "bounded actual-verification diagnostics: status={}; stdout={}; stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn diagnostic_git(root: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .current_dir(root)
        .args([
            "-c",
            "user.name=Signer",
            "-c",
            "user.email=signer@example.test",
            "-c",
            "gpg.format=ssh",
            "-c",
            "gpg.ssh.program=ssh-keygen",
            "-c",
        ])
        .arg("user.signingkey=../key")
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "fixture Git failed: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
