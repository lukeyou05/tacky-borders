use anyhow::{Context, anyhow};
use std::collections::VecDeque;
use std::path::Path;
use std::thread::{self, JoinHandle};
use std::time;
use std::{io, mem, ptr};
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Networking::WinSock::{
    ADDRESS_FAMILY, AF_UNIX, AcceptEx, INVALID_SOCKET, SEND_RECV_FLAGS, SOCK_STREAM, SOCKADDR,
    SOCKADDR_UN, SOCKET, SOCKET_ERROR, SOMAXCONN, WSA_FLAG_OVERLAPPED, WSA_IO_PENDING, WSABUF,
    WSACleanup, WSADATA, WSAGetLastError, WSARecv, WSASend, WSASocketW, WSAStartup, accept, bind,
    closesocket, connect, listen, recv, send,
};
use windows::Win32::System::IO::{
    CancelIoEx, CreateIoCompletionPort, GetQueuedCompletionStatus, GetQueuedCompletionStatusEx,
    OVERLAPPED, OVERLAPPED_ENTRY, PostQueuedCompletionStatus,
};
use windows::Win32::System::Threading::INFINITE;
use windows::core::PSTR;

use crate::utils::LogIfErr;

const UNIX_ADDR_LEN: u32 = mem::size_of::<SOCKADDR_UN>() as u32;

pub struct UnixListener {
    pub socket: UnixDomainSocket,
    pub overlapped: Option<Box<OVERLAPPED>>,
}

unsafe impl Send for UnixListener {}

impl UnixListener {
    pub fn bind(socket_path: &Path) -> anyhow::Result<Self> {
        let server_socket = UnixDomainSocket::new()?;
        server_socket.bind(socket_path)?;
        server_socket.listen(SOMAXCONN as i32)?;

        Ok(Self {
            socket: server_socket,
            overlapped: None,
        })
    }

    /// NOTE: The returned `UnixStream` inherits the overlapped mode of this `UnixListener`
    pub fn accept(&self) -> anyhow::Result<UnixStream> {
        if self.overlapped.is_some() {
            // I'm not 100% sure why we need at least this Vec len, but it's just double the len used
            // in AcceptEx (double I assume because there's both the local and remote addresses)
            let mut client_buffer = vec![0u8; ((UNIX_ADDR_LEN + 16) * 2) as usize];
            let mut client_overlapped = Box::new(OVERLAPPED::default());
            let client_socket = self
                .socket
                .accept_overlapped(&mut client_buffer, client_overlapped.as_mut())?;

            Ok(UnixStream {
                socket: client_socket,
                buffer: client_buffer,
                overlapped: Some(client_overlapped),
                flags: 0,
            })
        } else {
            let client_socket = self.socket.accept(None, None)?;

            Ok(UnixStream {
                socket: client_socket,
                buffer: Vec::new(),
                overlapped: None,
                flags: 0,
            })
        }
    }

    pub fn token(&self) -> usize {
        self.socket.0.0
    }
}

pub struct UnixStream {
    pub socket: UnixDomainSocket,
    pub buffer: Vec<u8>,
    pub overlapped: Option<Box<OVERLAPPED>>,
    pub flags: u32,
}

unsafe impl Send for UnixStream {}

impl UnixStream {
    pub fn connect(path: &Path) -> anyhow::Result<Self> {
        let client_socket = UnixDomainSocket::new()?;
        client_socket.connect(path)?;

        Ok(Self {
            socket: client_socket,
            buffer: Vec::new(),
            overlapped: None,
            flags: 0,
        })
    }

    /// NOTE: This takes ownership of the input buffer to avoid race conditions
    pub fn read(&mut self, outputbuffer: Vec<u8>) -> anyhow::Result<u32> {
        // Reset flags between I/O operations
        self.flags = 0;

        // Here is where we take ownership of the buffer
        self.buffer = outputbuffer;

        if let Some(overlapped) = self.overlapped.as_mut() {
            self.socket
                .read_overlapped(&mut self.buffer, overlapped.as_mut(), &mut self.flags)
        } else {
            self.socket
                .read(&mut self.buffer, SEND_RECV_FLAGS(self.flags as i32))
        }
    }

    pub fn write(&mut self, inputbuffer: &[u8]) -> anyhow::Result<u32> {
        // Reset flags between I/O operations
        self.flags = 0;

        if let Some(overlapped) = self.overlapped.as_mut() {
            self.socket
                .write_overlapped(inputbuffer, overlapped.as_mut(), self.flags)
        } else {
            self.socket
                .write(inputbuffer, SEND_RECV_FLAGS(self.flags as i32))
        }
    }

    pub fn token(&self) -> usize {
        self.socket.0.0
    }

    pub fn take_buffer(&mut self) -> Vec<u8> {
        mem::take(&mut self.buffer)
    }
}

#[derive(Debug)]
pub struct UnixDomainSocket(SOCKET);

impl UnixDomainSocket {
    pub fn new() -> anyhow::Result<Self> {
        let socket = unsafe {
            WSASocketW(
                AF_UNIX as i32,
                SOCK_STREAM.0,
                0,
                None,
                0,
                WSA_FLAG_OVERLAPPED,
            )
        }?;

        Ok(Self(socket))
    }

    pub fn bind(&self, path: &Path) -> anyhow::Result<()> {
        let sockaddr_un = sockaddr_un(path)?;

        let iresult = unsafe {
            bind(
                self.0,
                ptr::addr_of!(sockaddr_un) as *const SOCKADDR,
                mem::size_of_val(&sockaddr_un) as i32,
            )
        };
        if iresult == SOCKET_ERROR {
            let last_error = io::Error::last_os_error();
            return Err(anyhow!("could not bind socket: {:?}", last_error));
        }

        Ok(())
    }

    pub fn connect(&self, path: &Path) -> anyhow::Result<()> {
        let sockaddr_un = sockaddr_un(path)?;

        if unsafe {
            connect(
                self.0,
                ptr::addr_of!(sockaddr_un) as *const SOCKADDR,
                mem::size_of_val(&sockaddr_un) as i32,
            )
        } == SOCKET_ERROR
        {
            let last_error = io::Error::last_os_error();
            return Err(anyhow!("could not connect to socket: {:?}", last_error));
        }

        Ok(())
    }

    pub fn listen(&self, backlog: i32) -> anyhow::Result<()> {
        if unsafe { listen(self.0, backlog) } == SOCKET_ERROR {
            let last_error = io::Error::last_os_error();
            return Err(anyhow!("could not listen to socket: {:?}", last_error));
        }

        Ok(())
    }

    pub fn accept(
        &self,
        addr: Option<&mut SOCKADDR>,
        addrlen: Option<&mut i32>,
    ) -> anyhow::Result<UnixDomainSocket> {
        let socket = unsafe {
            accept(
                self.0,
                addr.map(|mut_ref| mut_ref as *mut _),
                addrlen.map(|mut_ref| mut_ref as *mut _),
            )
        }
        .context("could not accept a client socket")?;

        Ok(UnixDomainSocket(socket))
    }

    pub fn accept_overlapped(
        &self,
        lpoutputbuffer: &mut [u8],
        lpoverlapped: &mut OVERLAPPED,
    ) -> anyhow::Result<UnixDomainSocket> {
        // Zero out unused OVERLAPPED struct fields (as per MSDN recommendation)
        *lpoverlapped = OVERLAPPED {
            hEvent: lpoverlapped.hEvent,
            ..Default::default()
        };

        let client_socket = UnixDomainSocket::new()?;
        let mut bytes_received = 0u32;

        if !unsafe {
            AcceptEx(
                self.0,
                client_socket.0,
                lpoutputbuffer as *mut _ as *mut _,
                0,
                // We add 16 to the address length because MSDN says so
                UNIX_ADDR_LEN + 16,
                UNIX_ADDR_LEN + 16,
                &mut bytes_received,
                lpoverlapped,
            )
        }
        .as_bool()
        {
            let last_error = io::Error::last_os_error();

            if last_error.raw_os_error() != Some(WSA_IO_PENDING.0) {
                return Err(anyhow!(
                    "could not accept a client socket: {:?}",
                    last_error
                ));
            }
        };

        Ok(client_socket)
    }

    pub fn read(&self, buf: &mut [u8], flags: SEND_RECV_FLAGS) -> anyhow::Result<u32> {
        let bytes_transferred = unsafe { recv(self.0, buf, flags) };

        if bytes_transferred == SOCKET_ERROR {
            let last_error = io::Error::last_os_error();
            return Err(anyhow!("could not receive data: {:?}", last_error));
        }

        Ok(bytes_transferred as u32)
    }

    pub fn read_overlapped(
        &self,
        lpoutputbuffer: &mut [u8],
        lpoverlapped: &mut OVERLAPPED,
        lpflags: &mut u32,
    ) -> anyhow::Result<u32> {
        // Zero out unused OVERLAPPED struct fields (as per MSDN recommendation)
        *lpoverlapped = OVERLAPPED {
            hEvent: lpoverlapped.hEvent,
            ..Default::default()
        };

        let lpbuffers = WSABUF {
            len: lpoutputbuffer.len() as u32,
            buf: PSTR(lpoutputbuffer.as_mut_ptr()),
        };
        let mut bytes_transferred = 0;

        // Note that we set lpnumberofbytesrecvd to a non-null pointer even though MSDN recommends
        // setting it to null when lpoverlapped is non-null. We do this anyways because the field
        // is still updated if the operation completes immediately, allowing us to indicate so.
        let iresult = unsafe {
            WSARecv(
                self.0,
                &[lpbuffers],
                Some(&mut bytes_transferred),
                lpflags,
                Some(lpoverlapped),
                None,
            )
        };

        if iresult == SOCKET_ERROR {
            let last_error = io::Error::last_os_error();

            if last_error.raw_os_error() != Some(WSA_IO_PENDING.0) {
                return Err(anyhow!("could not receive data: {:?}", last_error));
            }
        }

        Ok(bytes_transferred)
    }

    pub fn write(&self, buf: &[u8], flags: SEND_RECV_FLAGS) -> anyhow::Result<u32> {
        let bytes_transferred = unsafe { send(self.0, buf, flags) };

        if bytes_transferred == SOCKET_ERROR {
            let last_error = io::Error::last_os_error();
            return Err(anyhow!("could not write data: {:?}", last_error));
        }

        Ok(bytes_transferred as u32)
    }

    pub fn write_overlapped(
        &self,
        lpinputbuffer: &[u8],
        lpoverlapped: &mut OVERLAPPED,
        lpflags: u32,
    ) -> anyhow::Result<u32> {
        // Zero out unused OVERLAPPED struct fields (as per MSDN recommendation)
        *lpoverlapped = OVERLAPPED {
            hEvent: lpoverlapped.hEvent,
            ..Default::default()
        };

        // WSABUF requires a mut ptr to the buffer, but WSASend shouldn't mutate anything.
        // It should be safe to cast a const ptr to a mut ptr as a workaround.
        let lpbuffers = WSABUF {
            len: lpinputbuffer.len() as u32,
            buf: PSTR(lpinputbuffer.as_ptr() as *mut _),
        };
        let mut bytes_transferred = 0;

        // Note that we set lpnumberofbytessent to a non-null pointer even though MSDN recommends
        // setting it to null when lpoverlapped is non-null. We do this anyways because the field
        // is still updated if the operation completes immediately, allowing us to indicate so.
        let iresult = unsafe {
            WSASend(
                self.0,
                &[lpbuffers],
                Some(&mut bytes_transferred),
                lpflags,
                Some(lpoverlapped),
                None,
            )
        };

        if iresult == SOCKET_ERROR {
            let last_error = io::Error::last_os_error();

            if last_error.raw_os_error() != Some(WSA_IO_PENDING.0) {
                return Err(anyhow!("could not write data: {:?}", last_error));
            }
        }

        Ok(bytes_transferred)
    }

    pub fn to_handle(&self) -> HANDLE {
        HANDLE(self.0.0 as _)
    }
}

pub trait OverlappedMode {
    /// # Safety
    ///
    /// Calling this function is not inherently unsafe, but enabling overlapped (asynchronous) I/O
    /// may have additional safety requirements depending on the design of the trait implementor:
    ///
    /// - Each I/O operation requires a separate `OVERLAPPED` struct. If a trait implementor owns
    ///   one or more `OVERLAPPED` structs (e.g. `UnixStream` and `UnixListener`), it must never
    ///   run more concurrent operations than the number of `OVERLAPPED` structs it owns.
    /// - An `OVERLAPPED` struct should not be dropped until its corresponding I/O operation has
    ///   completed. Dropping it early can lead to use-after-free errors.
    unsafe fn set_overlapped(&mut self, enabled: bool);
}

impl OverlappedMode for UnixListener {
    unsafe fn set_overlapped(&mut self, enabled: bool) {
        if enabled && self.overlapped.is_none() {
            self.overlapped = Some(Box::new(OVERLAPPED::default()));
        } else {
            self.overlapped = None;
        }
    }
}

impl OverlappedMode for UnixStream {
    unsafe fn set_overlapped(&mut self, enabled: bool) {
        if enabled && self.overlapped.is_none() {
            self.overlapped = Some(Box::new(OVERLAPPED::default()));
        } else {
            self.overlapped = None;
        }
    }
}

impl Drop for UnixDomainSocket {
    fn drop(&mut self) {
        let iresult = unsafe { closesocket(self.0) };
        if iresult != 0 {
            error!(
                "could not close unix domain socket {:?}: {:?}",
                self.0,
                unsafe { WSAGetLastError() }
            )
        }
    }
}

fn sockaddr_un(path: &Path) -> anyhow::Result<SOCKADDR_UN> {
    let mut sun_path = [0i8; 108];
    let path_bytes = path.as_os_str().as_encoded_bytes();

    if path_bytes.len() > sun_path.len() {
        return Err(anyhow!("socket path is too long"));
    }

    for (i, byte) in path_bytes.iter().enumerate() {
        sun_path[i] = *byte as i8;
    }

    Ok(SOCKADDR_UN {
        sun_family: ADDRESS_FAMILY(AF_UNIX),
        sun_path,
    })
}

#[derive(Debug)]
pub struct CompletionPort(HANDLE);

unsafe impl Send for CompletionPort {}

impl CompletionPort {
    pub fn new(threads: u32) -> anyhow::Result<Self> {
        let iocp_handle =
            match unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, None, 0, threads) } {
                Ok(handle) => handle,
                Err(err) => {
                    return Err(anyhow!("could not create iocp: {err}"));
                }
            };

        Ok(Self(iocp_handle))
    }

    pub fn associate_handle(&self, handle: HANDLE, token: usize) -> anyhow::Result<()> {
        // This just returns the HANDLE of the existing iocp, so we can ignore the return value
        let _ = unsafe { CreateIoCompletionPort(handle, Some(self.0), token, 0) }
            .map_err(|err| anyhow!("could not add handle to iocp: {err}"))?;

        Ok(())
    }

    pub fn poll_single(
        &self,
        timeout: Option<time::Duration>,
        entry: &mut OVERLAPPED_ENTRY,
    ) -> anyhow::Result<()> {
        let mut bytes_transferred = 0u32;
        let mut completion_key = 0usize;
        let mut lpoverlapped: *mut OVERLAPPED = ptr::null_mut();

        let timeout_ms = match timeout {
            Some(duration) => duration.as_millis() as u32,
            None => INFINITE,
        };

        // TODO: Replace context() with with_context()
        unsafe {
            GetQueuedCompletionStatus(
                self.0,
                &mut bytes_transferred,
                &mut completion_key,
                &mut lpoverlapped,
                timeout_ms,
            )
        }
        .context(format!(
            "could not get queued completion status: {}",
            io::Error::last_os_error(),
        ))?;

        *entry = OVERLAPPED_ENTRY {
            lpCompletionKey: completion_key,
            lpOverlapped: lpoverlapped,
            Internal: 0,
            dwNumberOfBytesTransferred: bytes_transferred,
        };

        Ok(())
    }

    pub fn poll_many(
        &self,
        timeout: Option<time::Duration>,
        entries: &mut [OVERLAPPED_ENTRY],
    ) -> anyhow::Result<u32> {
        let mut num_entries_removed = 0u32;

        let timeout_ms = match timeout {
            Some(duration) => duration.as_millis() as u32,
            None => INFINITE,
        };

        unsafe {
            GetQueuedCompletionStatusEx(
                self.0,
                entries,
                &mut num_entries_removed,
                timeout_ms,
                false,
            )
        }
        .context(format!(
            "could not get queued completion status: {}",
            io::Error::last_os_error(),
        ))?;

        Ok(num_entries_removed)
    }
}

impl Drop for CompletionPort {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) }
            .with_context(|| format!("could not close i/o completion port {:?}", self.0))
            .log_if_err();
    }
}

// Like AsRawHandle, but specifically for windows-rs' HANDLE type
pub trait AsWin32Handle {
    fn as_win32_handle(&self) -> HANDLE;
}

impl AsWin32Handle for CompletionPort {
    fn as_win32_handle(&self) -> HANDLE {
        self.0
    }
}

pub trait AsWin32Socket {
    fn as_win32_socket(&self) -> SOCKET;
}

impl AsWin32Socket for UnixDomainSocket {
    fn as_win32_socket(&self) -> SOCKET {
        self.0
    }
}

pub struct UnixStreamSink {
    iocp_handle: HANDLE,
    thread_handle: Option<JoinHandle<()>>,
}

impl UnixStreamSink {
    const MAX_COMPLETION_EVENTS: usize = 8;
    const BUFFER_POOL_PRUNE_INTERVAL: time::Duration = time::Duration::from_secs(600);
    const BUFFER_SIZE: usize = 32768;

    // Currently, tokens/keys are just the values of the corresponding SOCKETs, which is why the value
    // below (INVALID_SOCKET) should work as a special key that won't interfere with others.
    const STOP_PACKET_KEY: usize = INVALID_SOCKET.0;

    pub fn new(
        socket_path: &Path,
        mut callback: impl FnMut(&[u8], u32) + Send + 'static,
    ) -> anyhow::Result<Self> {
        // Start the WinSock service
        // TODO: Use std::sync::Once to get this startup to run only once
        let iresult = unsafe { WSAStartup(0x202, &mut WSADATA::default()) };
        if iresult != 0 {
            return Err(anyhow!("WSAStartup failure: {iresult}"));
        }

        let mut listener = UnixListener::bind(socket_path)?;
        unsafe { listener.set_overlapped(true) };
        let listener_key = listener.token();

        let port = CompletionPort::new(2)?;
        port.associate_handle(listener.socket.to_handle(), listener_key)?;

        let iocp_handle = port.as_win32_handle();

        let thread_handle = thread::spawn(move || {
            debug!("entering unix stream sink thread");

            move || -> anyhow::Result<()> {
                let mut entries = vec![OVERLAPPED_ENTRY::default(); Self::MAX_COMPLETION_EVENTS];
                let mut buffer_pool = VecDeque::<Vec<u8>>::new();
                let mut streams_queue = VecDeque::<(usize, Box<UnixStream>)>::new();
                let mut last_buffer_pool_prune = time::Instant::now();

                // Queue up our first accept I/O operation.
                let stream = Box::new(listener.accept()?);
                port.associate_handle(stream.socket.to_handle(), stream.token())?;
                streams_queue.push_back((stream.token(), stream));

                let mut should_cleanup = false;

                loop {
                    if last_buffer_pool_prune.elapsed() > Self::BUFFER_POOL_PRUNE_INTERVAL {
                        debug!("pruning buffer pool for unix stream sink");
                        buffer_pool.truncate(1);
                        last_buffer_pool_prune = time::Instant::now();
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
                                debug!("creating new buffer for unix stream sink");
                                vec![0u8; Self::BUFFER_SIZE]
                            });
                            stream.read(outputbuffer)?;

                            // Queue up a new accept I/O operation.
                            let stream = Box::new(listener.accept()?);
                            port.associate_handle(stream.socket.to_handle(), stream.token())?;
                            streams_queue.push_back((stream.token(), stream));
                        } else if entry.lpCompletionKey != Self::STOP_PACKET_KEY {
                            // Stream has been read; ready to process
                            let position = streams_queue
                                .iter()
                                .position(|(token, _)| *token == entry.lpCompletionKey)
                                .context("could not find stream")?;
                            let mut stream = streams_queue
                                .remove(position)
                                .context("could not remove stream from queue")?
                                .1;

                            callback(&stream.buffer, entry.dwNumberOfBytesTransferred);

                            // We don't need this stream anymore, so place its buffer into the pool
                            buffer_pool.push_back(stream.take_buffer());
                        } else {
                            // Stop packet has been sent; cleanup and exit the thread
                            should_cleanup = true;
                        }
                    }

                    if should_cleanup {
                        Self::cleanup(listener, listener_key, port, entries, streams_queue)?;
                        break;
                    }
                }

                Ok(())
            }()
            .log_if_err();

            debug!("exiting unix stream sink thread");
        });

        Ok(Self {
            iocp_handle,
            thread_handle: Some(thread_handle),
        })
    }

    fn cleanup(
        listener: UnixListener,
        listener_key: usize,
        port: CompletionPort,
        mut entries: Vec<OVERLAPPED_ENTRY>,
        mut streams_queue: VecDeque<(usize, Box<UnixStream>)>,
    ) -> anyhow::Result<()> {
        // Cancel any pending I/O operations on the listener
        let listener_handle = listener.socket.to_handle();
        unsafe { CancelIoEx(listener_handle, None) }
            .with_context(|| format!("could not cancel i/o for listener {listener_handle:?}"))
            .log_if_err();

        // Cancel any pending I/O operations on each stream
        // NOTE: A stream may not have any pending I/O operations if it is still in
        // the accept stage, and CancelIoEx will return an error in those cases.
        for (_, stream) in streams_queue.iter() {
            let stream_handle = stream.socket.to_handle();
            unsafe { CancelIoEx(stream_handle, None) }
                .with_context(|| format!("could not cancel i/o for stream {stream_handle:?}"))
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
    }
}

impl Drop for UnixStreamSink {
    fn drop(&mut self) {
        let post_res =
            unsafe { PostQueuedCompletionStatus(self.iocp_handle, 0, Self::STOP_PACKET_KEY, None) };

        match post_res {
            Ok(()) => match self.thread_handle.take() {
                Some(handle) => {
                    if let Err(err) = handle.join() {
                        error!("could not join unix stream sink thread handle: {err:?}");
                    }
                }
                None => error!("could not take unix stream sink thread handle"),
            },
            Err(err) => error!(
                "could not post stop packet to iocp {:?} for unix stream sink: {err}",
                self.iocp_handle
            ),
        }

        unsafe { WSACleanup() };
    }
}

pub fn write_to_unix_socket(socket_path: &Path, message: &mut [u8]) -> anyhow::Result<()> {
    let mut stream = UnixStream::connect(socket_path)?;
    stream.write(message)?;

    Ok(())
}
