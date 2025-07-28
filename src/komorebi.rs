use anyhow::{Context, anyhow};
use dirs::home_dir;
use serde::Deserialize;
use std::collections::{HashMap, VecDeque};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::{fs, thread, time};
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, WPARAM};
use windows::Win32::Networking::WinSock::{INVALID_SOCKET, WSACleanup, WSADATA, WSAStartup};
use windows::Win32::System::IO::{CancelIoEx, OVERLAPPED_ENTRY, PostQueuedCompletionStatus};
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

use crate::APP_STATE;
use crate::colors::ColorBrushConfig;
use crate::config::serde_default_bool;
use crate::iocp::{AsWin32Handle, CompletionPort, UnixListener, UnixStream};
use crate::utils::{LogIfErr, WM_APP_KOMOREBI, get_foreground_window, is_window, post_message_w};

const BUFFER_POOL_PRUNE_INTERVAL: time::Duration = time::Duration::from_secs(600);
const BUFFER_SIZE: usize = 32768;
const FOCUS_STATE_PRUNE_INTERVAL: time::Duration = time::Duration::from_secs(600);

// Currently, tokens/keys are just the values of the corresponding SOCKETs, which is why the value
// below (INVALID_SOCKET) should work as a special key that won't interfere with others.
const STOP_PACKET_KEY: usize = INVALID_SOCKET.0;

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
    iocp_handle: HANDLE,
    thread_handle: Option<JoinHandle<()>>,
}

impl KomorebiIntegration {
    pub fn new() -> anyhow::Result<Self> {
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

        let listener = UnixListener::bind(&socket_path)?;
        let listener_key = listener.token();

        let port = CompletionPort::new(2)?;
        port.associate_handle(listener.socket.to_handle(), listener_key)?;

        let iocp_handle = port.as_win32_handle();

        let focus_state = Arc::new(Mutex::new(HashMap::new()));
        let focus_state_clone = focus_state.clone();

        // Attempt to subscribe to komorebic
        if !Command::new("komorebic")
            .arg("subscribe-socket")
            .arg(socket_file)
            .creation_flags(CREATE_NO_WINDOW.0)
            .status()
            .context("could not get komorebic subscribe-socket exit status")?
            .success()
        {
            return Err(anyhow!("could not subscribe to komorebic socket"));
        }

        let thread_handle = thread::spawn(move || {
            debug!("starting komorebic integration");

            move || -> anyhow::Result<()> {
                let mut entries = vec![OVERLAPPED_ENTRY::default(); 8];
                let mut buffer_pool = VecDeque::<Vec<u8>>::new();
                let mut streams_queue = VecDeque::<(usize, Box<UnixStream>)>::new();

                // Queue up our first accept I/O operation.
                let stream = Box::new(listener.accept()?);
                port.associate_handle(stream.socket.to_handle(), stream.token())?;
                streams_queue.push_back((stream.token(), stream));

                let mut last_buffer_pool_prune = time::Instant::now();
                let mut last_focus_state_prune = time::Instant::now();

                let mut should_cleanup = false;

                loop {
                    if last_buffer_pool_prune.elapsed() > BUFFER_POOL_PRUNE_INTERVAL {
                        debug!("pruning buffer pool for komorebi integration");
                        buffer_pool.truncate(1);
                        last_buffer_pool_prune = time::Instant::now();
                    }
                    if last_focus_state_prune.elapsed() > FOCUS_STATE_PRUNE_INTERVAL {
                        debug!("pruning focus state for komorebi integration");
                        focus_state_clone
                            .lock()
                            .unwrap()
                            .retain(|&hwnd_isize, _| is_window(Some(HWND(hwnd_isize as _))));
                        last_focus_state_prune = time::Instant::now();
                    }

                    // This will block until an I/O operation has completed
                    let num_removed = port.poll_many(None, &mut entries)?;

                    for entry in entries[..num_removed as usize].iter() {
                        if entry.lpCompletionKey == listener_key {
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
                        } else if entry.lpCompletionKey != STOP_PACKET_KEY {
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
                                &focus_state_clone,
                                &stream.buffer,
                                entry.dwNumberOfBytesTransferred,
                            );

                            // We don't need this stream anymore, so place its buffer into the pool
                            buffer_pool.push_back(stream.take_buffer());
                        } else {
                            // Stop packet has been sent; cleanup and exit the thread
                            should_cleanup = true;
                        }
                    }

                    if should_cleanup {
                        // Cleanup outside the loop
                        break;
                    }
                }

                // Cancel any pending I/O operations on the listener
                let listener_handle = listener.socket.to_handle();
                unsafe { CancelIoEx(listener_handle, None) }
                    .with_context(|| {
                        format!("could not cancel i/o for listener {listener_handle:?}")
                    })
                    .log_if_err();

                // Cancel any pending I/O operations on each stream
                // NOTE: A stream may not have any pending I/O operations if it is still in
                // the accept stage, and CancelIoEx will return an error in those cases.
                for (_, stream) in streams_queue.iter() {
                    let stream_handle = stream.socket.to_handle();
                    unsafe { CancelIoEx(stream_handle, None) }
                        .with_context(|| {
                            format!("could not cancel i/o for stream {stream_handle:?}")
                        })
                        .log_if_err();
                }

                // MSDN states that we must wait for I/O operations to complete (even if canceled)
                // before dropping OVERLAPPED structs to avoid use-after-free, so we'll wait below.
                while !streams_queue.is_empty() {
                    // NOTE: poll_many() should return an error after the timeout.
                    let timeout = time::Duration::from_secs(1);
                    let num_removed = port.poll_many(Some(timeout), &mut entries)?;

                    for entry in entries[..num_removed as usize].iter() {
                        if entry.lpCompletionKey == listener_key {
                            let _ = streams_queue.pop_back();
                        } else {
                            let position = streams_queue
                                .iter()
                                .position(|(token, _)| *token == entry.lpCompletionKey)
                                .context("could not find completion key")?;
                            let _ = streams_queue.remove(position);
                        }
                    }
                }

                Ok(())
            }()
            .log_if_err();

            debug!("exiting komorebi integration thread");
        });

        Ok(Self {
            focus_state,
            iocp_handle,
            thread_handle: Some(thread_handle),
        })
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
        focus_state_mutex: &Arc<Mutex<HashMap<isize, WindowKind>>>,
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
                        // If this is a monocole, I assume there's only 1 window in "windows"
                        let tracking_hwnd =
                            monocle.get("windows").get("elements").as_array().unwrap()[0]
                                .get("hwnd")
                                .as_i64()
                                .unwrap() as isize;
                        let mut focus_state = focus_state_mutex.lock().unwrap();
                        let _ = focus_state.insert(tracking_hwnd, new_focus_state);
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
                        let tracking_hwnd = c.get("windows").get("elements").as_array().unwrap()
                            [c.get("windows").get("focused").as_u64().unwrap() as usize]
                            .get("hwnd")
                            .as_i64()
                            .unwrap() as isize;
                        let mut focus_state = focus_state_mutex.lock().unwrap();
                        let _ = focus_state.insert(tracking_hwnd, new_focus_state);
                    }
                }
                {
                    for window in ws
                        .get("floating_windows")
                        .get("elements")
                        .as_array()
                        .unwrap()
                    {
                        let mut new_focus_state = WindowKind::Unfocused;

                        if foreground_window.0 as isize
                            == window.get("hwnd").as_i64().unwrap() as isize
                        {
                            new_focus_state = WindowKind::Floating;
                        }

                        {
                            let tracking_hwnd = window.get("hwnd").as_i64().unwrap() as isize;
                            let mut focus_state = focus_state_mutex.lock().unwrap();
                            let _ = focus_state.insert(tracking_hwnd, new_focus_state);
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

impl Drop for KomorebiIntegration {
    fn drop(&mut self) {
        debug!("stopping komorebi integration");

        let post_res =
            unsafe { PostQueuedCompletionStatus(self.iocp_handle, 0, STOP_PACKET_KEY, None) };

        match post_res {
            Ok(()) => match self.thread_handle.take() {
                Some(handle) => {
                    if let Err(err) = handle.join() {
                        error!("could not join komorebi integration thread handle: {err:?}");
                    }
                }
                None => error!("could not take komorebi integration thread handle"),
            },
            Err(err) => error!("could not post stop packet to komorebi integration: {err}"),
        }

        unsafe { WSACleanup() };
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
