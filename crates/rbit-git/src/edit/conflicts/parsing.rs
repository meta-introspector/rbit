//! Parse complete stage records and load only bounded regular text objects.
use super::{
    BTreeMap, ConflictFile, EditError, EditWorkspace, FilePath, GitError, MAX_FILE_BYTES, MAX_FILES, MAX_TOTAL_BYTES, Mode, unsupported,
    validate_id,
};

#[derive(Debug, Default)]
pub(super) struct Stages {
    ours: Option<(String, String)>,
    theirs: Option<(String, String)>,
}

pub(super) fn parse(output: &[u8]) -> Result<(String, BTreeMap<String, Stages>), EditError> {
    let text = std::str::from_utf8(output).map_err(|_| unsupported())?;
    let mut fields = text.split('\0');
    let tree = fields.next().ok_or_else(unsupported)?.trim().to_owned();
    validate_id(&tree)?;
    let mut entries = BTreeMap::<String, Stages>::new();
    for record in fields {
        if record.is_empty() {
            continue;
        }
        let (metadata, path) = record.split_once('\t').ok_or_else(unsupported)?;
        let path = FilePath::parse(path)
            .map_err(|_| unsupported())
            .and_then(|parsed| if parsed.as_str() == path { Ok(path.to_owned()) } else { Err(unsupported()) })?;
        let parts: Vec<_> = metadata.split(' ').collect();
        if parts.len() != 3 || !matches!(parts[0], "100644" | "100755") {
            return Err(unsupported());
        }
        validate_id(parts[1])?;
        let stages = entries.entry(path).or_default();
        let target = match parts[2] {
            "1" => continue,
            "2" => &mut stages.ours,
            "3" => &mut stages.theirs,
            _ => return Err(unsupported()),
        };
        if target.replace((parts[0].into(), parts[1].into())).is_some() {
            return Err(unsupported());
        }
        if entries.len() > MAX_FILES {
            return Err(unsupported());
        }
    }
    Ok((tree, entries))
}

pub(super) async fn read_files(
    edit: &EditWorkspace,
    tree: &str,
    head: &str,
    base: &str,
    stages: BTreeMap<String, Stages>,
) -> Result<(Vec<ConflictFile>, BTreeMap<String, Mode>), EditError> {
    let (root, tree, head, base) = (edit.scratch.path().to_owned(), tree.to_owned(), head.to_owned(), base.to_owned());
    tokio::task::spawn_blocking(move || {
        let repo = gix::open_opts(root, gix::open::Options::isolated().config_overrides(["core.useReplaceRefs=false"]))
            .map_err(GitError::backend)?;
        let merged = repo.find_tree(oid(&tree)?).map_err(GitError::backend)?;
        let ours = repo.find_commit(oid(&head)?).map_err(GitError::backend)?.tree().map_err(GitError::backend)?;
        let theirs = repo.find_commit(oid(&base)?).map_err(GitError::backend)?.tree().map_err(GitError::backend)?;
        let (mut files, mut modes, mut total) = (Vec::new(), BTreeMap::new(), 0);
        for (path, stages) in stages {
            let (Some((our_mode, our_id)), Some((their_mode, their_id))) = (stages.ours, stages.theirs) else {
                return Err(unsupported());
            };
            if our_mode != their_mode {
                return Err(unsupported());
            }
            let source = text_at(&repo, &ours, &path, Some(&our_id))?;
            let target = text_at(&repo, &theirs, &path, Some(&their_id))?;
            let text = text_at(&repo, &merged, &path, None)?;
            total += source.len() + target.len() + text.len();
            if total > MAX_TOTAL_BYTES {
                return Err(unsupported());
            }
            modes.insert(path.clone(), if our_mode == "100755" { Mode::Executable } else { Mode::File });
            files.push(ConflictFile { path, source, target, merged: text });
        }
        Ok((files, modes))
    })
    .await
    .map_err(GitError::from)?
}

fn oid(id: &str) -> Result<gix::ObjectId, GitError> {
    gix::ObjectId::from_hex(id.as_bytes()).map_err(GitError::backend)
}

fn text_at(repo: &gix::Repository, tree: &gix::Tree<'_>, path: &str, expected: Option<&str>) -> Result<String, EditError> {
    let entry = tree.lookup_entry_by_path(path).map_err(GitError::backend)?.ok_or_else(unsupported)?;
    if !entry.mode().is_blob() || expected.is_some_and(|id| entry.object_id().to_hex() != id) {
        return Err(unsupported());
    }
    let header = repo.find_header(entry.object_id()).map_err(GitError::backend)?;
    if header.size() > MAX_FILE_BYTES as u64 {
        return Err(unsupported());
    }
    let object = repo.find_object(entry.object_id()).map_err(GitError::backend)?;
    let text = std::str::from_utf8(&object.data).map_err(|_| unsupported())?;
    if text.contains('\0') || rbit_core::lfs::LfsPointer::parse(text.as_bytes()).is_some() {
        return Err(unsupported());
    }
    Ok(text.to_owned())
}
