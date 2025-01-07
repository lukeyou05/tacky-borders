use anyhow::{anyhow, Context};
use dirs::home_dir;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, LazyLock, Mutex};
use std::{fs, io, mem, ptr, thread};
use windows::Win32::Foundation::{FALSE, HANDLE, HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST;

use crate::colors::ColorConfig;
use crate::iocp::CompletionPort;
use crate::iocp::UnixListener;
use crate::utils::{get_foreground_window, post_message_w, LogIfErr, WM_APP_KOMOREBI};
use crate::windows_api::{is_zoomed, monitor_from_window};
use crate::APP_STATE;

// NOTE: in komorebi it's <border hwnd, WindowKind>, but here it's <tracking hwnd, WindowKind>
pub static FOCUS_STATE: LazyLock<Mutex<HashMap<isize, WindowKind>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
pub struct KomorebiColorsConfig {
    pub stack_color: ColorConfig,
    pub monocle_color: ColorConfig,
    pub floating_color: ColorConfig,
}

pub struct KomorebiIntegration {
    pub unix_listener: Option<Arc<Mutex<UnixListener>>>,
}

impl KomorebiIntegration {
    pub fn new() -> Self {
        Self {
            unix_listener: None,
        }
    }

    pub fn start(&mut self) -> anyhow::Result<()> {
        debug!("starting komorebic integration");

        // TODO: should handle the None case instead of using unwrap_or_default()
        let socket_path = Self::get_komorebic_socket_path().unwrap_or_default();

        if fs::exists(&socket_path).context("could not check if file exists")? {
            fs::remove_file(&socket_path)?;
        }

        let listener = UnixListener::bind(&socket_path)?;
        listener.listen()?;

        let completion_port = CompletionPort::new(1)?;
        completion_port.associate_handle(HANDLE(listener.socket.0 .0 as _))?;

        let listener_arc = Arc::new(Mutex::new(listener));

        self.unix_listener = Some(listener_arc.clone());

        let _ = thread::spawn(|| {
            move || -> anyhow::Result<()> {
                let listener = listener_arc;

                loop {
                    let mut outputbuffer = [0u8; 8192];

                    let mut stream = listener.lock().unwrap().accept()?;
                    completion_port.associate_handle(HANDLE(stream.socket.0 .0 as _))?;

                    // wait for accept
                    let overlapped_entry = completion_port.get_queued_completion_status(None)?;

                    /*println!(
                        "{} {}",
                        overlapped_entry.lpCompletionKey,
                        listener_arc.lock().unwrap().socket.0 .0
                    );*/

                    stream.read(&mut outputbuffer)?;

                    let overlapped_entry = completion_port.get_queued_completion_status(None)?;

                    // TODO: this shouldnt be necessary since it's in the OVERLAPPED_ENTRY?
                    /*let mut bytes_received = 0u32;

                    unsafe {
                        WSAGetOverlappedResult(
                            stream.socket.0,
                            &overlapped,
                            &mut bytes_received,
                            FALSE,
                            &mut flags,
                        )
                    }
                    .unwrap();*/

                    Self::process_komorebi_notification(
                        &outputbuffer,
                        overlapped_entry.dwNumberOfBytesTransferred as i32,
                    );
                }
            }()
            .log_if_err();
        });

        // Subscribe to komorebic
        Command::new("komorebic")
            .arg("subscribe-socket")
            .arg("tacky-borders.sock")
            .spawn()
            .context("could not subscribe to komorebic socket")?;

        Ok(())
    }

    pub fn stop(&mut self) -> anyhow::Result<()> {
        if let Some(ref unix_listener) = self.unix_listener {
            unix_listener.lock().unwrap().shutdown();
            self.unix_listener = None;
        }

        Ok(())
    }

    pub fn get_komorebic_socket_path() -> anyhow::Result<PathBuf> {
        let home_dir = home_dir().context("could not get home dir")?;

        Ok(home_dir
            .join("AppData")
            .join("Local")
            .join("komorebi")
            .join("tacky-borders.sock"))
    }

    // Largely adapted from komorebi's own border implementation. Thanks @LGUG2Z
    pub fn process_komorebi_notification(buffer: &[u8], bytes_received: i32) {
        let notification: serde_json::Value =
            match serde_json::from_slice(&buffer[..bytes_received as usize]) {
                Ok(event) => event,
                Err(err) => {
                    error!("could not parse unix domain socket buffer: {err}");
                    return;
                }
            };

        let previous_focus_state = (*FOCUS_STATE.lock().unwrap()).clone();

        // TODO: replace all the unwrap with unwrap_or_default later, rn we just do it to test. also
        // use filter() on some of the Option values
        let monitors = &notification["state"]["monitors"];
        let focused_monitor_idx = monitors["focused"].as_u64().unwrap() as usize;
        let focused_workspace_idx = monitors["elements"].as_array().unwrap()[focused_monitor_idx]
            ["workspaces"]["focused"]
            .as_u64()
            .unwrap() as usize;
        let floating_window_hwnds = monitors["elements"].as_array().unwrap()[focused_monitor_idx]
            ["workspaces"]["elements"]
            .as_array()
            .unwrap()[focused_workspace_idx]["floating_windows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["hwnd"].as_i64().unwrap() as isize)
            .collect::<Vec<_>>();
        let foreground_window = get_foreground_window();

        for (monitor_idx, m) in monitors["elements"].as_array().unwrap().iter().enumerate() {
            // Only operate on the focused workspace of each monitor
            if let Some(ws) = m["workspaces"]["elements"]
                .as_array()
                .unwrap()
                .get(m["workspaces"]["focused"].as_u64().unwrap() as usize)
            {
                // Workspaces with tiling disabled don't have borders
                if !ws["tile"].as_bool().unwrap() {
                    // NOTE: i dont think we have to do anything here
                    println!("not tiled");
                }

                // Handle the monocle container separately
                if let Some(monocole) = ws.get("monocle_container").filter(|value| !value.is_null())
                {
                    let new_focus_state = if monitor_idx != focused_monitor_idx {
                        WindowKind::Unfocused
                    } else {
                        WindowKind::Monocle
                    };
                    {
                        let mut focus_state = FOCUS_STATE.lock().unwrap();
                        focus_state.insert(
                            // NOTE: if this is a monocole, I assume there's only 1 window in "windows"
                            monocole["windows"]["elements"].as_array().unwrap()[0]["hwnd"]
                                .as_i64()
                                .unwrap() as isize,
                            new_focus_state,
                        );
                    }
                }

                let foreground_hwnd = get_foreground_window();
                let foreground_monitor_id =
                    monitor_from_window(foreground_hwnd, MONITOR_DEFAULTTONEAREST);
                let is_maximized = foreground_monitor_id.0 as isize
                    == m["id"].as_i64().unwrap() as isize
                    && is_zoomed(foreground_hwnd);

                if is_maximized {
                    // NOTE: again, don't think we need to do anything here
                    println!("is maximized");
                }

                // Destroy any borders not associated with the focused workspace
                // NOTE: ^ again, don't think we need to do this

                for (idx, c) in ws["containers"]["elements"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .enumerate()
                {
                    #[allow(unused_assignments)]
                    let mut last_focus_state = None;

                    let new_focus_state = if idx
                        != ws["containers"]["focused"].as_i64().unwrap() as usize
                        || monitor_idx != focused_monitor_idx
                        || c["windows"]["elements"]
                            .as_array()
                            .unwrap()
                            .get(c["windows"]["focused"].as_u64().unwrap() as usize)
                            .map(|w| {
                                w["hwnd"].as_i64().unwrap() as isize != foreground_hwnd.0 as isize
                            })
                            .unwrap_or_default()
                    {
                        WindowKind::Unfocused
                    } else if c["windows"]["elements"].as_array().unwrap().len() > 1 {
                        WindowKind::Stack
                    } else {
                        WindowKind::Single
                    };

                    // Update the window kind for all containers on this workspace
                    {
                        let mut focus_state = FOCUS_STATE.lock().unwrap();
                        last_focus_state = focus_state.insert(
                            c["windows"]["elements"].as_array().unwrap()
                                [c["windows"]["focused"].as_u64().unwrap() as usize]["hwnd"]
                                .as_i64()
                                .unwrap() as isize,
                            new_focus_state,
                        );
                    }
                }
                {
                    for window in ws["floating_windows"].as_array().unwrap() {
                        #[allow(unused_assignments)]
                        let mut last_focus_state = None;
                        let mut new_focus_state = WindowKind::Unfocused;

                        if foreground_window.0 as isize == window["hwnd"].as_i64().unwrap() as isize
                        {
                            new_focus_state = WindowKind::Floating;
                        }

                        {
                            let mut focus_state = FOCUS_STATE.lock().unwrap();
                            last_focus_state = focus_state
                                .insert(window["hwnd"].as_i64().unwrap() as isize, new_focus_state);
                        }
                    }
                }
            }
        }

        let new_focus_state = FOCUS_STATE.lock().unwrap();

        for (tracking, border) in APP_STATE.borders.lock().unwrap().iter() {
            let previous_window_kind = previous_focus_state.get(tracking);
            let new_window_kind = new_focus_state.get(tracking);

            // Only post update messages when the window kind has actually changed
            if previous_window_kind != new_window_kind {
                let border_hwnd = HWND(*border as _);
                post_message_w(border_hwnd, WM_APP_KOMOREBI, WPARAM(0), LPARAM(0))
                    .context("WM_APP_KOMOREBI")
                    .log_if_err();
            }
        }

        //let focus_state = FOCUS_STATE.lock().unwrap();
        //println!("focus_states: {:?}", focus_state);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WindowKind {
    Single,
    Stack,
    Monocle,
    Unfocused,
    Floating,
}
