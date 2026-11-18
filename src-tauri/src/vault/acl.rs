use std::ffi::OsStr;
use std::fs::{self, File};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;
use std::path::{Path, PathBuf};
use std::ptr;

use chrono::{DateTime, Utc};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, BOOL, ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS,
    ERROR_INVALID_OWNER, ERROR_LOCK_VIOLATION, ERROR_NOT_SAME_DEVICE, ERROR_PRIVILEGE_NOT_HELD,
    ERROR_SHARING_VIOLATION, ERROR_SUCCESS, FILETIME, HANDLE, HLOCAL, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSecurityDescriptorToStringSecurityDescriptorW,
    ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW,
    SetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    GetSecurityDescriptorControl, GetSecurityDescriptorDacl, GetSecurityDescriptorGroup,
    GetSecurityDescriptorOwner, ACL, DACL_SECURITY_INFORMATION, GROUP_SECURITY_INFORMATION,
    OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    SECURITY_DESCRIPTOR_CONTROL, SE_DACL_PROTECTED, UNPROTECTED_DACL_SECURITY_INFORMATION,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, GetCompressedFileSizeW, GetFileAttributesW, GetFileInformationByHandle,
    GetVolumePathNameW, MoveFileExW, SetFileAttributesW, SetFileTime, BY_HANDLE_FILE_INFORMATION,
    DELETE, FILE_ATTRIBUTE_COMPRESSED, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_ENCRYPTED,
    FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_ATTRIBUTE_SPARSE_FILE, FILE_GENERIC_READ, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES, INVALID_FILE_ATTRIBUTES, INVALID_FILE_SIZE,
    MOVEFILE_COPY_ALLOWED, MOVEFILE_WRITE_THROUGH, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::FSCTL_SET_COMPRESSION;

use super::VaultError;

const WINDOWS_TO_UNIX_EPOCH_TICKS: i128 = 116_444_736_000_000_000;
const COMPRESSION_FORMAT_NONE: u16 = 0;
const COMPRESSION_FORMAT_DEFAULT: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSecuritySnapshot {
    pub ntfs_acl_sddl: String,
    pub creation_time_utc: DateTime<Utc>,
    pub modified_time_utc: DateTime<Utc>,
    pub attributes: u32,
    pub logical_size_bytes: u64,
    pub physical_size_bytes: u64,
}

struct LocalAllocGuard(*mut core::ffi::c_void);

impl Drop for LocalAllocGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                LocalFree(self.0 as HLOCAL);
            }
        }
    }
}

struct HandleGuard(HANDLE);

impl HandleGuard {
    fn into_file(mut self) -> File {
        let raw = self.0;
        self.0 = INVALID_HANDLE_VALUE;
        unsafe { File::from_raw_handle(raw as std::os::windows::io::RawHandle) }
    }
}

impl Drop for HandleGuard {
    fn drop(&mut self) {
        if self.0 != INVALID_HANDLE_VALUE && !self.0.is_null() {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

fn to_wide_null(path: &Path) -> Vec<u16> {
    to_wide_null_os(path.as_os_str())
}

fn to_wide_null_os(s: &OsStr) -> Vec<u16> {
    let mut wide: Vec<u16> = s.encode_wide().collect();
    wide.push(0);
    wide
}

fn is_lock_error(code: u32) -> bool {
    code == ERROR_SHARING_VIOLATION || code == ERROR_LOCK_VIOLATION
}

fn map_open_or_move_err(path: &Path, err: io::Error, context: &str) -> VaultError {
    if let Some(raw) = err.raw_os_error() {
        if is_lock_error(raw as u32) {
            return VaultError::FileLocked {
                path: path.display().to_string(),
                source: err,
            };
        }
    }
    VaultError::Io {
        context: context.to_string(),
        source: err,
    }
}

pub fn filetime_to_utc(ft: FILETIME) -> Result<DateTime<Utc>, VaultError> {
    let ticks = ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64);
    let unix_ticks = (ticks as i128) - WINDOWS_TO_UNIX_EPOCH_TICKS;
    let secs = unix_ticks.div_euclid(10_000_000);
    let rem_ticks = unix_ticks.rem_euclid(10_000_000) as u32;
    let nanos = rem_ticks * 100;

    let secs_i64 = i64::try_from(secs).map_err(|_| VaultError::InvalidTimestamp {
        value: ticks.to_string(),
        message: "FILETIME seconds out of DateTime<Utc> range".to_string(),
    })?;

    DateTime::from_timestamp(secs_i64, nanos).ok_or_else(|| VaultError::InvalidTimestamp {
        value: ticks.to_string(),
        message: "FILETIME outside chrono DateTime<Utc> representable range".to_string(),
    })
}

pub fn utc_to_filetime(dt: DateTime<Utc>) -> Result<FILETIME, VaultError> {
    let secs = dt.timestamp() as i128;
    let sub_nanos = dt.timestamp_subsec_nanos() as i128;
    let unix_ticks = secs * 10_000_000 + (sub_nanos / 100);
    let win_ticks = unix_ticks + WINDOWS_TO_UNIX_EPOCH_TICKS;
    if win_ticks < 0 {
        return Err(VaultError::InvalidTimestamp {
            value: dt.to_rfc3339(),
            message: "timestamp predates Windows FILETIME epoch (1601-01-01)".to_string(),
        });
    }
    let ticks_u64 = u64::try_from(win_ticks).map_err(|_| VaultError::InvalidTimestamp {
        value: dt.to_rfc3339(),
        message: "timestamp overflows u64 FILETIME".to_string(),
    })?;
    Ok(FILETIME {
        dwLowDateTime: (ticks_u64 & 0xFFFF_FFFF) as u32,
        dwHighDateTime: (ticks_u64 >> 32) as u32,
    })
}

/// Validates that `path` is a regular non-symlink file and opens an exclusive
/// read+delete handle (`dwShareMode = 0`) so any active lock from another
/// process is caught immediately without modifying disk state.
pub fn ensure_not_locked_and_open_read(path: &Path) -> Result<File, VaultError> {
    let meta = fs::symlink_metadata(path).map_err(|err| {
        if err.kind() == io::ErrorKind::NotFound {
            VaultError::ItemNotFound {
                path: path.display().to_string(),
            }
        } else {
            map_open_or_move_err(
                path,
                err,
                &format!("reading metadata for '{}'", path.display()),
            )
        }
    })?;

    if !meta.file_type().is_file() {
        return Err(VaultError::NotAFile {
            path: path.display().to_string(),
        });
    }

    let wide = to_wide_null(path);
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_GENERIC_READ | DELETE,
            0, // FILE_SHARE_NONE: fails if any other open handle conflicts
            ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };

    if handle == INVALID_HANDLE_VALUE {
        let err = io::Error::last_os_error();
        return Err(map_open_or_move_err(
            path,
            err,
            &format!("checking exclusive lock on '{}'", path.display()),
        ));
    }

    Ok(HandleGuard(handle).into_file())
}

pub fn get_physical_file_size(path: &Path, fallback_logical: u64) -> u64 {
    let wide = to_wide_null(path);
    let mut high: u32 = 0;
    let low = unsafe { GetCompressedFileSizeW(wide.as_ptr(), &mut high) };
    if low == INVALID_FILE_SIZE {
        let err = unsafe { GetLastError() };
        if err != ERROR_SUCCESS {
            return fallback_logical;
        }
    }
    ((high as u64) << 32) | (low as u64)
}

pub fn capture_sddl(path: &Path) -> Result<String, VaultError> {
    let wide = to_wide_null(path);
    let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
    let sec_info = OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;

    let status = unsafe {
        GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            sec_info,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &mut sd,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(VaultError::AclError {
            path: path.display().to_string(),
            message: format!(
                "GetNamedSecurityInfoW failed: {}",
                io::Error::from_raw_os_error(status as i32)
            ),
        });
    }
    let _sd_guard = LocalAllocGuard(sd);

    let mut string_sd: *mut u16 = ptr::null_mut();
    let mut string_sd_len: u32 = 0;
    let ok = unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            sd,
            SDDL_REVISION_1,
            sec_info,
            &mut string_sd,
            &mut string_sd_len,
        )
    };
    if ok == 0 || string_sd.is_null() {
        return Err(VaultError::AclError {
            path: path.display().to_string(),
            message: format!(
                "ConvertSecurityDescriptorToStringSecurityDescriptorW failed: {}",
                io::Error::last_os_error()
            ),
        });
    }
    let _str_guard = LocalAllocGuard(string_sd.cast());

    let slice = unsafe {
        let mut len = 0usize;
        while *string_sd.add(len) != 0 {
            len += 1;
        }
        std::slice::from_raw_parts(string_sd, len)
    };
    Ok(String::from_utf16_lossy(slice))
}

pub fn capture_file_security_snapshot(path: &Path) -> Result<FileSecuritySnapshot, VaultError> {
    let file = ensure_not_locked_and_open_read(path)?;
    capture_security_snapshot_from_handle(path, &file)
}

pub fn capture_security_snapshot_from_handle(
    path: &Path,
    file: &File,
) -> Result<FileSecuritySnapshot, VaultError> {
    use std::os::windows::io::AsRawHandle;

    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut info) };
    if ok == 0 {
        return Err(VaultError::Io {
            context: format!("GetFileInformationByHandle on '{}'", path.display()),
            source: io::Error::last_os_error(),
        });
    }

    let logical_size_bytes = ((info.nFileSizeHigh as u64) << 32) | (info.nFileSizeLow as u64);
    let physical_size_bytes = get_physical_file_size(path, logical_size_bytes);
    let creation_time_utc = filetime_to_utc(info.ftCreationTime)?;
    let modified_time_utc = filetime_to_utc(info.ftLastWriteTime)?;
    let ntfs_acl_sddl = capture_sddl(path)?;

    Ok(FileSecuritySnapshot {
        ntfs_acl_sddl,
        creation_time_utc,
        modified_time_utc,
        attributes: info.dwFileAttributes,
        logical_size_bytes,
        physical_size_bytes,
    })
}

pub fn get_volume_root(path: &Path) -> Result<PathBuf, VaultError> {
    let abs = std::path::absolute(path).map_err(|err| VaultError::Io {
        context: format!("resolving absolute path for '{}'", path.display()),
        source: err,
    })?;
    let wide = to_wide_null(&abs);
    let buf_len = (wide.len() + 64).max(4096);
    let mut out = vec![0u16; buf_len];

    let ok = unsafe { GetVolumePathNameW(wide.as_ptr(), out.as_mut_ptr(), out.len() as u32) };
    if ok == 0 {
        return Err(VaultError::Io {
            context: format!("GetVolumePathNameW for '{}'", abs.display()),
            source: io::Error::last_os_error(),
        });
    }

    let nul_pos = out.iter().position(|&c| c == 0).unwrap_or(out.len());
    let raw = String::from_utf16_lossy(&out[..nul_pos]);
    let normalized = raw.strip_prefix(r"\\?\").unwrap_or(&raw);
    Ok(PathBuf::from(normalized))
}

pub fn atomic_move_file(
    src: &Path,
    dst: &Path,
    allow_cross_volume_fallback: bool,
) -> Result<(), VaultError> {
    let src_w = to_wide_null(src);
    let dst_w = to_wide_null(dst);

    let ok = unsafe { MoveFileExW(src_w.as_ptr(), dst_w.as_ptr(), MOVEFILE_WRITE_THROUGH) };
    if ok != 0 {
        return Ok(());
    }

    let err = io::Error::last_os_error();
    let code = err.raw_os_error().unwrap_or_default() as u32;
    if allow_cross_volume_fallback && code == ERROR_NOT_SAME_DEVICE {
        let retry_ok = unsafe {
            MoveFileExW(
                src_w.as_ptr(),
                dst_w.as_ptr(),
                MOVEFILE_WRITE_THROUGH | MOVEFILE_COPY_ALLOWED,
            )
        };
        if retry_ok != 0 {
            return Ok(());
        }
        let retry_err = io::Error::last_os_error();
        let retry_code = retry_err.raw_os_error().unwrap_or_default() as u32;
        if retry_code == ERROR_FILE_EXISTS || retry_code == ERROR_ALREADY_EXISTS {
            return Err(VaultError::DestinationExists {
                path: dst.display().to_string(),
            });
        }
        return Err(map_open_or_move_err(
            src,
            retry_err,
            &format!("moving '{}' to '{}'", src.display(), dst.display()),
        ));
    }

    if code == ERROR_FILE_EXISTS || code == ERROR_ALREADY_EXISTS {
        return Err(VaultError::DestinationExists {
            path: dst.display().to_string(),
        });
    }

    Err(map_open_or_move_err(
        src,
        err,
        &format!("moving '{}' to '{}'", src.display(), dst.display()),
    ))
}

pub fn try_set_ntfs_compression(path: &Path, enable: bool) -> Result<bool, VaultError> {
    let wide = to_wide_null(path);
    let attrs = unsafe { GetFileAttributesW(wide.as_ptr()) };
    if attrs == INVALID_FILE_ATTRIBUTES {
        return Ok(false);
    }
    let was_readonly = (attrs & FILE_ATTRIBUTE_READONLY) != 0;
    if was_readonly {
        unsafe {
            SetFileAttributesW(wide.as_ptr(), attrs & !FILE_ATTRIBUTE_READONLY);
        }
    }

    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_GENERIC_READ | windows_sys::Win32::Storage::FileSystem::FILE_GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };

    let result = if handle == INVALID_HANDLE_VALUE {
        false
    } else {
        let _guard = HandleGuard(handle);
        let mut format: u16 = if enable {
            COMPRESSION_FORMAT_DEFAULT
        } else {
            COMPRESSION_FORMAT_NONE
        };
        let mut bytes_returned: u32 = 0;
        let ok = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_SET_COMPRESSION,
                (&mut format as *mut u16).cast(),
                std::mem::size_of::<u16>() as u32,
                ptr::null_mut(),
                0,
                &mut bytes_returned,
                ptr::null_mut(),
            )
        };
        ok != 0
    };

    if was_readonly {
        // Re-read current attributes so compression attribute bit is preserved if toggled
        let updated = unsafe { GetFileAttributesW(wide.as_ptr()) };
        if updated != INVALID_FILE_ATTRIBUTES {
            unsafe {
                SetFileAttributesW(wide.as_ptr(), updated | FILE_ATTRIBUTE_READONLY);
            }
        }
    }

    Ok(result)
}

fn set_file_timestamps(
    path: &Path,
    creation_time: DateTime<Utc>,
    modified_time: DateTime<Utc>,
) -> Result<(), VaultError> {
    let ctime = utc_to_filetime(creation_time)?;
    let mtime = utc_to_filetime(modified_time)?;
    let wide = to_wide_null(path);

    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_WRITE_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(VaultError::Io {
            context: format!("opening '{}' to restore timestamps", path.display()),
            source: io::Error::last_os_error(),
        });
    }
    let _guard = HandleGuard(handle);

    let ok = unsafe { SetFileTime(handle, &ctime, ptr::null(), &mtime) };
    if ok == 0 {
        return Err(VaultError::Io {
            context: format!("SetFileTime on '{}'", path.display()),
            source: io::Error::last_os_error(),
        });
    }
    Ok(())
}

fn restore_sddl(path: &Path, sddl: &str) -> Result<(), VaultError> {
    if sddl.trim().is_empty() {
        return Ok(());
    }

    let sddl_wide = to_wide_null_os(OsStr::new(sddl));
    let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
    let mut sd_size: u32 = 0;

    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl_wide.as_ptr(),
            SDDL_REVISION_1,
            &mut sd,
            &mut sd_size,
        )
    };
    if ok == 0 || sd.is_null() {
        return Err(VaultError::AclError {
            path: path.display().to_string(),
            message: format!(
                "ConvertStringSecurityDescriptorToSecurityDescriptorW failed: {}",
                io::Error::last_os_error()
            ),
        });
    }
    let _sd_guard = LocalAllocGuard(sd);

    let mut owner: PSID = ptr::null_mut();
    let mut owner_defaulted: BOOL = 0;
    if unsafe { GetSecurityDescriptorOwner(sd, &mut owner, &mut owner_defaulted) } == 0 {
        return Err(VaultError::AclError {
            path: path.display().to_string(),
            message: format!(
                "GetSecurityDescriptorOwner failed: {}",
                io::Error::last_os_error()
            ),
        });
    }

    let mut group: PSID = ptr::null_mut();
    let mut group_defaulted: BOOL = 0;
    if unsafe { GetSecurityDescriptorGroup(sd, &mut group, &mut group_defaulted) } == 0 {
        return Err(VaultError::AclError {
            path: path.display().to_string(),
            message: format!(
                "GetSecurityDescriptorGroup failed: {}",
                io::Error::last_os_error()
            ),
        });
    }

    let mut dacl_present: BOOL = 0;
    let mut dacl: *mut ACL = ptr::null_mut();
    let mut dacl_defaulted: BOOL = 0;
    if unsafe { GetSecurityDescriptorDacl(sd, &mut dacl_present, &mut dacl, &mut dacl_defaulted) }
        == 0
    {
        return Err(VaultError::AclError {
            path: path.display().to_string(),
            message: format!(
                "GetSecurityDescriptorDacl failed: {}",
                io::Error::last_os_error()
            ),
        });
    }

    let mut control: SECURITY_DESCRIPTOR_CONTROL = 0;
    let mut revision: u32 = 0;
    if unsafe { GetSecurityDescriptorControl(sd, &mut control, &mut revision) } == 0 {
        return Err(VaultError::AclError {
            path: path.display().to_string(),
            message: format!(
                "GetSecurityDescriptorControl failed: {}",
                io::Error::last_os_error()
            ),
        });
    }

    let dacl_inherit_flag = if (control & SE_DACL_PROTECTED) != 0 {
        PROTECTED_DACL_SECURITY_INFORMATION
    } else {
        UNPROTECTED_DACL_SECURITY_INFORMATION
    };

    let path_wide = to_wide_null(path);
    let mut full_sec_info = dacl_inherit_flag;
    if !owner.is_null() {
        full_sec_info |= OWNER_SECURITY_INFORMATION;
    }
    if !group.is_null() {
        full_sec_info |= GROUP_SECURITY_INFORMATION;
    }
    if dacl_present != 0 {
        full_sec_info |= DACL_SECURITY_INFORMATION;
    }

    let status = unsafe {
        SetNamedSecurityInfoW(
            path_wide.as_ptr(),
            SE_FILE_OBJECT,
            full_sec_info,
            owner,
            group,
            if dacl_present != 0 { dacl } else { ptr::null() },
            ptr::null(),
        )
    };

    if status == ERROR_SUCCESS {
        return Ok(());
    }

    // Fallback when running unelevated and original owner/group requires SeRestorePrivilege
    if (status == ERROR_INVALID_OWNER || status == ERROR_PRIVILEGE_NOT_HELD) && dacl_present != 0
    {
        let dacl_only_status = unsafe {
            SetNamedSecurityInfoW(
                path_wide.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | dacl_inherit_flag,
                ptr::null_mut(),
                ptr::null_mut(),
                dacl,
                ptr::null(),
            )
        };
        if dacl_only_status == ERROR_SUCCESS {
            return Ok(());
        }
        return Err(VaultError::AclError {
            path: path.display().to_string(),
            message: format!(
                "SetNamedSecurityInfoW DACL fallback failed: {}",
                io::Error::from_raw_os_error(dacl_only_status as i32)
            ),
        });
    }

    Err(VaultError::AclError {
        path: path.display().to_string(),
        message: format!(
            "SetNamedSecurityInfoW failed: {}",
            io::Error::from_raw_os_error(status as i32)
        ),
    })
}

fn sanitize_settable_attributes(attrs: u32) -> u32 {
    let masked = attrs
        & !(FILE_ATTRIBUTE_DIRECTORY
            | FILE_ATTRIBUTE_REPARSE_POINT
            | FILE_ATTRIBUTE_COMPRESSED
            | FILE_ATTRIBUTE_SPARSE_FILE
            | FILE_ATTRIBUTE_ENCRYPTED);
    if masked == 0 {
        FILE_ATTRIBUTE_NORMAL
    } else {
        masked
    }
}

pub fn restore_file_metadata_and_acl(
    path: &Path,
    sddl: &str,
    creation_time: DateTime<Utc>,
    modified_time: DateTime<Utc>,
    attributes: u32,
) -> Result<(), VaultError> {
    // Restore original uncompressed state if the file was compressed inside the vault
    if (attributes & FILE_ATTRIBUTE_COMPRESSED) == 0 {
        let _ = try_set_ntfs_compression(path, false);
    }

    let wide = to_wide_null(path);
    let current_attrs = unsafe { GetFileAttributesW(wide.as_ptr()) };
    if current_attrs != INVALID_FILE_ATTRIBUTES && (current_attrs & FILE_ATTRIBUTE_READONLY) != 0 {
        unsafe {
            SetFileAttributesW(wide.as_ptr(), current_attrs & !FILE_ATTRIBUTE_READONLY);
        }
    }

    set_file_timestamps(path, creation_time, modified_time)?;
    restore_sddl(path, sddl)?;

    // Ensure timestamps remain exact if ACL update touched anything
    let _ = set_file_timestamps(path, creation_time, modified_time);

    let target_attrs = sanitize_settable_attributes(attributes);
    let attr_ok = unsafe { SetFileAttributesW(wide.as_ptr(), target_attrs) };
    if attr_ok == 0 {
        return Err(VaultError::Io {
            context: format!("SetFileAttributesW on '{}'", path.display()),
            source: io::Error::last_os_error(),
        });
    }

    Ok(())
}

pub fn clear_readonly_and_remove_file(path: &Path) -> Result<(), VaultError> {
    let wide = to_wide_null(path);
    let attrs = unsafe { GetFileAttributesW(wide.as_ptr()) };
    if attrs != INVALID_FILE_ATTRIBUTES && (attrs & FILE_ATTRIBUTE_READONLY) != 0 {
        unsafe {
            SetFileAttributesW(wide.as_ptr(), attrs & !FILE_ATTRIBUTE_READONLY);
        }
    }

    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(map_open_or_move_err(
            path,
            err,
            &format!("unlinking vault file '{}'", path.display()),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;
    use tempfile::tempdir;

    #[test]
    fn captures_and_restores_sddl_timestamps_and_attributes() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("acl_roundtrip.txt");
        std::fs::write(&file_path, b"acl test").unwrap();

        let target_ctime = Utc.with_ymd_and_hms(2024, 5, 1, 10, 20, 30).unwrap()
            + chrono::Duration::nanoseconds(123_456_700);
        let target_mtime = Utc.with_ymd_and_hms(2025, 1, 15, 8, 0, 5).unwrap()
            + chrono::Duration::nanoseconds(987_654_300);

        let initial_snap = capture_file_security_snapshot(&file_path).unwrap();
        assert!(!initial_snap.ntfs_acl_sddl.is_empty());

        restore_file_metadata_and_acl(
            &file_path,
            &initial_snap.ntfs_acl_sddl,
            target_ctime,
            target_mtime,
            initial_snap.attributes | 0x1, // FILE_ATTRIBUTE_READONLY
        )
        .unwrap();

        let after_snap = capture_file_security_snapshot(&file_path).unwrap();
        assert_eq!(after_snap.ntfs_acl_sddl, initial_snap.ntfs_acl_sddl);
        assert_eq!(after_snap.creation_time_utc, target_ctime);
        assert_eq!(after_snap.modified_time_utc, target_mtime);
        assert_ne!(after_snap.attributes & 0x1, 0);

        clear_readonly_and_remove_file(&file_path).unwrap();
        assert!(!file_path.exists());
    }

    #[test]
    fn detects_exclusively_locked_file_gracefully() {
        let dir = tempdir().unwrap();
        let locked_path = dir.path().join("locked.bin");
        std::fs::write(&locked_path, b"locked content").unwrap();

        let exclusive_handle = OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&locked_path)
            .unwrap();

        let err = ensure_not_locked_and_open_read(&locked_path).unwrap_err();
        assert!(matches!(err, VaultError::FileLocked { .. }));
        drop(exclusive_handle);

        // Also reject when another process holds FILE_SHARE_READ without FILE_SHARE_DELETE
        let read_no_delete_handle = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&locked_path)
            .unwrap();
        let err_read = ensure_not_locked_and_open_read(&locked_path).unwrap_err();
        assert!(matches!(err_read, VaultError::FileLocked { .. }));
        drop(read_no_delete_handle);
    }

    #[test]
    fn resolves_volume_root_for_temp_path() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("vol_probe.txt");
        std::fs::write(&file_path, b"ok").unwrap();

        let root = get_volume_root(&file_path).unwrap();
        let root_str = root.to_string_lossy();
        assert!(root_str.ends_with('\\'));
        assert!(root_str.contains(":\\"));
    }
}
