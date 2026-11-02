//! Per-user local pipe naming, ACLs and peer identity checks.
use std::{
    ffi::c_void,
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::{Path, PathBuf},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{HANDLE, LocalFree},
    Security::{
        Authorization::{ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW},
        GetTokenInformation, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    System::{
        Pipes::{GetNamedPipeClientProcessId, GetNamedPipeServerProcessId},
        Threading::{GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION},
    },
};

fn checked(ok: i32) -> io::Result<()> {
    if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

fn process_sid(process: HANDLE) -> io::Result<String> {
    // All buffers remain live and aligned through the native calls; owned handles are closed on return.
    unsafe {
        let mut token = ptr::null_mut();
        checked(OpenProcessToken(process, TOKEN_QUERY, &mut token))?;
        let token = OwnedHandle::from_raw_handle(token);
        let mut size = 0;
        GetTokenInformation(token.as_raw_handle(), TokenUser, ptr::null_mut(), 0, &mut size);
        if size == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buffer = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
        checked(GetTokenInformation(token.as_raw_handle(), TokenUser, buffer.as_mut_ptr().cast(), size, &mut size))?;
        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        let mut sid = ptr::null_mut();
        checked(ConvertSidToStringSidW(user.User.Sid, &mut sid))?;
        let mut len = 0;
        while *sid.add(len) != 0 {
            len += 1;
        }
        let result = String::from_utf16_lossy(std::slice::from_raw_parts(sid, len));
        LocalFree(sid.cast());
        Ok(result)
    }
}

pub fn current_sid() -> io::Result<String> {
    // The current-process pseudo-handle is valid and must not be closed.
    process_sid(unsafe { GetCurrentProcess() })
}

pub fn pipe_path(name: &str) -> io::Result<PathBuf> {
    if name.is_empty() || name.len() > 64 || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Pipe suffix must contain 1-64 ASCII letters, digits or hyphens"));
    }
    Ok(PathBuf::from(format!(r"\\.\pipe\SharedAI-{}-{name}", current_sid()?)))
}

pub fn validate_path(path: &Path) -> io::Result<()> {
    let text = path.to_str().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid pipe path"))?;
    let prefix = format!(r"\\.\pipe\SharedAI-{}-", current_sid()?);
    let name = text
        .strip_prefix(&prefix)
        .ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied, "Pipe must use the current user's local SharedAI namespace"))?;
    if pipe_path(name)? != path {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid pipe path"));
    }
    Ok(())
}

pub fn verify_peer(pipe: &impl AsRawHandle, server: bool) -> io::Result<()> {
    // The connected pipe identifies the peer process; query-only access never elevates privileges.
    unsafe {
        let mut pid = 0;
        checked(if server {
            GetNamedPipeServerProcessId(pipe.as_raw_handle(), &mut pid)
        } else {
            GetNamedPipeClientProcessId(pipe.as_raw_handle(), &mut pid)
        })?;
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return Err(io::Error::last_os_error());
        }
        let process = OwnedHandle::from_raw_handle(process);
        if process_sid(process.as_raw_handle())? != current_sid()? {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Foreign pipe peer"));
        }
    }
    Ok(())
}

pub fn create_pipe(path: &Path, first: bool) -> io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    validate_path(path)?;
    let sddl: Vec<u16> = format!("D:P(A;;GA;;;{})", current_sid()?).encode_utf16().chain(Some(0)).collect();
    // The protected DACL grants only this user access; descriptor lives until pipe creation returns.
    unsafe {
        let mut descriptor = ptr::null_mut();
        checked(ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), 1, &mut descriptor, ptr::null_mut()))?;
        let mut attributes =
            SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: descriptor, bInheritHandle: 0 };
        let result = tokio::net::windows::named_pipe::ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .max_instances(33)
            .in_buffer_size(8192)
            .out_buffer_size(8192)
            .create_with_security_attributes_raw(path, (&mut attributes as *mut SECURITY_ATTRIBUTES).cast::<c_void>());
        LocalFree(descriptor);
        result
    }
}

/// Create a private data directory with inheritable user-only access; never chmod an existing path.
pub fn create_private_directory(path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    if name[..name.len() - 1].contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "NUL in data path"));
    }
    let sddl: Vec<u16> = format!("D:P(A;OICI;GA;;;{})", current_sid()?).encode_utf16().chain(Some(0)).collect();
    // Native allocations live through CreateDirectoryW and are released on every return path.
    let result = unsafe {
        let mut descriptor = ptr::null_mut();
        checked(ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), 1, &mut descriptor, ptr::null_mut()))?;
        let attributes =
            SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: descriptor, bInheritHandle: 0 };
        let result = checked(windows_sys::Win32::Storage::FileSystem::CreateDirectoryW(name.as_ptr(), &attributes));
        LocalFree(descriptor);
        result
    };
    match result {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
        Err(err) => return Err(err),
    }
    validate_private_storage(path)
}

/// Reject reparse points and non-user grants on storage; also reject hostile ancestor deletion rights.
pub fn validate_private_storage(path: &Path) -> io::Result<()> {
    use std::os::windows::{ffi::OsStrExt, fs::MetadataExt};
    use windows_sys::Win32::Security::{
        ACCESS_ALLOWED_ACE, ACE_HEADER,
        Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT},
        DACL_SECURITY_INFORMATION, GetAce, OWNER_SECURITY_INFORMATION,
    };
    let user = current_sid()?;
    for (index, candidate) in path.ancestors().enumerate() {
        if candidate.as_os_str().is_empty() {
            continue;
        }
        if std::fs::symlink_metadata(candidate)?.file_attributes() & 0x400 != 0 {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Reparse point in registry path"));
        }
        let name: Vec<u16> = candidate.as_os_str().encode_wide().chain(Some(0)).collect();
        if name[..name.len() - 1].contains(&0) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "NUL in data path"));
        }
        // GetNamedSecurityInfo supplies a validated, LocalFree-owned descriptor; ACE pointers borrow it.
        unsafe {
            let mut owner = ptr::null_mut();
            let mut dacl = ptr::null_mut();
            let mut descriptor = ptr::null_mut();
            let status = GetNamedSecurityInfoW(
                name.as_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                ptr::null_mut(),
                &mut dacl,
                ptr::null_mut(),
                &mut descriptor,
            );
            if status != 0 {
                return Err(io::Error::from_raw_os_error(status as i32));
            }
            let result = (|| {
                let sid_text = |sid| -> io::Result<String> {
                    let mut text = ptr::null_mut();
                    checked(ConvertSidToStringSidW(sid, &mut text))?;
                    let mut len = 0;
                    while *text.add(len) != 0 {
                        len += 1;
                    }
                    let value = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
                    LocalFree(text.cast());
                    Ok(value)
                };
                let owner = sid_text(owner)?;
                let trusted_owner = owner == user
                    || owner == "S-1-5-18"
                    || owner == "S-1-5-32-544"
                    || owner == "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";
                if index == 0 && owner != user || !trusted_owner {
                    return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Foreign registry owner"));
                }
                if dacl.is_null() {
                    return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Unrestricted registry ACL"));
                }
                for item in 0..u32::from((*dacl).AceCount) {
                    let mut ace = ptr::null_mut();
                    checked(GetAce(dacl, item, &mut ace))?;
                    let header = &*ace.cast::<ACE_HEADER>();
                    if header.AceFlags & 0x08 != 0 {
                        continue;
                    } // INHERIT_ONLY_ACE does not apply here.
                    if header.AceType == 1 {
                        continue;
                    } // A deny ACE cannot grant access.
                    if header.AceType != 0 {
                        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Unsupported registry ACL entry"));
                    }
                    let allowed = &*ace.cast::<ACCESS_ALLOWED_ACE>();
                    let sid = sid_text((&allowed.SidStart as *const u32).cast_mut().cast())?;
                    if sid == user {
                        continue;
                    }
                    let trusted_admin =
                        sid == "S-1-5-18" || sid == "S-1-5-32-544" || sid == "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";
                    // DELETE, WRITE_DAC, WRITE_OWNER, FILE_DELETE_CHILD, GENERIC_ALL can replace an ancestor.
                    if index == 0 || !trusted_admin && allowed.Mask & 0x100d0040 != 0 {
                        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Unsafe registry ACL grant"));
                    }
                }
                Ok(())
            })();
            LocalFree(descriptor);
            result?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn private_pipe_namespace_and_single_instance() {
        let path = pipe_path(&format!("unit-{}", std::process::id())).unwrap();
        validate_path(&path).unwrap();
        assert!(pipe_path("../bad").is_err());
        assert!(validate_path(Path::new(r"\\remote\pipe\SharedAI")).is_err());
        let _first = create_pipe(&path, true).unwrap();
        assert!(create_pipe(&path, true).is_err());
    }
}
