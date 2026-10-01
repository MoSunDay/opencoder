use super::ReadOnlyAgentsFs;
use anyhow::{ensure, Result};
use nfsserve::{
    nfs::{nfs_fh3, nfsstat3, post_op_attr},
    vfs::NFSFileSystem,
    xdr::XDR,
};
use std::io::{Cursor, Read};

fn word(input: &mut Cursor<&[u8]>) -> Result<u32> {
    let mut bytes = [0u8; 4];
    input.read_exact(&mut bytes)?;
    Ok(u32::from_be_bytes(bytes))
}

fn put(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn skip_auth(input: &mut Cursor<&[u8]>) -> Result<()> {
    word(input)?;
    let size = word(input)? as u64;
    ensure!(size <= 4096, "RPC authentication exceeds limit");
    let end = input.position() + size.div_ceil(4) * 4;
    ensure!(
        end <= input.get_ref().len() as u64,
        "truncated RPC authentication"
    );
    input.set_position(end);
    Ok(())
}

pub(crate) async fn reply(
    fs: &ReadOnlyAgentsFs,
    request: &[u8],
    port: u16,
) -> Result<Option<Vec<u8>>> {
    let mut input = Cursor::new(request);
    let xid = word(&mut input)?;
    if word(&mut input)? != 0 || word(&mut input)? != 2 {
        return Ok(None);
    }
    let program = word(&mut input)?;
    let version = word(&mut input)?;
    let procedure = word(&mut input)?;
    if program != 100227 && !(program == 100000 && version == 2 && procedure == 3) {
        return Ok(None);
    }
    skip_auth(&mut input)?;
    skip_auth(&mut input)?;
    let mut output = Vec::new();
    if program == 100000 {
        let mapped_program = word(&mut input)?;
        let mapped_version = word(&mut input)?;
        let protocol = word(&mut input)?;
        word(&mut input)?;
        for value in [xid, 1, 0, 0, 0, 0] {
            put(&mut output, value);
        }
        put(
            &mut output,
            if matches!(mapped_program, 100003 | 100005 | 100227)
                && mapped_version == 3
                && protocol == 6
            {
                port as u32
            } else {
                0
            },
        );
        return Ok(Some(output));
    }
    for value in [
        xid,
        1,
        0,
        0,
        0,
        if version != 3 {
            2
        } else if procedure > 2 {
            3
        } else {
            0
        },
    ] {
        put(&mut output, value);
    }
    if version != 3 {
        put(&mut output, 3);
        put(&mut output, 3);
        return Ok(Some(output));
    }
    if procedure == 0 || procedure > 2 {
        return Ok(Some(output));
    }
    if procedure == 2 {
        nfsstat3::NFS3ERR_ROFS.serialize(&mut output)?;
        post_op_attr::Void.serialize(&mut output)?;
        return Ok(Some(output));
    }
    let mut handle = nfs_fh3::default();
    handle.deserialize(&mut input)?;
    let mask = word(&mut input)?;
    let attributes = match fs
        .fh_to_id(&handle)
        .and_then(|id| Ok((id, fs.local_acl_path(id)?)))
    {
        Ok((id, path)) => fs.getattr(id).await.map(|attributes| (path, attributes)),
        Err(error) => Err(error),
    };
    let (path, attributes) = match attributes {
        Ok(attributes) if mask & !15 == 0 => attributes,
        Ok(_) => {
            nfsstat3::NFS3ERR_INVAL.serialize(&mut output)?;
            post_op_attr::Void.serialize(&mut output)?;
            return Ok(Some(output));
        }
        Err(error) => {
            error.serialize(&mut output)?;
            post_op_attr::Void.serialize(&mut output)?;
            return Ok(Some(output));
        }
    };
    let acls = (|| -> Result<_> { Ok((read_acl(&path, false)?, read_acl(&path, true)?)) })();
    let (access, default) = match acls {
        Ok(acls) => acls,
        Err(_) => {
            nfsstat3::NFS3ERR_IO.serialize(&mut output)?;
            post_op_attr::Void.serialize(&mut output)?;
            return Ok(Some(output));
        }
    };
    let access = access.unwrap_or_else(|| {
        vec![
            (1, attributes.uid, (attributes.mode >> 6) & 7),
            (4, attributes.gid, (attributes.mode >> 3) & 7),
            (32, 0, attributes.mode & 7),
        ]
    });
    let default = default.unwrap_or_default();
    nfsstat3::NFS3_OK.serialize(&mut output)?;
    post_op_attr::attributes(attributes).serialize(&mut output)?;
    put(&mut output, mask);
    encode(&mut output, &access, mask & 1 != 0, mask & 3 != 0, false);
    encode(&mut output, &default, mask & 4 != 0, mask & 12 != 0, true);
    Ok(Some(output))
}

fn encode(
    output: &mut Vec<u8>,
    entries: &[(u32, u32, u32)],
    values: bool,
    count: bool,
    default: bool,
) {
    let mut entries = entries.to_vec();
    if entries.len() == 3 {
        let group = entries.iter().find(|entry| entry.0 == 4).unwrap().2;
        entries.insert(2, (16, 0, group));
    }
    put(
        output,
        if count || values {
            entries.len() as u32
        } else {
            0
        },
    );
    put(output, if values { entries.len() as u32 } else { 0 });
    if values {
        for (kind, id, permission) in entries {
            put(output, kind | if default { 0x1000 } else { 0 });
            put(output, id);
            put(output, permission);
        }
    }
}

#[cfg(target_os = "linux")]
fn read_acl(path: &std::path::Path, default: bool) -> Result<Option<Vec<(u32, u32, u32)>>> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    let name = if default {
        c"system.posix_acl_default"
    } else {
        c"system.posix_acl_access"
    };
    let mut buffer = vec![0u8; 65536];
    let count = unsafe {
        libc::lgetxattr(
            path.as_ptr(),
            name.as_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
        )
    };
    if count < 0 {
        let error = std::io::Error::last_os_error();
        if matches!(error.raw_os_error(), Some(libc::ENODATA | libc::EOPNOTSUPP)) {
            return Ok(None);
        }
        return Err(error.into());
    }
    buffer.truncate(count as usize);
    ensure!(
        buffer.len() >= 4 && buffer[..4] == [2, 0, 0, 0] && (buffer.len() - 4).is_multiple_of(8),
        "invalid POSIX ACL attribute"
    );
    Ok(Some(
        buffer[4..]
            .as_chunks::<8>()
            .0
            .iter()
            .map(|entry| {
                (
                    u16::from_le_bytes(entry[..2].try_into().unwrap()) as u32,
                    u32::from_le_bytes(entry[4..].try_into().unwrap()),
                    u16::from_le_bytes(entry[2..4].try_into().unwrap()) as u32,
                )
            })
            .collect(),
    ))
}

#[cfg(not(target_os = "linux"))]
fn read_acl(_: &std::path::Path, _: bool) -> Result<Option<Vec<(u32, u32, u32)>>> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_acl_has_the_required_mask_and_matching_counts() {
        let mut output = vec![];
        encode(
            &mut output,
            &[(1, 0, 6), (4, 0, 4), (32, 0, 4)],
            true,
            false,
            false,
        );
        let mut input = Cursor::new(output.as_slice());
        assert_eq!(word(&mut input).unwrap(), 4);
        assert_eq!(word(&mut input).unwrap(), 4);
        for (kind, permission) in [(1, 6), (4, 4), (16, 4), (32, 4)] {
            assert_eq!(word(&mut input).unwrap(), kind);
            word(&mut input).unwrap();
            assert_eq!(word(&mut input).unwrap(), permission);
        }
    }

    #[test]
    fn default_count_only_reply_does_not_include_entries() {
        let mut output = vec![];
        encode(
            &mut output,
            &[(1, 0, 7), (4, 0, 5), (32, 0, 5)],
            false,
            true,
            true,
        );
        assert_eq!(output, [0, 0, 0, 4, 0, 0, 0, 0]);
    }
}
