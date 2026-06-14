use anyhow::Context;
use serde::Deserialize;
use serde_json::json;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};

use crate::APP_STATE;
use crate::colors::ColorBrushConfig;
use crate::config::{Config, WidthConfig};
use crate::iocp::{UnixListener, UnixStream};
use crate::utils::{
    LogIfErr, WM_APP_SET_COLORS, WM_APP_SET_WIDTH, get_border_for_window, post_message_w,
    remove_file_if_exists,
};

/// Payload send to borders through `WM_APP_SET_COLORS`.
pub struct IpcSetColorsPayload {
    pub active_color: Option<ColorBrushConfig>,
    pub inactive_color: Option<ColorBrushConfig>,
}

/// Payload send to borders through `WM_APP_SET_WIDTH`.
pub struct IpcSetWidthPayload {
    pub width_config: WidthConfig,
}

pub fn socket_path() -> anyhow::Result<PathBuf> {
    Config::get_dir().map(|dir| dir.join("tacky-borders.sock"))
}

/// IPC Server that handles communication between a CLI and daemon.
/// Changes made via IPC are not written back to the config file.
pub struct IpcServer {
    socket_path: PathBuf,
    stop: Arc<AtomicBool>,
    thread_handle: Option<JoinHandle<()>>,
}

impl IpcServer {
    pub fn new(socket_path: &Path) -> anyhow::Result<Self> {
        // Remove a stale socket file left over from a previous run; bind fails otherwise
        remove_file_if_exists(socket_path).context("could not remove stale ipc socket")?;

        let listener = UnixListener::bind(socket_path).context("could not bind ipc socket")?;

        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = stop.clone();

        let thread_handle = thread::spawn(move || run_server(listener, stop_clone));

        info!("ipc server listening on {}", socket_path.display());

        Ok(Self {
            socket_path: socket_path.to_owned(),
            stop,
            thread_handle: Some(thread_handle),
        })
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);

        // Unblock the accept() call in the server thread with a dummy connection
        let _ = UnixStream::connect(&self.socket_path);

        match self.thread_handle.take() {
            Some(handle) => {
                if let Err(err) = handle.join() {
                    error!("could not join ipc server thread handle: {err:?}");
                }
            }
            None => error!("could not take ipc server thread handle"),
        }

        // The listener has been dropped (its thread exited), so the socket file can go too
        remove_file_if_exists(&self.socket_path)
            .context("could not remove ipc socket")
            .log_if_err();

        debug!("ipc server stopped");
    }
}

fn run_server(listener: UnixListener, stop: Arc<AtomicBool>) {
    debug!("entering ipc server thread");

    loop {
        match listener.accept() {
            Ok(stream) => {
                // The dummy connection sent by Drop should not be processed
                if stop.load(Ordering::Relaxed) {
                    break;
                }

                thread::spawn(move || {
                    if let Err(err) = handle_client(stream) {
                        debug!("ipc client disconnected: {err:#}");
                    }
                });
            }
            Err(err) => {
                if !stop.load(Ordering::Relaxed) {
                    error!("could not accept ipc client: {err}");
                }
                break;
            }
        }
    }

    debug!("exiting ipc server thread");
}

fn handle_client(stream: UnixStream) -> anyhow::Result<()> {
    let reader = BufReader::new(&stream);

    for line in reader.lines() {
        let line = line.context("could not read line from ipc client")?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let mut response = process_command(trimmed);
        response.push('\n');

        (&stream)
            .write_all(response.as_bytes())
            .context("could not write response to ipc client")?;
    }

    Ok(())
}

/// All commands that can be sent through the IPC mechanism. When serialized,
/// the enum variant is in snake case and denoted with "cmd".
/// Example JSON format: {"cmd":"set_color","active":<color>,"inactive":<color>}
#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum IpcCommand {
    SetColor {
        #[serde(default)]
        active: Option<ColorBrushConfig>,
        #[serde(default)]
        inactive: Option<ColorBrushConfig>,
        /// When true, only the currently focused window's border is updated.
        #[serde(default)]
        focused: bool,
    },
    SetWidth {
        width: WidthConfig,
        /// When true, only the currently focused window's border is updated.
        #[serde(default)]
        focused: bool,
    },
    Reload,
    GetState,
}

fn process_command(raw: &str) -> String {
    let command: IpcCommand = match serde_json::from_str(raw) {
        Ok(command) => command,
        Err(err) => {
            return json!({"ok": false, "error": format!("invalid command: {err}")}).to_string();
        }
    };

    match command {
        IpcCommand::SetColor {
            active,
            inactive,
            focused,
        } => {
            if active.is_none() && inactive.is_none() {
                return json!({"ok": false, "error": "no colors provided"}).to_string();
            }
            apply_colors(active, inactive, focused);
            json!({"ok": true}).to_string()
        }
        IpcCommand::SetWidth { width, focused } => {
            apply_width(width, focused);
            json!({"ok": true}).to_string()
        }
        IpcCommand::Reload => {
            Config::reload();
            crate::reload_borders();
            json!({"ok": true}).to_string()
        }
        IpcCommand::GetState => {
            let config = APP_STATE.config.read().unwrap();
            json!({
                "ok": true,
                "active_window": *APP_STATE.active_window.lock().unwrap(),
                "border_count": APP_STATE.borders.lock().unwrap().len(),
                "active_color": config.global.active_color,
                "inactive_color": config.global.inactive_color,
            })
            .to_string()
        }
    }
}

/// Applies new colors to borders at runtime.
fn apply_colors(
    active: Option<ColorBrushConfig>,
    inactive: Option<ColorBrushConfig>,
    focused_only: bool,
) {
    let border_hwnds: Vec<HWND> = if focused_only {
        let active_tracking = HWND(*APP_STATE.active_window.lock().unwrap() as _);
        get_border_for_window(active_tracking)
            .map(|hwnd| vec![hwnd])
            .unwrap_or_default()
    } else {
        // Update the in-memory global config so newly created borders pick up
        // the colors too.  The config file is never written.
        {
            let mut config = APP_STATE.config.write().unwrap();
            if let Some(ref color) = active {
                config.global.active_color = color.clone();
            }
            if let Some(ref color) = inactive {
                config.global.inactive_color = color.clone();
            }
        }

        APP_STATE
            .borders
            .lock()
            .unwrap()
            .values()
            .map(|hwnd_isize| HWND(*hwnd_isize as _))
            .collect()
    };

    // Each border window gets its own heap-allocated payload so that ownership
    // is unambiguous: the wnd_proc reclaims it with Box::from_raw.
    for border_hwnd in border_hwnds {
        let payload = Box::new(IpcSetColorsPayload {
            active_color: active.clone(),
            inactive_color: inactive.clone(),
        });
        let payload_ptr = Box::into_raw(payload);

        if let Err(err) = post_message_w(
            Some(border_hwnd),
            WM_APP_SET_COLORS,
            WPARAM(0),
            LPARAM(payload_ptr as isize),
        ) {
            // PostMessage failed — reclaim the payload so it isn't leaked
            drop(unsafe { Box::from_raw(payload_ptr) });
            error!("could not post WM_APP_SET_COLORS to {border_hwnd:?}: {err:#}");
        }
    }
}

fn apply_width(width_config: WidthConfig, focused_only: bool) {
    let border_hwnds: Vec<HWND> = if focused_only {
        let active_tracking = HWND(*APP_STATE.active_window.lock().unwrap() as _);
        get_border_for_window(active_tracking)
            .map(|hwnd| vec![hwnd])
            .unwrap_or_default()
    } else {
        APP_STATE.config.write().unwrap().global.border_width = width_config;

        APP_STATE
            .borders
            .lock()
            .unwrap()
            .values()
            .map(|hwnd_isize| HWND(*hwnd_isize as _))
            .collect()
    };

    for border_hwnd in border_hwnds {
        let payload = Box::new(IpcSetWidthPayload { width_config });
        let payload_ptr = Box::into_raw(payload);

        if let Err(err) = post_message_w(
            Some(border_hwnd),
            WM_APP_SET_WIDTH,
            WPARAM(0),
            LPARAM(payload_ptr as isize),
        ) {
            drop(unsafe { Box::from_raw(payload_ptr) });
            error!("could not post WM_APP_SET_WIDTH to {border_hwnd:?}: {err:#}");
        }
    }
}
