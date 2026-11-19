//! Compute immutable merge objects before either guarded or ordinary publication.
use super::{Git, MergeEnvironment, MergeError, MergeSpec, Merged, Signature, hand_over, rebase};
use crate::{
    overlay::{MERGES_DIR, Overlay},
    pull_refs::validate_id,
};
use rbit_core::{issue::MergeStrategy, pull::CommitIdentity};

/// The merge of `spec`, handed to ref `target` of `spec.repo` if it still is `expected`.
pub(super) async fn build(
    env: &MergeEnvironment,
    spec: &MergeSpec<'_>,
    target: &str,
    expected: &str,
    reason: &str,
) -> Result<Merged, MergeError> {
    let (scratch, commit) = prepare(env, spec).await?;
    let git = Git { binary: &env.git_binary, dir: scratch.path() };

    // Hand the result to the base repository: objects first, then the atomic ref update.
    let base = Git { binary: &env.git_binary, dir: spec.repo };
    hand_over(&git, &base, target, &commit, expected, reason).await?;
    drop(scratch);
    Ok(Merged { commit })
}

pub(super) async fn prepare(env: &MergeEnvironment, spec: &MergeSpec<'_>) -> Result<(Overlay, String), MergeError> {
    validate_id(spec.base_tip)?;
    validate_id(spec.head)?;
    let scratch = Overlay::create_in(&env.tmp_dir.join(MERGES_DIR), &[spec.repo]).await?;
    let git = Git { binary: &env.git_binary, dir: scratch.path() };

    if git.is_ancestor(spec.head, spec.base_tip).await? {
        return Err(MergeError::NothingToMerge);
    }
    let Some(merge_base) = git.merge_base(spec.base_tip, spec.head).await? else {
        return Err(MergeError::Unrelated);
    };

    let commit = match spec.strategy {
        MergeStrategy::Merge | MergeStrategy::Squash => {
            let tree = git.merge_tree(None, spec.base_tip, spec.head).await?.map_err(MergeError::Conflict)?;
            let (parents, author): (Vec<&str>, &CommitIdentity) = match spec.strategy {
                MergeStrategy::Merge => (vec![spec.base_tip, spec.head], spec.merger),
                _ => (vec![spec.base_tip], spec.author),
            };
            let author = Signature::new(author, spec.at);
            git.commit_tree(&tree, &parents, spec.message.as_bytes(), &author, &Signature::new(spec.merger, spec.at)).await?
        }
        MergeStrategy::Rebase => rebase(&git, spec, &merge_base).await?,
    };

    Ok((scratch, commit))
}
