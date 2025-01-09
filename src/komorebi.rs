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
use windows::Win32::Networking::WinSock::{WSACleanup, WSAStartup, WSADATA};
use windows::Win32::System::Threading::CREATE_NO_WINDOW;
use windows::Win32::System::IO::OVERLAPPED_ENTRY;

use crate::colors::ColorConfig;
use crate::iocp::{CompletionPort, UnixDomainSocket};
use crate::iocp::{UnixListener, UnixStream};
use crate::utils::{get_foreground_window, post_message_w, LogIfErr, WM_APP_KOMOREBI};
use crate::APP_STATE;

const BUFFER_POOL_REFRESH_INTERVAL: time::Duration = time::Duration::from_secs(300);

#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
pub struct KomorebiColorsConfig {
    pub stack_color: ColorConfig,
    pub monocle_color: ColorConfig,
    pub floating_color: ColorConfig,
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

    // Frankly, this is just a really crappy, Windows-only, version of something like compio.
    // Good learning experience though.
    pub fn start(&mut self) -> anyhow::Result<()> {
        debug!("starting komorebic integration");

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
        port.associate_handle(listener.socket.to_handle())?;

        // Prevent overriding already existing sockets
        if self.listen_socket.is_none() {
            self.listen_socket = Some(listener.socket.clone());
        }

        // Get a reference to the mutex because I don't think we can pass 'self' into threads
        let focus_state = self.focus_state.clone();

        let _ = thread::spawn(move || {
            move || -> anyhow::Result<()> {
                let mut entries = [OVERLAPPED_ENTRY::default(); 8];
                let mut bufferpool = VecDeque::<Vec<u8>>::new();

                let mut accept_streams = HashMap::<usize, Box<UnixStream>>::new();
                let mut read_streams = HashMap::<usize, Box<UnixStream>>::new();

                // Queue up our first accept I/O operation.
                let stream = Box::new(listener.accept()?);
                port.associate_handle(stream.socket.to_handle())?;
                accept_streams.insert(listener.token(), stream);

                let mut now = time::Instant::now();

                loop {
                    // Clear the buffer pool after the refresh interval has elapsed
                    if now.elapsed() > BUFFER_POOL_REFRESH_INTERVAL {
                        debug!("cleaning up buffer pool for komorebic socket");
                        bufferpool.clear();
                        now = time::Instant::now();
                    }

                    // This will block until an I/O operation has completed (accept or read)
                    let num_removed = port.poll_many(None, &mut entries)?;

                    // Now we can iterate through the completed I/O operations
                    for entry in entries[..num_removed as usize].iter() {
                        if let Some(mut stream) = accept_streams.remove(&entry.lpCompletionKey) {
                            // Stream has been accepted; ready to read

                            // Attempt to retrieve a buffer from the bufferpool
                            // NOTE: we use unwrap_or_else() because it is lazily evaluated
                            let outputbuffer = bufferpool.pop_front().unwrap_or_else(|| {
                                debug!("creating new buffer for komorebic socket");
                                vec![0u8; 8192]
                            });
                            stream.read(outputbuffer)?;

                            // The stream has now begun its read operation, so place it in read_streams
                            read_streams.insert(stream.token(), stream);

                            // Queue up a new accept I/O operation.
                            let stream = Box::new(listener.accept()?);
                            port.associate_handle(stream.socket.to_handle())?;
                            accept_streams.insert(listener.token(), stream);
                        } else if let Some(mut stream) = read_streams.remove(&entry.lpCompletionKey)
                        {
                            // Stream has been read; ready to process

                            Self::process_komorebi_notification(
                                focus_state.clone(),
                                &stream.buffer,
                                entry.dwNumberOfBytesTransferred,
                            );

                            // We don't need this stream anymore, so place its buffer into the pool
                            bufferpool.push_back(stream.take_buffer());
                        } else {
                            error!("invalid completion key found");
                        }
                    }
                }
            }()
            .log_if_err();
        });

        // Subscribe to komorebic
        Command::new("komorebic")
            .arg("subscribe-socket")
            .arg(socket_file)
            .creation_flags(CREATE_NO_WINDOW.0)
            .spawn()
            .context("could not subscribe to komorebic socket")?;

        Ok(())
    }

    pub fn stop(&mut self) -> anyhow::Result<()> {
        // If this is Some, it means WinSock is (most likely) running, so we need to cleanup
        if self.listen_socket.is_some() {
            unsafe { WSACleanup() };

            // I assume setting this to None will drop the socket, which also calls closesocket()
            self.listen_socket = None;
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
    pub fn process_komorebi_notification(
        focus_state_mutex: Arc<Mutex<HashMap<isize, WindowKind>>>,
        buffer: &[u8],
        bytes_received: u32,
    ) {
        let notification: serde_json::Value =
            match serde_json::from_slice(&buffer[..bytes_received as usize]) {
                Ok(event) => event,
                Err(err) => {
                    error!("could not parse unix domain socket buffer: {err}");
                    return;
                }
            };

        let previous_focus_state = (*focus_state_mutex.lock().unwrap()).clone();

        // TODO: replace all the unwrap with unwrap_or_default later, rn we just do it to test. also
        // use filter() on some of the Option values
        let monitors = &notification["state"]["monitors"];
        let focused_monitor_idx = monitors["focused"].as_u64().unwrap() as usize;
        let foreground_window = get_foreground_window();

        for (monitor_idx, m) in monitors["elements"].as_array().unwrap().iter().enumerate() {
            // Only operate on the focused workspace of each monitor
            if let Some(ws) = m["workspaces"]["elements"]
                .as_array()
                .unwrap()
                .get(m["workspaces"]["focused"].as_u64().unwrap() as usize)
            {
                // Handle the monocle container separately
                if let Some(monocole) = ws.get("monocle_container").filter(|value| !value.is_null())
                {
                    let new_focus_state = if monitor_idx != focused_monitor_idx {
                        WindowKind::Unfocused
                    } else {
                        WindowKind::Monocle
                    };
                    {
                        let mut focus_state = focus_state_mutex.lock().unwrap();
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

                for (idx, c) in ws["containers"]["elements"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .enumerate()
                {
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
                        let mut focus_state = focus_state_mutex.lock().unwrap();
                        let _ = focus_state.insert(
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
                        let mut new_focus_state = WindowKind::Unfocused;

                        if foreground_window.0 as isize == window["hwnd"].as_i64().unwrap() as isize
                        {
                            new_focus_state = WindowKind::Floating;
                        }

                        {
                            let mut focus_state = focus_state_mutex.lock().unwrap();
                            let _ = focus_state
                                .insert(window["hwnd"].as_i64().unwrap() as isize, new_focus_state);
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
