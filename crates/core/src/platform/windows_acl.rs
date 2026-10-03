//! Protected ACLs are supplied at creation, before private bytes are written.
use std::{
    ffi::c_void,
    fs::File,
    io,
    mem::size_of,
    os::windows::{ffi::OsStrExt, io::FromRawHandle},
    path::Path,
    ptr,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, LocalFree, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE},
    Security::{
        Authorization::*, GetSecurityDescriptorControl, GetTokenInformation, TokenUser,
        ACCESS_ALLOWED_ACE, ACL, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
        SECURITY_ATTRIBUTES, SE_DACL_PROTECTED, TOKEN_QUERY, TOKEN_USER,
    },
    Storage::FileSystem::{
        CreateDirectoryW, CreateFileW, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE,
        FILE_SHARE_READ,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

struct LocalMemory(*mut c_void);
impl Drop for LocalMemory {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}

pub(super) fn wide(path: &Path) -> io::Result<Vec<u16>> {
    let path = std::path::absolute(path)?;
    let mut wide: Vec<_> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path contains NUL",
        ));
    }
    if !wide.starts_with(&[92, 92, 63, 92]) {
        if wide.starts_with(&[92, 92]) {
            wide.splice(..2, "\\\\?\\UNC\\".encode_utf16());
        } else {
            wide.splice(..0, "\\\\?\\".encode_utf16());
        }
    }
    wide.push(0);
    Ok(wide)
}

fn sid_string(sid: *mut c_void) -> io::Result<String> {
    let mut value = ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &mut value) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let _memory = LocalMemory(value.cast());
    let mut length = 0;
    while unsafe { *value.add(length) } != 0 {
        length += 1;
    }
    Ok(String::from_utf16_lossy(unsafe {
        std::slice::from_raw_parts(value, length)
    }))
}

fn current_sid() -> io::Result<String> {
    let mut token = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let result = (|| {
        let mut bytes = 0;
        unsafe {
            GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut bytes);
        }
        let mut buffer = vec![0u64; (bytes as usize).div_ceil(8)];
        if unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                bytes,
                &mut bytes,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
        sid_string(user.User.Sid)
    })();
    unsafe {
        CloseHandle(token);
    }
    result
}

fn descriptor() -> io::Result<LocalMemory> {
    let sid = current_sid()?;
    let text: Vec<u16> = format!("O:{sid}D:P(A;OICI;FA;;;{sid})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut descriptor = ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text.as_ptr(),
            1,
            &mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(LocalMemory(descriptor))
}

fn attributes(descriptor: &LocalMemory) -> SECURITY_ATTRIBUTES {
    SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    }
}

pub(super) fn create_file(path: &Path) -> io::Result<File> {
    let descriptor = descriptor()?;
    let path = wide(path)?;
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_DELETE,
            &attributes(&descriptor),
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_handle(handle) })
    }
}

pub(super) fn create_directory(path: &Path) -> io::Result<()> {
    let descriptor = descriptor()?;
    let path = wide(path)?;
    if unsafe { CreateDirectoryW(path.as_ptr(), &attributes(&descriptor)) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub(super) fn private_access(path: &Path) -> io::Result<bool> {
    let metadata = std::fs::symlink_metadata(path)?;
    if super::fs::is_link(&metadata) {
        return Ok(false);
    }
    let path = wide(path)?;
    let mut owner = ptr::null_mut();
    let mut dacl: *mut ACL = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    let code = unsafe {
        GetNamedSecurityInfoW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if code != 0 {
        return Err(io::Error::from_raw_os_error(code as i32));
    }
    let _memory = LocalMemory(descriptor);
    let mut control = 0;
    let mut revision = 0;
    if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let current = current_sid()?;
    if dacl.is_null() || control & SE_DACL_PROTECTED == 0 || sid_string(owner)? != current {
        return Ok(false);
    }
    let mut owner_allowed = false;
    for index in 0..unsafe { (*dacl).AceCount } as u32 {
        let mut ace = ptr::null_mut();
        if unsafe { windows_sys::Win32::Security::GetAce(dacl, index, &mut ace) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let header = unsafe { &*ace.cast::<windows_sys::Win32::Security::ACE_HEADER>() };
        // Only explicit allow entries for the owner, SYSTEM and administrators.
        if header.AceType != 0 || header.AceFlags & 0x10 != 0 {
            return Ok(false);
        }
        let ace = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
        let sid = sid_string((&ace.SidStart as *const u32).cast_mut().cast())?;
        if sid == current {
            owner_allowed = ace.Mask & 0x1f01ff == 0x1f01ff;
        } else if sid != "S-1-5-18" && sid != "S-1-5-32-544" {
            return Ok(false);
        }
    }
    Ok(owner_allowed)
}
