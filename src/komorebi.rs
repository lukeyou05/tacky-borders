use anyhow::{anyhow, Context};
use dirs::home_dir;
use serde::Deserialize;
use std::collections::{HashMap, VecDeque};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::{fs, thread, time};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Networking::WinSock::{closesocket, WSACleanup, WSAStartup, WSADATA};
use windows::Win32::System::Threading::CREATE_NO_WINDOW;
use windows::Win32::System::IO::OVERLAPPED_ENTRY;

use crate::colors::ColorBrushConfig;
use crate::config::{serde_default_bool, Config};
use crate::iocp::{CompletionPort, UnixDomainSocket};
use crate::iocp::{UnixListener, UnixStream};
use crate::utils::{get_foreground_window, post_message_w, LogIfErr, WM_APP_KOMOREBI};
use crate::APP_STATE;

const BUFFER_POOL_REFRESH_INTERVAL: time::Duration = time::Duration::from_secs(600);
const BUFFER_SIZE: usize = 32768;

#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct KomorebiColorsConfig {
    pub stack_color: Option<ColorBrushConfig>,
    pub monocle_color: Option<ColorBrushConfig>,
    pub floating_color: Option<ColorBrushConfig>,
    #[serde(default = "serde_default_bool::<true>")]
    pub enabled: bool,
}

pub struct KomorebiIntegration {
    // NOTE: in komorebi it's <Border HWND, WindowKind>, but here it's <Tracking HWND, WindowKind>
    pub focus_state: Arc<Mutex<HashMap<isize, WindowKind>>>,
    pub listen_socket: Option<UnixDomainSocket>,
}

impl KomorebiIntegration {
    pub fn new() -> Self {
        Self {
            focus_state: Arc::new(Mutex::new(HashMap::new())),
            listen_socket: None,
        }
    }

    pub fn is_enabled(&mut self, config: &Config) -> bool {
        config.global.komorebi_colors.enabled
            || config.window_rules.iter().any(|rule| {
                rule.komorebi_colors
                    .as_ref()
                    .map(|komocolors| komocolors.enabled)
                    .unwrap_or(false)
            })
    }

    // Frankly, this is just a really crappy, Windows-only, version of something like compio.
    // Good learning experience though.
    pub fn start(&mut self) -> anyhow::Result<()> {
        debug!("starting komorebic integration");

        if self.is_running() {
            return Err(anyhow!("komorebi integration is already running"));
        }

        // Start the WinSock service
        let iresult = unsafe { WSAStartup(0x202, &mut WSADATA::default()) };
        if iresult != 0 {
            return Err(anyhow!("WSAStartup failure: {iresult}"));
        }

        let socket_path =
            Self::get_komorebic_socket_path().context("could not get komorebic socket path")?;
        let socket_file = socket_path
            .file_name()
            .and_then(|file| file.to_str())
            .context("could not get komorebic socket name")?;

        // If the socket file already exists, we cannot bind to it, so we must delete it first
        if fs::exists(&socket_path).context("could not check if komorebic socket exists")? {
            fs::remove_file(&socket_path)?;
        }

        let mut listener = UnixListener::bind(&socket_path)?;
        listener.listen()?;

        let port = CompletionPort::new(2)?;
        port.associate_handle(listener.socket.to_handle(), listener.token())?;

        // Prevent overriding already existing sockets
        if self.listen_socket.is_some() {
            return Err(anyhow!("found existing socket; cannot reassign"));
        }
        self.listen_socket = Some(listener.socket.clone());

        let focus_state = self.focus_state.clone();

        let _ = thread::spawn(move || {
            move || -> anyhow::Result<()> {
                let mut entries = vec![OVERLAPPED_ENTRY::default(); 8];
                let mut buffer_pool = VecDeque::<Vec<u8>>::new();
                let mut streams_queue = VecDeque::<(usize, Box<UnixStream>)>::new();

                // Queue up our first accept I/O operation.
                let stream = Box::new(listener.accept()?);
                port.associate_handle(stream.socket.to_handle(), stream.token())?;
                streams_queue.push_back((stream.token(), stream));

                let mut now = time::Instant::now();

                loop {
                    // Clear the buffer pool after the refresh interval has elapsed
                    if now.elapsed() > BUFFER_POOL_REFRESH_INTERVAL {
                        debug!("cleaning up buffer pool for komorebic socket");
                        buffer_pool.clear();
                        now = time::Instant::now();
                    }

                    // This will block until an I/O operation has completed (accept or read)
                    let num_removed = port.poll_many(None, &mut entries)?;

                    for entry in entries[..num_removed as usize].iter() {
                        if entry.lpCompletionKey == listener.token() {
                            // Stream has been accepted; ready to read
                            let stream =
                                &mut streams_queue.back_mut().context("could not get stream")?.1;

                            // Attempt to retrieve a buffer from the bufferpool
                            let outputbuffer = buffer_pool.pop_front().unwrap_or_else(|| {
                                debug!("creating new buffer for komorebic socket");
                                vec![0u8; BUFFER_SIZE]
                            });
                            stream.read(outputbuffer)?;

                            // Queue up a new accept I/O operation.
                            let stream = Box::new(listener.accept()?);
                            port.associate_handle(stream.socket.to_handle(), stream.token())?;
                            streams_queue.push_back((stream.token(), stream));
                        } else {
                            // Stream has been read; ready to process
                            let position = streams_queue
                                .iter()
                                .position(|(token, _)| *token == entry.lpCompletionKey)
                                .context("could not find stream")?;
                            let mut stream = streams_queue
                                .remove(position)
                                .context("could not remove stream from queue")?
                                .1;

                            Self::process_komorebi_notification(
                                focus_state.clone(),
                                &stream.buffer,
                                entry.dwNumberOfBytesTransferred,
                            );

                            // We don't need this stream anymore, so place its buffer into the pool
                            buffer_pool.push_back(stream.take_buffer());
                        }
                    }
                }
            }()
            .log_if_err();
        });

        // Attempt to subscribe to komorebic, stopping integration if subscription fails
        if !Command::new("komorebic")
            .arg("subscribe-socket")
            .arg(socket_file)
            .creation_flags(CREATE_NO_WINDOW.0)
            .status()
            .context("could not get komorebic subscribe-socket exit status")?
            .success()
        {
            error!("could not subscribe to komorebic socket; stopping integration");
            self.stop().context("could not stop komorebi integration")?;
        }

        Ok(())
    }

    pub fn stop(&mut self) -> anyhow::Result<()> {
        debug!("stopping komorebi integration");

        // If this is Some, it means WinSock is (most likely) running, so we need to cleanup. Doing
        // so should also cause the socket worker thread to automatically exit.
        if let Some(ref socket) = self.listen_socket {
            unsafe { WSACleanup() };

            unsafe { closesocket(socket.0) };
            self.listen_socket = None;
        }

        Ok(())
    }

    pub fn is_running(&self) -> bool {
        self.listen_socket.is_some()
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
    pub fn process_komorebi_notification(
        focus_state_mutex: Arc<Mutex<HashMap<isize, WindowKind>>>,
        buffer: &[u8],
        bytes_received: u32,
    ) {
        let notification: serde_json_borrow::Value =
            match serde_json::from_slice(&buffer[..bytes_received as usize]) {
                Ok(event) => event,
                Err(err) => {
                    error!("could not parse unix domain socket buffer: {err}");
                    return;
                }
            };

        let previous_focus_state = (*focus_state_mutex.lock().unwrap()).clone();

        let monitors = notification.get("state").get("monitors");
        let focused_monitor_idx = monitors.get("focused").as_u64().unwrap() as usize;
        let foreground_window = get_foreground_window();

        for (monitor_idx, m) in monitors
            .get("elements")
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            // Only operate on the focused workspace of each monitor
            if let Some(ws) = m
                .get("workspaces")
                .get("elements")
                .as_array()
                .unwrap()
                .get(m.get("workspaces").get("focused").as_u64().unwrap() as usize)
            {
                // Handle the monocle container separately
                let monocle = ws.get("monocle_container");
                if !monocle.is_null() {
                    let new_focus_state = if monitor_idx != focused_monitor_idx {
                        WindowKind::Unfocused
                    } else {
                        WindowKind::Monocle
                    };
                    {
                        let mut focus_state = focus_state_mutex.lock().unwrap();
                        focus_state.insert(
                            // NOTE: if this is a monocole, I assume there's only 1 window in "windows"
                            monocle.get("windows").get("elements").as_array().unwrap()[0]
                                .get("hwnd")
                                .as_i64()
                                .unwrap() as isize,
                            new_focus_state,
                        );
                    }
                }

                let foreground_hwnd = get_foreground_window();

                for (idx, c) in ws
                    .get("containers")
                    .get("elements")
                    .as_array()
                    .unwrap()
                    .iter()
                    .enumerate()
                {
                    let new_focus_state = if idx
                        != ws.get("containers").get("focused").as_i64().unwrap() as usize
                        || monitor_idx != focused_monitor_idx
                        || c.get("windows")
                            .get("elements")
                            .as_array()
                            .unwrap()
                            .get(c.get("windows").get("focused").as_u64().unwrap() as usize)
                            .map(|w| {
                                w.get("hwnd").as_i64().unwrap() as isize
                                    != foreground_hwnd.0 as isize
                            })
                            .unwrap_or_default()
                    {
                        WindowKind::Unfocused
                    } else if c.get("windows").get("elements").as_array().unwrap().len() > 1 {
                        WindowKind::Stack
                    } else {
                        WindowKind::Single
                    };

                    // Update the window kind for all containers on this workspace
                    {
                        let mut focus_state = focus_state_mutex.lock().unwrap();
                        let _ = focus_state.insert(
                            c.get("windows").get("elements").as_array().unwrap()
                                [c.get("windows").get("focused").as_u64().unwrap() as usize]
                                .get("hwnd")
                                .as_i64()
                                .unwrap() as isize,
                            new_focus_state,
                        );
                    }
                }
                {
                    for window in ws.get("floating_windows").as_array().unwrap() {
                        let mut new_focus_state = WindowKind::Unfocused;

                        if foreground_window.0 as isize
                            == window.get("hwnd").as_i64().unwrap() as isize
                        {
                            new_focus_state = WindowKind::Floating;
                        }

                        {
                            let mut focus_state = focus_state_mutex.lock().unwrap();
                            let _ = focus_state.insert(
                                window.get("hwnd").as_i64().unwrap() as isize,
                                new_focus_state,
                            );
                        }
                    }
                }
            }
        }

        let new_focus_state = focus_state_mutex.lock().unwrap();

        for (tracking, border) in APP_STATE.borders.lock().unwrap().iter() {
            let previous_window_kind = previous_focus_state.get(tracking);
            let new_window_kind = new_focus_state.get(tracking);

            // Only post update messages when the window kind has actually changed
            if previous_window_kind != new_window_kind {
                // If the window kinds were just Single and Unfocused, then we can just rely on
                // tacky-borders' internal logic to update border colors
                if matches!(
                    previous_window_kind,
                    Some(WindowKind::Single) | Some(WindowKind::Unfocused)
                ) && matches!(
                    new_window_kind,
                    Some(WindowKind::Single) | Some(WindowKind::Unfocused)
                ) {
                    continue;
                }

                let border_hwnd = HWND(*border as _);
                post_message_w(Some(border_hwnd), WM_APP_KOMOREBI, WPARAM(0), LPARAM(0))
                    .context("WM_APP_KOMOREBI")
                    .log_if_err();
            }
        }
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
