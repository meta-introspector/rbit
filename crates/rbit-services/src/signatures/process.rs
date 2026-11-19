//! Owned, bounded verifier subprocesses with host-controlled arguments and environment.
use std::{
    io,
    process::{Output, Stdio},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};

pub(super) async fn execute(command: &mut Command) -> io::Result<Output> {
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("LC_ALL", "C")
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(root) = std::env::var_os("SYSTEMROOT") {
        command.env("SYSTEMROOT", root);
    }
    // Win32 OpenSSH requires ProgramData before initializing even its stdio.
    // Verification still uses only the explicit, owned signer/signature files.
    #[cfg(windows)]
    if let Some(data) = std::env::var_os("ProgramData") {
        command.env("ProgramData", data);
    }
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().ok_or_else(|| io::Error::other("missing stdout"))?;
    let stderr = child.stderr.take().ok_or_else(|| io::Error::other("missing stderr"))?;
    let result =
        tokio::time::timeout(Duration::from_secs(5), async { tokio::try_join!(child.wait(), read_bounded(stdout), read_bounded(stderr)) })
            .await;
    if let Ok(Ok((status, stdout, stderr))) = result {
        Ok(Output { status, stdout, stderr })
    } else {
        let _ = child.kill().await;
        let _ = child.wait().await;
        Err(io::Error::other("signature verifier exceeded its time or output limit"))
    }
}
async fn read_bounded(input: impl AsyncRead + Unpin) -> io::Result<Vec<u8>> {
    let mut data = Vec::new();
    input.take(16 * 1024 + 1).read_to_end(&mut data).await?;
    if data.len() > 16 * 1024 {
        return Err(io::Error::other("verifier output exceeded limit"));
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn signatures_child_drops_untrusted_configuration_environment() {
        let mut command = if cfg!(windows) {
            let mut command = Command::new("cmd.exe");
            command.args(["/D", "/C", "set"]);
            command
        } else {
            Command::new("env")
        };
        for key in ["HOME", "GNUPGHOME", "SSH_AUTH_SOCK", "GIT_CONFIG"] {
            command.env(key, "rbit-untrusted-signature-environment");
        }
        let output = execute(&mut command).await.unwrap();
        assert!(output.status.success());
        assert!(!output.stdout.is_empty(), "environment probe must produce observable output");
        let environment = String::from_utf8_lossy(&output.stdout);
        assert!(!environment.contains("rbit-untrusted-signature-environment"));
        #[cfg(windows)]
        {
            let data = std::env::var("ProgramData").expect("Windows OpenSSH requires host ProgramData");
            assert!(environment.lines().any(|line| line.eq_ignore_ascii_case(&format!("ProgramData={data}"))));
        }
    }
    #[cfg(windows)]
    #[tokio::test]
    async fn signatures_windows_selected_verifier_supports_git_signature_arguments() {
        let dir = tempfile::tempdir().unwrap();
        let missing = "rbit-nonexistent-signature-capability-probe";
        let mut command = Command::new("ssh-keygen");
        command.current_dir(dir.path()).args(["-Y", "verify", "-n", "git", "-I", "rbit", "-f", "missing-signers", "-s", missing]);
        let Ok(output) = execute(&mut command).await else { return };
        let error = String::from_utf8_lossy(&output.stderr);
        let selected = execute(Command::new("where.exe").arg("ssh-keygen.exe")).await.unwrap();
        let version = execute(Command::new("ssh").arg("-V")).await.unwrap();
        assert!(
            error.contains("signature file"),
            "selected ssh-keygen cannot verify Git signatures: status={:?}; stdout={}; stderr={error}; root_present={}; where_status={:?}; where_stdout={}; where_stderr={}; version_status={:?}; version_stdout={}; version_stderr={}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            std::env::var_os("SYSTEMROOT").is_some(),
            selected.status.code(),
            String::from_utf8_lossy(&selected.stdout),
            String::from_utf8_lossy(&selected.stderr),
            version.status.code(),
            String::from_utf8_lossy(&version.stdout),
            String::from_utf8_lossy(&version.stderr)
        );
    }
    #[tokio::test]
    async fn signatures_missing_verifier_is_an_error() {
        assert!(execute(&mut Command::new("rbit-verifier-that-does-not-exist")).await.is_err());
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn signatures_output_limit_kills_and_reaps_child() {
        let start = std::time::Instant::now();
        assert!(execute(Command::new("sh").args(["-c", "exec yes x"])).await.is_err());
        assert!(start.elapsed() < Duration::from_secs(5));
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn signatures_timeout_kills_and_reaps_child() {
        let started = std::time::Instant::now();
        assert!(execute(Command::new("sh").args(["-c", "exec sleep 30"])).await.is_err());
        assert!(started.elapsed() >= Duration::from_secs(4));
    }
}

#[cfg(test)]
#[path = "process_transport_tests.rs"]
mod transport_tests;
