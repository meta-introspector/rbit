//! Real SSH/GPG signatures over Git objects, repository trust and immediate revocation.
use rbit_core::{Config, User, Visibility};
use rbit_services::{CreateRepository, Services, signatures::SignatureState};
use rbit_storage::{Database, KvStore, NewUser};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
};

struct Fixture {
    directory: tempfile::TempDir,
    services: Services,
    user: User,
    work: PathBuf,
}
impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.storage.data_dir = directory.path().join("data");
        std::fs::create_dir_all(config.storage.repositories_dir()).unwrap();
        let database = Database::open(&config.storage.sqlite_path()).await.unwrap();
        let kv = KvStore::open(&config.storage.kv_path()).unwrap();
        let services = Services::new(Arc::new(config), &database, &kv).await.unwrap();
        let user = database
            .accounts()
            .create_user(NewUser {
                name: "signer".parse().unwrap(),
                email: "signer@example.com".parse().unwrap(),
                display_name: None,
                password_hash: "not-a-password".into(),
                is_admin: false,
            })
            .await
            .unwrap();
        services
            .repositories
            .create(&user, CreateRepository { name: "signed".into(), description: String::new(), visibility: Visibility::Public })
            .await
            .unwrap();
        let work = directory.path().join("work");
        std::fs::create_dir(&work).unwrap();
        git(&work, &["init", "-q"]);
        std::fs::write(work.join("hello.txt"), "hello\n").unwrap();
        git(&work, &["add", "."]);
        Self { directory, services, user, work }
    }
    async fn repo(&self) -> rbit_services::RepoContext {
        self.services.repositories.open(Some(&self.user), "signer", "signed").await.unwrap()
    }
    async fn push(&self) -> String {
        let repo = self.repo().await;
        git(&self.work, &["push", "--force", repo.disk_path.to_str().unwrap(), "HEAD:refs/heads/main"]);
        git(&self.work, &["rev-parse", "HEAD"]).trim().to_owned()
    }
}
fn run(command: &mut Command) -> String {
    let output = command.output().unwrap();
    assert!(output.status.success(), "command failed: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}
fn git(root: &Path, args: &[&str]) -> String {
    run(Command::new("git")
        .current_dir(root)
        .args(["-c", "user.name=Signer", "-c", "user.email=signer@example.com", "-c", "init.defaultBranch=main"])
        .args(args))
}
fn available(program: &str) -> bool {
    Command::new(program).arg("--version").output().is_ok()
}

fn tampered_commit(work: &Path, repository: &Path, id: &str) -> String {
    let mut raw = git(work, &["cat-file", "commit", id]).into_bytes();
    raw.extend_from_slice(b"tampered\n");
    let mut child = Command::new("git")
        .arg("--git-dir")
        .arg(repository)
        .args(["hash-object", "-w", "-t", "commit", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&raw).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[tokio::test]
async fn signatures_ssh_commit_tag_bad_signature_and_removed_trust() {
    if !available("ssh-keygen") {
        return;
    }
    let f = Fixture::new().await;
    let key = f.directory.path().join("signing");
    run(Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-f"]).arg(&key));
    git(&f.work, &["config", "gpg.format", "ssh"]);
    git(&f.work, &["config", "gpg.ssh.program", "ssh-keygen"]);
    git(&f.work, &["config", "user.signingkey", key.to_str().unwrap()]);
    git(&f.work, &["commit", "-S", "-qm", "signed"]);
    let id = f.push().await;
    let repo = f.repo().await;
    assert_eq!(f.services.signatures.verify(&repo, id.clone()).await.unwrap().state, SignatureState::Unknown);
    f.services
        .signatures
        .add_key(&f.user, &repo, "release".into(), "ssh".into(), std::fs::read_to_string(key.with_extension("pub")).unwrap())
        .await
        .unwrap();
    assert_eq!(f.services.signatures.verify(&repo, id.clone()).await.unwrap().state, SignatureState::Verified);
    git(&f.work, &["tag", "-s", "v1", "-m", "signed tag"]);
    git(&f.work, &["push", repo.disk_path.to_str().unwrap(), "refs/tags/v1"]);
    let tag_id = repo.reader.signature_tag_id("v1".into()).await.unwrap();
    let tag = f.services.signatures.verify(&repo, tag_id).await.unwrap();
    assert_eq!(tag.kind, "tag");
    assert_eq!(tag.state, SignatureState::Verified);
    let bad = tampered_commit(&f.work, &repo.disk_path, &id);
    assert_eq!(f.services.signatures.verify(&repo, bad).await.unwrap().state, SignatureState::Invalid);
    let trusted = f.services.signatures.list_keys(&repo).await.unwrap();
    f.services.signatures.delete_key(&f.user, &repo, trusted[0].id).await.unwrap();
    assert_eq!(f.services.signatures.verify(&repo, id).await.unwrap().state, SignatureState::Unknown);
    f.services.signatures.shutdown().await;
}

fn fixture_gpg(home: &Path) -> Command {
    let mut command = Command::new("gpg");
    command.current_dir(home).args(["--homedir", "."]);
    command
}

fn sign_openpgp(f: &Fixture, args: &[&str]) {
    run(Command::new("git")
        .current_dir(&f.work)
        .env("GNUPGHOME", "../gnupg")
        .args([
            "-c",
            "user.name=Signer",
            "-c",
            "user.email=signer@example.com",
            "-c",
            "user.signingkey=signer@example.com",
            "-c",
            "gpg.program=gpg",
            "-c",
            "gpg.format=openpgp",
        ])
        .args(args));
}

#[tokio::test]
async fn signatures_openpgp_verifies_with_only_enrolled_keys() {
    let installed = available("gpg");
    assert!(!cfg!(target_os = "linux") || installed, "Linux signature coverage requires GnuPG");
    if !installed {
        return;
    }
    let f = Fixture::new().await;
    let home = f.directory.path().join("gnupg");
    std::fs::create_dir(&home).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    run(fixture_gpg(&home).args([
        "--batch",
        "--pinentry-mode",
        "loopback",
        "--passphrase",
        "",
        "--quick-generate-key",
        "Signer <signer@example.com>",
        "ed25519",
        "sign",
        "0",
    ]));
    let public = run(fixture_gpg(&home).args(["--armor", "--export", "signer@example.com"]));
    let secret = run(fixture_gpg(&home).args([
        "--batch",
        "--pinentry-mode",
        "loopback",
        "--passphrase",
        "",
        "--armor",
        "--export-secret-keys",
        "signer@example.com",
    ]));
    let disguised = secret.replace("PRIVATE KEY BLOCK", "PUBLIC KEY BLOCK");
    let repo = f.repo().await;
    assert!(f.services.signatures.add_key(&f.user, &repo, "secret".into(), "openpgp".into(), disguised).await.is_err());
    assert_eq!(f.services.signatures.list_keys(&repo).await.unwrap(), []);

    sign_openpgp(&f, &["commit", "-S", "-qm", "signed"]);
    let id = f.push().await;
    let repo = f.repo().await;
    assert_eq!(f.services.signatures.verify(&repo, id.clone()).await.unwrap().state, SignatureState::Unknown);
    f.services.signatures.add_key(&f.user, &repo, "release".into(), "openpgp".into(), public).await.unwrap();
    assert_eq!(f.services.signatures.verify(&repo, id.clone()).await.unwrap().state, SignatureState::Verified);
    sign_openpgp(&f, &["tag", "-s", "v1", "-m", "OpenPGP tag"]);
    git(&f.work, &["push", repo.disk_path.to_str().unwrap(), "refs/tags/v1"]);
    let tag_id = repo.reader.signature_tag_id("v1".into()).await.unwrap();
    let tag = f.services.signatures.verify(&repo, tag_id).await.unwrap();
    assert_eq!(tag.kind, "tag");
    assert_eq!(tag.state, SignatureState::Verified);
    let bad = tampered_commit(&f.work, &repo.disk_path, &id);
    assert_eq!(f.services.signatures.verify(&repo, bad).await.unwrap().state, SignatureState::Invalid);
    let key = f.services.signatures.list_keys(&repo).await.unwrap().remove(0);
    f.services.signatures.delete_key(&f.user, &repo, key.id).await.unwrap();
    assert_eq!(f.services.signatures.verify(&repo, id.clone()).await.unwrap().state, SignatureState::Unknown);
    let revocation_path = std::fs::read_dir(home.join("openpgp-revocs.d")).unwrap().next().unwrap().unwrap().path();
    let revocation = std::fs::read_to_string(revocation_path).unwrap();
    let start = revocation.find(":-----BEGIN PGP PUBLIC KEY BLOCK-----").unwrap() + 1;
    let revocation_file = f.directory.path().join("revoke.asc");
    std::fs::write(&revocation_file, &revocation[start..]).unwrap();
    run(fixture_gpg(&home).args(["--batch", "--import"]).arg("../revoke.asc"));
    let revoked = run(fixture_gpg(&home).args(["--armor", "--export", "signer@example.com"]));
    f.services.signatures.add_key(&f.user, &repo, "revoked".into(), "openpgp".into(), revoked).await.unwrap();
    assert_eq!(f.services.signatures.verify(&repo, id).await.unwrap().state, SignatureState::Invalid);
    let _ = Command::new("gpgconf").current_dir(&home).args(["--homedir", "."]).args(["--kill", "all"]).output();
    f.services.signatures.shutdown().await;
}
