//! Git change enumeration and bounded ownership-path regression tests.
use gix::objs::tree::EntryKind;

use super::*;

fn tree<'a>(repo: &'a gix::Repository, entries: &[(&str, EntryKind, ObjectId)]) -> gix::Tree<'a> {
    let mut entries = entries
        .iter()
        .map(|(name, kind, id)| gix::objs::tree::Entry { mode: (*kind).into(), filename: (*name).into(), oid: *id })
        .collect::<Vec<_>>();
    entries.sort();
    let id = repo.write_object(gix::objs::Tree { entries }).unwrap();
    repo.find_tree(id).unwrap()
}

#[test]
fn ownership_paths_include_file_tree_and_submodule_replacements() {
    let dir = tempfile::tempdir().unwrap();
    let repo = gix::init_bare(dir.path()).unwrap();
    let blob = repo.write_blob("content").unwrap().detach();
    let nested = tree(&repo, &[("inside", EntryKind::Blob, blob)]);
    let file = tree(&repo, &[("owned", EntryKind::Blob, blob)]);
    let directory = tree(&repo, &[("owned", EntryKind::Tree, nested.id)]);
    // A gitlink is never dereferenced by the path-only walk.
    let submodule = tree(&repo, &[("owned", EntryKind::Commit, ObjectId::from_hex(b"1111111111111111111111111111111111111111").unwrap())]);
    for (old, new) in [(&file, &directory), (&directory, &file), (&file, &submodule), (&submodule, &file), (&directory, &submodule)] {
        let (changes, incomplete) = bounded_changes_with_rewrites(&repo, Some(old), new, 10, false).unwrap();
        assert!(!incomplete);
        let paths: Vec<_> = changes.iter().map(|change| change.location().to_str().unwrap()).collect();
        assert!(paths.contains(&"owned"), "owned file path must survive a mode change: {paths:?}");
    }
    let (_, incomplete) = bounded_changes_with_rewrites(&repo, Some(&file), &directory, 1, false).unwrap();
    assert!(incomplete, "replacement and child exceed a one-path limit");
}

#[test]
fn bounded_changes_keep_the_full_diff_prefix_including_renames() {
    let dir = tempfile::tempdir().unwrap();
    let repo = gix::init_bare(dir.path()).unwrap();
    let blob = |text| repo.write_blob(text).unwrap().detach();
    let before = blob("one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\n");
    let after = blob("one\ntwo\nthree\nfour\nfive\nsix\nseven\nEIGHT\n");
    let exact = blob("another file with entirely distinct content\n");
    let nested = tree(&repo, &[("inside.txt", EntryKind::Blob, blob("nested file\n"))]);
    let old = tree(
        &repo,
        &[
            ("edit.txt", EntryKind::Blob, blob("old\n")),
            ("exact-before.txt", EntryKind::Blob, exact),
            ("fuzzy-before.txt", EntryKind::Blob, before),
            ("old-dir", EntryKind::Tree, nested.id),
        ],
    );
    let new = tree(
        &repo,
        &[
            ("edit.txt", EntryKind::Blob, blob("new\n")),
            ("exact-after.txt", EntryKind::Blob, exact),
            ("fuzzy-after.txt", EntryKind::Blob, after),
            ("new-dir", EntryKind::Tree, nested.id),
            ("unrelated.txt", EntryKind::Blob, blob("unrelated addition\n")),
        ],
    );
    // The previous eager implementation is the reference for classification and
    // ordering, including directory moves and similarity-based rename matching.
    let options = gix::diff::Options::default().with_rewrites(Some(gix::diff::Rewrites::default()));
    let expected = repo
        .diff_tree_to_tree(Some(&old), Some(&new), options)
        .unwrap()
        .into_iter()
        .filter(|change| describe(change.clone()).is_some())
        .collect::<Vec<_>>();
    for path in ["exact-after.txt", "fuzzy-after.txt", "new-dir/inside.txt"] {
        assert!(
            expected.iter().any(|change| {
                describe(change.clone()).is_some_and(|(name, _, status, _, _)| name == path && status == ChangeStatus::Renamed)
            }),
            "{path}: {expected:?}"
        );
    }
    for limit in 0..=expected.len() + 1 {
        let (actual, omitted) = bounded_changes(&repo, Some(&old), &new, limit).unwrap();
        assert_eq!(actual, expected[..limit.min(expected.len())], "file limit {limit}");
        assert_eq!(omitted, limit < expected.len());
    }
}

#[test]
fn file_limits_stop_before_an_unneeded_subtree_is_loaded() {
    let dir = tempfile::tempdir().unwrap();
    let repo = gix::init_bare(dir.path()).unwrap();
    let before = repo.write_blob("old\n").unwrap().detach();
    let after = repo.write_blob("new\n").unwrap().detach();
    let missing = ObjectId::from_hex(b"ffffffffffffffffffffffffffffffffffffffff").unwrap();
    let old = tree(&repo, &[("a.txt", EntryKind::Blob, before), ("b.txt", EntryKind::Blob, before)]);
    let new = tree(&repo, &[("a.txt", EntryKind::Blob, after), ("b.txt", EntryKind::Blob, after), ("z-late", EntryKind::Tree, missing)]);
    let limits = DiffLimits { max_files: 1, ..DiffLimits::default() };
    let (files, omitted) = diff_trees(&repo, Some(&old), &new, limits).unwrap();
    assert!(omitted);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "a.txt");
    assert_eq!((files[0].additions, files[0].deletions), (1, 1));
    // No timing assertion: the missing tree makes any unnecessary traversal fail.
    assert!(bounded_changes(&repo, Some(&old), &new, usize::MAX).is_err());
    let (files, omitted) = bounded_changes(&repo, None, &new, 0).unwrap();
    assert!(files.is_empty() && omitted, "a zero-file budget needs no rename discovery");
    let (files, omitted) = bounded_changes(&repo, Some(&old), &old, 0).unwrap();
    assert!(files.is_empty() && !omitted, "an unchanged tree is not truncated");
}
