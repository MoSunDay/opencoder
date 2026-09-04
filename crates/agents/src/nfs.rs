//! Read-only NFSv3 [`NFSFileSystem`] over the agents root.
//!
//! The export is a *plain directory tree* (`<root>/prompts|skills|tools|
//! memory/<name>/v{n}/…` plus per-agent `meta.json` dirs), so the VFS maps
//! NFS objects 1:1 onto real paths:
//!
//! - **fileid ↔ path**: `fileid3` is a stable FNV-1a hash of the canonical
//!   *relative* path; the opaque NFS file handle carries the relative path
//!   bytes themselves (`id_to_fh`/`fh_to_id` overrides). A tiny id→path
//!   registry is the only state — integration glue required because the
//!   `NFSFileSystem` trait exchanges opaque `u64` ids in both directions.
//! - **traversal rejection at the FS layer**: handle/lookup names must be a
//!   single canonical UTF-8 component — `..`, `.`, absolute paths, embedded
//!   separators, NUL and non-UTF8 are rejected before any `join`.
//! - **read-only**: every mutating op returns [`nfsstat3::NFS3ERR_ROFS`],
//!   whatever the trait-level capability says; missing paths are
//!   `NFS3ERR_NOENT`.
//!
//! Pure-functional style: free functions over a plain struct; the trait
//! impl below is the external integration surface.

use std::collections::HashMap;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use async_trait::async_trait;
use nfsserve::fs_util::metadata_to_fattr3;
use nfsserve::nfs::{fattr3, fileid3, filename3, nfs_fh3, nfspath3, nfsstat3, nfsstring, sattr3};
use nfsserve::vfs::{DirEntry, NFSFileSystem, ReadDirResult, VFSCapabilities};

/// Payload budget for the relative path inside a file handle. NFSv3 caps
/// handles at 64 bytes (`FHSIZE3`); we stay under it so real clients never
/// see an oversized handle.
const MAX_FH_PATH: usize = 60;

/// Upper bound on a single READ reply (matches nfsserve's advertised
/// `rtmax`/`rtpref` of 1 MiB) so a hostile `count` can't force a huge alloc.
const MAX_READ: usize = 1024 * 1024;

/// The exported tree: absolute root + the id→path glue registry.
pub struct ReadOnlyAgentsFs {
    root: PathBuf,
    ids: RwLock<HashMap<fileid3, PathBuf>>,
}

/// Build an exporter for `root`. The root is not validated here — spawn
/// (`serve::spawn_nfs_server`) rejects missing/non-dir roots.
pub fn agents_fs(root: PathBuf) -> ReadOnlyAgentsFs {
    ReadOnlyAgentsFs {
        root,
        ids: RwLock::new(HashMap::new()),
    }
}

/// FNV-1a 64 over the canonical relative path. `0` is a reserved fileid,
/// so it is mapped to `1` (FNV never yields 0 in practice anyway).
fn path_id(rel: &Path) -> fileid3 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in rel.to_string_lossy().as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    if h == 0 {
        1
    } else {
        h
    }
}

/// Canonical relative-path bytes. The root (`""`) is encoded as `"/"`:
/// a zero-length opaque handle is illegal for real clients (the Linux
/// kernel fails the mount with `EBADHANDLE`=521). `rel_from_handle`
/// decodes both `"/"` and `""` back to the root.
fn rel_bytes(rel: &Path) -> Vec<u8> {
    let s = rel.to_string_lossy();
    if s.is_empty() {
        b"/".to_vec()
    } else {
        s.into_owned().into_bytes()
    }
}

/// Decode opaque file-handle payload into a canonical relative path.
/// The root's handle is exactly `"/"` — a zero-length opaque handle is
/// illegal for real clients (the Linux kernel fails the mount with
/// `EBADHANDLE`=521). Rejects oversized, non-UTF8, NUL-containing,
/// absolute and non-canonical (`.`/`..`/empty component) payloads —
/// traversal dies here.
fn rel_from_handle(data: &[u8]) -> Result<PathBuf, nfsstat3> {
    let bad = nfsstat3::NFS3ERR_BADHANDLE;
    if data == b"/" {
        return Ok(PathBuf::new());
    }
    if data.len() > MAX_FH_PATH || data.contains(&0) {
        return Err(bad);
    }
    let s = std::str::from_utf8(data).map_err(|_| bad)?;
    if s.is_empty() {
        return Ok(PathBuf::new());
    }
    if s.starts_with('/') {
        return Err(bad);
    }
    if s.split('/').any(|c| c.is_empty() || c == "." || c == "..") {
        return Err(bad);
    }
    Ok(PathBuf::from(s))
}

/// Validate a LOOKUP filename: one canonical UTF-8 component. `..`,
/// separators, NUL, non-UTF8 and empty names are `NFS3ERR_NOENT` — rejected
/// before any filesystem access.
fn lookup_component(name: &filename3) -> Result<PathBuf, nfsstat3> {
    let noent = nfsstat3::NFS3ERR_NOENT;
    let s = std::str::from_utf8(name.as_ref()).map_err(|_| noent)?;
    if s.is_empty() || s == "." || s == ".." || s.contains('/') || s.contains('\0') {
        return Err(noent);
    }
    Ok(PathBuf::from(s))
}

/// Register (or refresh) `rel` in the id→path registry, returning its id.
fn register(ids: &RwLock<HashMap<fileid3, PathBuf>>, rel: &Path) -> fileid3 {
    let id = path_id(rel);
    ids.write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id, rel.to_path_buf());
    id
}

/// Reverse lookup: id → relative path. Unknown ids are stale handles.
fn path_of(ids: &RwLock<HashMap<fileid3, PathBuf>>, id: fileid3) -> Result<PathBuf, nfsstat3> {
    ids.read()
        .unwrap_or_else(|e| e.into_inner())
        .get(&id)
        .cloned()
        .ok_or(nfsstat3::NFS3ERR_STALE)
}

/// `symlink_metadata` of `root/rel` — missing ⇒ `NFS3ERR_NOENT`. Symlink
/// metadata (not followed) so dangling links still getattr/readlink.
fn stat(root: &Path, rel: &Path) -> Result<std::fs::Metadata, nfsstat3> {
    std::fs::symlink_metadata(root.join(rel)).map_err(|_| nfsstat3::NFS3ERR_NOENT)
}

#[async_trait]
impl NFSFileSystem for ReadOnlyAgentsFs {
    fn capabilities(&self) -> VFSCapabilities {
        VFSCapabilities::ReadOnly
    }

    fn root_dir(&self) -> fileid3 {
        register(&self.ids, Path::new(""))
    }

    fn id_to_fh(&self, id: fileid3) -> nfs_fh3 {
        match path_of(&self.ids, id) {
            Ok(rel) => nfs_fh3 {
                data: rel_bytes(&rel),
            },
            // Unknown id: emit a handle that always fails rel_from_handle
            // (NUL is rejected) instead of aliasing the root.
            Err(_) => nfs_fh3 { data: vec![0] },
        }
    }

    fn fh_to_id(&self, fh: &nfs_fh3) -> Result<fileid3, nfsstat3> {
        Ok(register(&self.ids, &rel_from_handle(&fh.data)?))
    }

    async fn lookup(&self, dirid: fileid3, filename: &filename3) -> Result<fileid3, nfsstat3> {
        let dir = path_of(&self.ids, dirid)?;
        stat(&self.root, &dir)?;
        let rel = dir.join(lookup_component(filename)?);
        if rel.as_os_str().len() > MAX_FH_PATH {
            return Err(nfsstat3::NFS3ERR_NOENT);
        }
        stat(&self.root, &rel)?;
        Ok(register(&self.ids, &rel))
    }

    async fn getattr(&self, id: fileid3) -> Result<fattr3, nfsstat3> {
        let rel = path_of(&self.ids, id)?;
        Ok(metadata_to_fattr3(id, &stat(&self.root, &rel)?))
    }

    async fn setattr(&self, _id: fileid3, _attr: sattr3) -> Result<fattr3, nfsstat3> {
        Err(nfsstat3::NFS3ERR_ROFS)
    }

    async fn read(
        &self,
        id: fileid3,
        offset: u64,
        count: u32,
    ) -> Result<(Vec<u8>, bool), nfsstat3> {
        let rel = path_of(&self.ids, id)?;
        let meta = stat(&self.root, &rel)?;
        if meta.is_dir() {
            return Err(nfsstat3::NFS3ERR_ISDIR);
        }
        if !meta.is_file() {
            return Err(nfsstat3::NFS3ERR_ACCES);
        }
        let mut f = std::fs::File::open(self.root.join(&rel)).map_err(|_| nfsstat3::NFS3ERR_IO)?;
        f.seek(SeekFrom::Start(offset))
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        let mut buf = vec![0u8; (count as usize).min(MAX_READ)];
        let n = f.read(&mut buf).map_err(|_| nfsstat3::NFS3ERR_IO)?;
        buf.truncate(n);
        let eof = offset.saturating_add(n as u64) >= meta.len();
        Ok((buf, eof))
    }

    async fn write(&self, _id: fileid3, _offset: u64, _data: &[u8]) -> Result<fattr3, nfsstat3> {
        Err(nfsstat3::NFS3ERR_ROFS)
    }

    async fn create(
        &self,
        _dirid: fileid3,
        _filename: &filename3,
        _attr: sattr3,
    ) -> Result<(fileid3, fattr3), nfsstat3> {
        Err(nfsstat3::NFS3ERR_ROFS)
    }

    async fn create_exclusive(
        &self,
        _dirid: fileid3,
        _filename: &filename3,
    ) -> Result<fileid3, nfsstat3> {
        Err(nfsstat3::NFS3ERR_ROFS)
    }

    async fn mkdir(
        &self,
        _dirid: fileid3,
        _filename: &filename3,
    ) -> Result<(fileid3, fattr3), nfsstat3> {
        Err(nfsstat3::NFS3ERR_ROFS)
    }

    async fn remove(&self, _dirid: fileid3, _filename: &filename3) -> Result<(), nfsstat3> {
        Err(nfsstat3::NFS3ERR_ROFS)
    }

    async fn rename(
        &self,
        _from_dirid: fileid3,
        _from_filename: &filename3,
        _to_dirid: fileid3,
        _to_filename: &filename3,
    ) -> Result<(), nfsstat3> {
        Err(nfsstat3::NFS3ERR_ROFS)
    }

    async fn symlink(
        &self,
        _dirid: fileid3,
        _linkname: &filename3,
        _symlink: &nfspath3,
        _attr: &sattr3,
    ) -> Result<(fileid3, fattr3), nfsstat3> {
        Err(nfsstat3::NFS3ERR_ROFS)
    }

    async fn readlink(&self, id: fileid3) -> Result<nfspath3, nfsstat3> {
        let rel = path_of(&self.ids, id)?;
        if !stat(&self.root, &rel)?.is_symlink() {
            return Err(nfsstat3::NFS3ERR_INVAL);
        }
        let target = std::fs::read_link(self.root.join(&rel)).map_err(|_| nfsstat3::NFS3ERR_IO)?;
        Ok(nfsstring(rel_bytes(&target)))
    }

    async fn readdir(
        &self,
        dirid: fileid3,
        start_after: fileid3,
        max_entries: usize,
    ) -> Result<ReadDirResult, nfsstat3> {
        let rel = path_of(&self.ids, dirid)?;
        if !stat(&self.root, &rel)?.is_dir() {
            return Err(nfsstat3::NFS3ERR_NOTDIR);
        }
        // Deterministic order: sort by name so readdir cookies stay stable.
        let mut names: Vec<(PathBuf, fileid3)> = std::fs::read_dir(self.root.join(&rel))
            .map_err(|_| nfsstat3::NFS3ERR_IO)?
            .filter_map(|e| e.ok())
            .map(|e| PathBuf::from(e.file_name().to_string_lossy().into_owned()))
            .filter(|n| rel.join(n).as_os_str().len() <= MAX_FH_PATH)
            .map(|n| {
                let id = path_id(&rel.join(&n));
                (n, id)
            })
            .collect();
        names.sort_by(|a, b| a.0.cmp(&b.0));

        // Unknown cookie (or 0): restart from the top rather than
        // erroring — clients resync from a fresh listing.
        let skip = names
            .iter()
            .position(|(_, id)| *id == start_after)
            .map_or(0, |pos| pos + 1);
        let mut entries = Vec::new();
        for (name, id) in &names[skip.min(names.len())..] {
            if entries.len() >= max_entries.max(1) {
                break;
            }
            let child = rel.join(name);
            let attr = match stat(&self.root, &child) {
                Ok(meta) => metadata_to_fattr3(*id, &meta),
                Err(_) => continue,
            };
            register(&self.ids, &child);
            entries.push(DirEntry {
                fileid: *id,
                name: nfsstring(rel_bytes(name)),
                attr,
            });
        }
        let end = skip.min(names.len()) + entries.len() >= names.len();
        Ok(ReadDirResult { entries, end })
    }
}

#[cfg(test)]
mod tests;
