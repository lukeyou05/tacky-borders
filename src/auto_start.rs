use std::sync::LazyLock;

use anyhow::{Context, Result, anyhow};
use winreg::{
    RegKey,
    enums::{HKEY_CURRENT_USER, KEY_READ},
};

const APP_NAME: &str = "tacky-borders";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

static EXE_PATH: LazyLock<String> = LazyLock::new(|| {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(|str| str.to_owned()))
        .expect("failed to get tackey-borders path")
});

pub fn is_autostart_enabled() -> Result<bool> {
    let exe_path = EXE_PATH.as_str();

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run_key = hkcu
        .open_subkey_with_flags(RUN_KEY, KEY_READ)
        .map_err(|e| anyhow!("failed to open HKEY_CURRENT_USER\\...\\Run - {e}"))?;

    match run_key.get_value::<String, _>(APP_NAME) {
        Ok(value) => Ok(value == exe_path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(anyhow!("failed to get the autostart registry key - {e}")),
    }
}

fn set_autostart(is_enabled: bool) -> Result<()> {
    let exe_path = EXE_PATH.as_str();

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (run_key, _disp) = hkcu.create_subkey(RUN_KEY)?;

    if is_enabled {
        if let Err(e) = run_key.delete_value(APP_NAME) {
            if e.kind() != std::io::ErrorKind::NotFound {
                return Err(anyhow!("failed to delete the autostart registry key"));
            }
        }
    } else {
        run_key
            .set_value(APP_NAME, &exe_path)
            .context("failed to set the autostart registry key")?;
    }

    Ok(())
}

pub fn toggle_autostart() -> Result<()> {
    let is_enabled = is_autostart_enabled()?;

    set_autostart(is_enabled)?;

    Ok(())
}
