//! Editor tree planning regressions.
use super::*;

fn path(path: &str) -> FilePath {
    FilePath::parse(path).unwrap()
}

fn blob(digit: char) -> BlobId {
    BlobId(digit.to_string().repeat(40))
}

/// `README` (file), `run.sh` (executable), `link` (symlink), `docs/a.md`, `lib` (submodule).
fn tree(path: &str) -> Option<Entry> {
    let entry = |mode| Some(Entry { mode, id: "f".repeat(40) });
    match path {
        "README" | "docs/a.md" => entry(Mode::File),
        "run.sh" => entry(Mode::Executable),
        "link" => entry(Mode::Symlink),
        "docs" => entry(Mode::Directory),
        "lib" => entry(Mode::Submodule),
        _ => None,
    }
}

fn message(result: Result<Vec<IndexLine>, EditError>) -> String {
    match result {
        Err(EditError::Rejected(message)) => message,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn writes_keep_modes_and_removals_come_first() {
    let changes = [
        Change::Update { path: path("run.sh"), blob: blob('1') },
        Change::Move { from: path("link"), to: path("new/dir/link"), blob: None },
        Change::Delete { path: path("docs/a.md") },
        Change::Create { path: path("docs/b.md"), blob: blob('2') },
    ];
    let lines = plan(&changes, tree).unwrap();
    assert_eq!(
        lines,
        [
            IndexLine::Remove("docs/a.md".into()),
            IndexLine::Remove("link".into()),
            IndexLine::Write { mode: Mode::File, id: "2".repeat(40), path: "docs/b.md".into() },
            IndexLine::Write { mode: Mode::Symlink, id: "f".repeat(40), path: "new/dir/link".into() },
            IndexLine::Write { mode: Mode::Executable, id: "1".repeat(40), path: "run.sh".into() },
        ]
    );
    let mut bytes = Vec::new();
    lines[0].write_to(&mut bytes);
    assert_eq!(bytes, format!("0 {}\tdocs/a.md\0", "0".repeat(40)).as_bytes());
}

#[test]
fn nothing_is_clobbered() {
    assert_eq!(message(plan(&[Change::Create { path: path("README"), blob: blob('1') }], tree)), "a file called README already exists");
    assert_eq!(message(plan(&[Change::Create { path: path("docs"), blob: blob('1') }], tree)), "a directory called docs already exists");
    let onto = Change::Move { from: path("README"), to: path("run.sh"), blob: None };
    assert_eq!(message(plan(&[onto], tree)), "a file called run.sh already exists");
    assert_eq!(message(plan(&[Change::Put { path: path("docs"), blob: blob('1') }], tree)), "docs is a directory");
    // An upload replaces a file of the same name (keeping its mode) but a symlink becomes a file.
    let put = plan(&[Change::Put { path: path("run.sh"), blob: blob('1') }, Change::Put { path: path("link"), blob: blob('2') }], tree);
    assert_eq!(
        put.unwrap(),
        [
            IndexLine::Write { mode: Mode::File, id: "2".repeat(40), path: "link".into() },
            IndexLine::Write { mode: Mode::Executable, id: "1".repeat(40), path: "run.sh".into() },
        ]
    );
}

#[test]
fn edited_moved_and_deleted_paths_must_be_files() {
    assert_eq!(message(plan(&[Change::Update { path: path("nope"), blob: blob('1') }], tree)), "there is no file called nope");
    assert_eq!(message(plan(&[Change::Delete { path: path("docs") }], tree)), "docs is a directory, not a file");
    assert_eq!(message(plan(&[Change::Delete { path: path("lib") }], tree)), "lib is a submodule, not a file");
    assert_eq!(message(plan(&[Change::Update { path: path("docs"), blob: blob('1') }], tree)), "docs is not a file");
    let twice = [Change::Delete { path: path("README") }, Change::Delete { path: path("README") }];
    assert_eq!(message(plan(&twice, tree)), "README is changed twice");
}

#[test]
fn paths_never_run_through_files() {
    let through = Change::Create { path: path("README/x"), blob: blob('1') };
    assert_eq!(message(plan(&[through], tree)), "README is a file, so it cannot contain x");
    let both = [Change::Create { path: path("a"), blob: blob('1') }, Change::Create { path: path("a/b"), blob: blob('2') }];
    assert_eq!(message(plan(&both, tree)), "a cannot be both a file and a directory");
    let twice = [Change::Create { path: path("a"), blob: blob('1') }, Change::Put { path: path("a"), blob: blob('2') }];
    assert_eq!(message(plan(&twice, tree)), "a is written twice");
    assert_eq!(message(plan(&[Change::Create { path: path("lib/x"), blob: blob('1') }], tree)), "lib is a submodule");
    // A file that is moved away frees its name for a directory.
    let replaced = [Change::Move { from: path("README"), to: path("README/index.md"), blob: None }];
    assert_eq!(
        plan(&replaced, tree).unwrap(),
        [IndexLine::Remove("README".into()), IndexLine::Write { mode: Mode::File, id: "f".repeat(40), path: "README/index.md".into() }]
    );
}

#[test]
fn renaming_onto_itself_is_an_update() {
    let same = [Change::Move { from: path("README"), to: path("README"), blob: Some(blob('3')) }];
    assert_eq!(plan(&same, tree).unwrap(), [IndexLine::Write { mode: Mode::File, id: "3".repeat(40), path: "README".into() }]);
}
