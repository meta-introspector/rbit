//! Bounded extraction of exact signed payloads from immutable commit and tag objects.
use crate::{GitError, RepositoryReader};

/// The exact byte sequences passed to a verifier; never a reconstructed commit.
#[derive(Debug)]
pub struct SignedObject {
    pub object_id: String,
    pub kind: &'static str,
    pub signature: Option<Vec<u8>>,
    pub payload: Vec<u8>,
}

impl RepositoryReader {
    /// Read an exact full object id, refusing blobs, trees and oversized objects.
    pub async fn signed_object(&self, object_id: String) -> Result<SignedObject, GitError> {
        self.with_repo(move |repo| {
            if object_id.len() != repo.object_hash().len_in_hex() {
                return Err(GitError::NotFound("object"));
            }
            let id = gix::ObjectId::from_hex(object_id.as_bytes()).map_err(|_| GitError::NotFound("object"))?;
            let header = repo.find_header(id).map_err(|_| GitError::NotFound("object"))?;
            if header.size() > 1024 * 1024 {
                return Err(GitError::NotFound("bounded signed object"));
            }
            let object = repo.find_object(id).map_err(GitError::backend)?;
            let (kind, signature, payload) = match object.kind {
                gix::objs::Kind::Commit => {
                    gix::objs::CommitRef::from_bytes(&object.data, id.kind()).map_err(GitError::backend)?;
                    let field = format!("{} ", gix::objs::commit::signature_field_name(id.kind()));
                    if object
                        .data
                        .split(|byte| *byte == b'\n')
                        .take_while(|line| !line.is_empty())
                        .filter(|line| line.starts_with(field.as_bytes()))
                        .count()
                        > 1
                    {
                        return Err(GitError::NotFound("unambiguous signed commit"));
                    }
                    let extracted = gix::objs::CommitRefIter::signature(&object.data, id.kind()).map_err(GitError::backend)?;
                    let (sig, payload) =
                        extracted.map_or_else(|| (None, Vec::new()), |(sig, payload)| (Some(sig.to_vec()), payload.to_bstring().to_vec()));
                    ("commit", sig, payload)
                }
                gix::objs::Kind::Tag => {
                    gix::objs::TagRef::from_bytes(&object.data, id.kind()).map_err(GitError::backend)?;
                    let extracted = gix::objs::TagRefIter::signature(&object.data);
                    let (sig, payload) = extracted
                        .map_or_else(|| (None, Vec::new()), |(sig, payload)| (Some(sig.data.to_vec()), payload.to_bstring().to_vec()));
                    ("tag", sig, payload)
                }
                _ => return Err(GitError::NotFound("commit or annotated tag")),
            };
            Ok(SignedObject { object_id: id.to_string(), kind, signature, payload })
        })
        .await
    }
}

impl RepositoryReader {
    /// Resolve only an exact tag ref, preserving annotated-tag objects rather than peeling.
    pub async fn signature_tag_id(&self, name: String) -> Result<String, GitError> {
        if name.is_empty() || name.len() > 1024 {
            return Err(GitError::NotFound("tag"));
        }
        self.with_repo(move |repo| {
            let reference = repo.find_reference(&format!("refs/tags/{name}")).map_err(|_| GitError::NotFound("tag"))?;
            Ok(reference.id().to_string())
        })
        .await
    }
}
