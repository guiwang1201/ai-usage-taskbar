//! "Start with Windows" through the per-user Run key. No admin rights, no scheduled task.

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::*;

use super::wide;

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE_NAME: &str = "IUsage";

/// The quoted path of the running executable, as UTF-16 without a terminator.
fn command() -> Option<Vec<u16>> {
    let exe = std::env::current_exe().ok()?;
    let quote = '"' as u16;
    Some(std::iter::once(quote).chain(exe.as_os_str().encode_wide()).chain(std::iter::once(quote)).collect())
}

/// True only when the Run entry points at this executable. An entry left behind by a
/// copy that has since been moved reads as off, and turning it on repairs it.
pub fn is_enabled() -> bool {
    match (read(RUN_KEY, VALUE_NAME), command()) {
        (Some(stored), Some(current)) => same_path(&stored, &current),
        _ => false,
    }
}

/// Paths differ at most in ASCII case here (drive letter, how the exe was launched);
/// folding only ASCII keeps the Unicode case tables out of the binary.
fn same_path(a: &[u16], b: &[u16]) -> bool {
    let fold = |unit: u16| if (b'A' as u16..=b'Z' as u16).contains(&unit) { unit + 32 } else { unit };
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| fold(*x) == fold(*y))
}

pub fn set_enabled(enabled: bool) -> bool {
    if enabled {
        command().is_some_and(|command| write(RUN_KEY, VALUE_NAME, &command))
    } else {
        delete(RUN_KEY, VALUE_NAME)
    }
}

fn read(subkey: &str, name: &str) -> Option<Vec<u16>> {
    let (subkey, name) = (wide(subkey), wide(name));
    unsafe {
        let mut bytes: u32 = 0;
        let status =
            RegGetValueW(HKEY_CURRENT_USER, subkey.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, null_mut(), null_mut(), &mut bytes);
        if status != ERROR_SUCCESS || bytes == 0 {
            return None;
        }
        let mut buffer = vec![0u16; bytes as usize / 2 + 1];
        let mut bytes = (buffer.len() * 2) as u32;
        let status = RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            buffer.as_mut_ptr() as *mut c_void,
            &mut bytes,
        );
        if status != ERROR_SUCCESS {
            return None;
        }
        let length = buffer.iter().position(|unit| *unit == 0).unwrap_or(buffer.len());
        buffer.truncate(length);
        Some(buffer)
    }
}

fn write(subkey: &str, name: &str, value: &[u16]) -> bool {
    let (subkey, name) = (wide(subkey), wide(name));
    let data: Vec<u16> = value.iter().copied().chain(std::iter::once(0)).collect();
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            name.as_ptr(),
            REG_SZ,
            data.as_ptr() as *const c_void,
            (data.len() * 2) as u32,
        ) == ERROR_SUCCESS
    }
}

fn delete(subkey: &str, name: &str) -> bool {
    let (subkey, name) = (wide(subkey), wide(name));
    let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, subkey.as_ptr(), name.as_ptr()) };
    status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_round_trips_through_the_registry() {
        // A scratch key, so the test never touches the real Run entry.
        let subkey = format!("Software\\IUsage-test-{}", std::process::id());
        let value: Vec<u16> = "\"D:\\软件 项目\\IUsage.exe\"".encode_utf16().collect();

        assert_eq!(read(&subkey, "IUsage"), None);
        assert!(write(&subkey, "IUsage", &value));
        assert_eq!(read(&subkey, "IUsage"), Some(value));
        assert!(delete(&subkey, "IUsage"));
        assert_eq!(read(&subkey, "IUsage"), None);
        assert!(delete(&subkey, "IUsage"), "deleting a missing value is not an error");

        let status = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, wide(&subkey).as_ptr()) };
        assert_eq!(status, ERROR_SUCCESS);
    }

    #[test]
    fn paths_compare_ignoring_ascii_case_only() {
        let units = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
        assert!(same_path(&units("\"D:\\项目\\IUsage.exe\""), &units("\"d:\\项目\\iusage.EXE\"")));
        assert!(!same_path(&units("\"D:\\a\\IUsage.exe\""), &units("\"D:\\b\\IUsage.exe\"")));
        assert!(!same_path(&units("\"D:\\a\\IUsage.exe\""), &units("\"D:\\a\\IUsage.exe\" ")));
    }

    #[test]
    fn command_is_the_quoted_executable_path() {
        let command = String::from_utf16(&command().unwrap()).unwrap();
        assert!(command.starts_with('"') && command.ends_with(".exe\""), "{command}");
    }
}
