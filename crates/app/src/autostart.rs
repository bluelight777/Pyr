//! Opening Pyr when Windows starts (like Discord): a value in the current user's Run key
//! that starts Pyr hidden in the tray (`--autostart`). Kept in step with the setting each
//! time Pyr starts, so it follows Pyr if it's reinstalled somewhere else. Windows' own
//! switch for it (Task Manager → Startup apps) is left to Windows and still wins.

#[cfg(windows)]
pub fn apply(on: bool) {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW};
    use windows::core::w;
    let run = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    let name = w!("Pyr");
    let current = unsafe {
        let mut buf = vec![0u16; 1024];
        let mut len = (buf.len() * 2) as u32;
        let r = RegGetValueW(HKEY_CURRENT_USER, run, name, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len));
        (r == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&buf[..(len as usize / 2).saturating_sub(1)]))
    };
    // Ingle's own value (Pyr was called Ingle before 0.8.0): it starts the old program, so
    // it goes, whichever way the setting is (Pyr's value below takes its place).
    let old = w!("Ingle");
    let had_old = unsafe { RegGetValueW(HKEY_CURRENT_USER, run, old, RRF_RT_REG_SZ, None, None, None) } == ERROR_SUCCESS;
    if had_old {
        let _ = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, run, old) };
    }
    if !on {
        if current.is_some() {
            let r = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, run, name) };
            if r != ERROR_SUCCESS {
                tracing::warn!("couldn't stop Pyr opening with Windows: {r:?}");
                pyr_engine::errors::report("Couldn't turn off opening Pyr when Windows starts.");
            }
        }
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let want = format!("\"{}\" --autostart", exe.display());
    if current.as_deref() == Some(want.as_str()) {
        return;
    }
    let data: Vec<u16> = want.encode_utf16().chain(std::iter::once(0)).collect();
    let r = unsafe {
        RegSetKeyValueW(HKEY_CURRENT_USER, run, name, REG_SZ.0, Some(data.as_ptr() as *const _), (data.len() * 2) as u32)
    };
    if r == ERROR_SUCCESS {
        tracing::info!("Pyr opens when Windows starts");
    } else {
        tracing::warn!("couldn't set Pyr to open with Windows: {r:?}");
        pyr_engine::errors::report("Couldn't set Pyr to open when Windows starts.");
    }
}

#[cfg(not(windows))]
pub fn apply(_on: bool) {}
