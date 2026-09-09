//! Bounded NFSv3 handles without limiting the exported resource path length.
use super::{
    fileid3, nfsstat3, path_id, rel_bytes, rel_from_handle, HashMap, Path, PathBuf, RwLock,
    MAX_FH_PATH,
};
use sha2::{Digest, Sha256};

const PREFIX: &[u8] = b"\0OCNF\x01";

pub(super) fn encode(path: &Path) -> Vec<u8> {
    let bytes = rel_bytes(path);
    if bytes.len() <= MAX_FH_PATH {
        return bytes;
    }
    let digest = Sha256::digest(&bytes);
    [PREFIX, &path_id(path).to_be_bytes(), digest.as_ref()].concat()
}

pub(super) fn resolve(
    root: &Path,
    ids: &RwLock<HashMap<fileid3, PathBuf>>,
    handle: &[u8],
) -> Result<PathBuf, nfsstat3> {
    if !handle.starts_with(PREFIX) {
        return rel_from_handle(handle);
    }
    if handle.len() != PREFIX.len() + 8 + 32 {
        return Err(nfsstat3::NFS3ERR_BADHANDLE);
    }
    let id = fileid3::from_be_bytes(handle[PREFIX.len()..PREFIX.len() + 8].try_into().unwrap());
    if let Some(path) = ids
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(&id)
        .filter(|path| encode(path) == handle)
    {
        return Ok(path.clone());
    }
    // A client may retain handles across export restart. Recover from the tree
    // instead of requiring a writable database or invalidating every old mount.
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        let directory =
            std::fs::read_dir(root.join(&relative)).map_err(|_| nfsstat3::NFS3ERR_IO)?;
        for entry in directory {
            let entry = entry.map_err(|_| nfsstat3::NFS3ERR_IO)?;
            let name = entry.file_name();
            if name.to_str().is_none() {
                continue;
            }
            let path = relative.join(name);
            if encode(&path) == handle {
                return Ok(path);
            }
            // Never follow links while resolving an opaque handle.
            if entry
                .file_type()
                .map_err(|_| nfsstat3::NFS3ERR_IO)?
                .is_dir()
            {
                pending.push(path);
            }
        }
    }
    Err(nfsstat3::NFS3ERR_STALE)
}
