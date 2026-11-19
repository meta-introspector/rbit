//! Shared immutable Git workflow fixtures for Actions and deployment page tests.
// Actions: dispatch form.
/// Commits `files` to `alice/rocket`'s `main`.
pub(super) fn push(repo: &rbit_services::RepoContext, files: &[(&str, &str)]) {
    let work = repo.disk_path.with_extension("work");
    std::fs::create_dir_all(&work).unwrap();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(["-c", "user.name=Ada", "-c", "user.email=ada@example.com", "-c", "init.defaultBranch=main", "-c", "core.autocrlf=false"])
            .args(args)
            .current_dir(&work)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", if cfg!(windows) { "NUL" } else { "/dev/null" })
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    };
    git(&["init", "-q"]);
    for (path, contents) in files {
        let file = work.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, contents).unwrap();
    }
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "Workflows"]);
    git(&["push", "-q", repo.disk_path.to_str().unwrap(), "HEAD:refs/heads/main"]);
}

pub(super) const RELEASE: &str = "name: Release
on:
  workflow_dispatch:
    inputs:
      version: {description: The version to tag, required: true}
      channel: {type: choice, options: [stable, beta], default: stable}
      dry-run: {type: boolean}
jobs:
  tag:
    runs-on: ubuntu-latest
    steps:
      - run: echo tagging
";
