use anyhow::{anyhow, Context};
use core::time;
use std::path::Path;
use std::{io, mem, ptr};
use windows::core::PSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Networking::WinSock::{
    bind, closesocket, listen, AcceptEx, WSARecv, WSASocketW, ADDRESS_FAMILY, AF_UNIX, SOCKADDR,
    SOCKADDR_UN, SOCKET, SOCKET_ERROR, SOCK_STREAM, SOMAXCONN, WSABUF, WSA_FLAG_OVERLAPPED,
    WSA_IO_PENDING,
};
use windows::Win32::System::Threading::INFINITE;
use windows::Win32::System::IO::{
    CreateIoCompletionPort, GetQueuedCompletionStatus, GetQueuedCompletionStatusEx, OVERLAPPED,
    OVERLAPPED_ENTRY,
};

use crate::utils::LogIfErr;

const UNIX_ADDR_LEN: u32 = mem::size_of::<SOCKADDR_UN>() as u32;

#[allow(unused)]
pub struct UnixListener {
    pub socket: UnixDomainSocket,
    pub buffer: Vec<u8>,
    pub overlapped: Box<OVERLAPPED>,
    pub flags: u32,
}

unsafe impl Send for UnixListener {}
unsafe impl Sync for UnixListener {}

#[allow(unused)]
impl UnixListener {
    pub fn bind(socket_path: &Path) -> anyhow::Result<Self> {
        let server_socket = UnixDomainSocket::new()?;
        server_socket.bind(socket_path)?;

        Ok(Self {
            socket: server_socket,
            buffer: Vec::new(),
            overlapped: Box::new(OVERLAPPED::default()),
            flags: 0,
        })
    }

    pub fn listen(&self) -> anyhow::Result<()> {
        self.socket.listen(SOMAXCONN as i32)
    }

    pub fn accept(&mut self) -> anyhow::Result<UnixStream> {
        // I'm not 100% sure why we need at least this Vec len, but it's just double the len used
        // in AcceptEx (double I assume because there's both the local and remote addresses)
        let mut client_stream = UnixStream {
            socket: UnixDomainSocket::default(),
            buffer: vec![0u8; ((UNIX_ADDR_LEN + 16) * 2) as usize],
            overlapped: Box::new(OVERLAPPED::default()),
            flags: 0,
        };

        client_stream.socket = self
            .socket
            .accept(&mut client_stream.buffer, client_stream.overlapped.as_mut())?;

        Ok(client_stream)
    }

    pub fn token(&self) -> usize {
        self.socket.0 .0
    }

    pub fn take_buffer(&mut self) -> Vec<u8> {
        mem::take(&mut self.buffer)
    }
}

impl Drop for UnixListener {
    fn drop(&mut self) {
        unsafe { closesocket(self.socket.0) };
    }
}

pub struct UnixStream {
    pub socket: UnixDomainSocket,
    pub buffer: Vec<u8>,
    // I'm not sure if I need the Box, but I'll keep in just in case because I don't if
    // GetQueuedCompletionStatus can get the OVERLAPPED pointers if the structs move in memory.
    pub overlapped: Box<OVERLAPPED>,
    pub flags: u32,
}

unsafe impl Send for UnixStream {}
unsafe impl Sync for UnixStream {}

#[allow(unused)]
impl UnixStream {
    pub fn read(&mut self, outputbuffer: Vec<u8>) -> anyhow::Result<()> {
        // Take ownership of this output buffer
        self.buffer = outputbuffer;

        self.socket
            .read(&mut self.buffer, self.overlapped.as_mut(), &mut self.flags)
    }

    pub fn token(&self) -> usize {
        self.socket.0 .0
    }

    pub fn take_buffer(&mut self) -> Vec<u8> {
        mem::take(&mut self.buffer)
    }
}

impl Drop for UnixStream {
    fn drop(&mut self) {
        unsafe { closesocket(self.socket.0) };
    }
}

#[derive(Debug, Default, Clone)]
pub struct UnixDomainSocket(pub SOCKET);

unsafe impl Send for UnixDomainSocket {}
unsafe impl Sync for UnixDomainSocket {}

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
        let mut sun_path = [0i8; 108];
        let path_bytes = path.as_os_str().as_encoded_bytes();

        if path_bytes.len() > sun_path.len() {
            return Err(anyhow!("socket path is too long"));
        }

        for (i, byte) in path_bytes.iter().enumerate() {
            sun_path[i] = *byte as i8;
        }

        let sockaddr_un = SOCKADDR_UN {
            sun_family: ADDRESS_FAMILY(AF_UNIX),
            sun_path,
        };

        let iresult = unsafe {
            bind(
                self.0,
                ptr::addr_of!(sockaddr_un) as *const SOCKADDR,
                mem::size_of_val(&sockaddr_un) as i32,
            )
        };
        if iresult == SOCKET_ERROR {
            let last_error = io::Error::last_os_error();
            unsafe { closesocket(self.0) };
            return Err(anyhow!("could not bind socket: {:?}", last_error));
        }

        Ok(())
    }

    pub fn listen(&self, backlog: i32) -> anyhow::Result<()> {
        if unsafe { listen(self.0, backlog) } == SOCKET_ERROR {
            let last_error = io::Error::last_os_error();
            unsafe { closesocket(self.0) };
            return Err(anyhow!("could not listen to socket: {:?}", last_error));
        }

        Ok(())
    }

    pub fn accept(
        &mut self,
        lpoutputbuffer: &mut [u8],
        lpoverlapped: &mut OVERLAPPED,
    ) -> anyhow::Result<UnixDomainSocket> {
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
                unsafe {
                    closesocket(self.0);
                }
                return Err(anyhow!(
                    "could not accept client socket connection: {:?}",
                    last_error
                ));
            }
        };

        Ok(client_socket)
    }

    pub fn read(
        &mut self,
        lpoutputbuffer: &mut [u8],
        lpoverlapped: &mut OVERLAPPED,
        lpflags: &mut u32,
    ) -> anyhow::Result<()> {
        let lpbuffers = WSABUF {
            len: lpoutputbuffer.len() as u32,
            buf: PSTR(lpoutputbuffer.as_mut_ptr()),
        };

        let iresult = unsafe {
            WSARecv(
                self.0,
                &[lpbuffers],
                None,
                lpflags,
                Some(lpoverlapped),
                None,
            )
        };

        if iresult == SOCKET_ERROR {
            let last_error = io::Error::last_os_error();

            if last_error.raw_os_error() != Some(WSA_IO_PENDING.0) {
                unsafe { closesocket(self.0) };
                return Err(anyhow!("could not receive data: {:?}", last_error));
            }
        }

        Ok(())
    }

    pub fn to_handle(&self) -> HANDLE {
        HANDLE(self.0 .0 as _)
    }
}

impl From<UnixDomainSocket> for HANDLE {
    fn from(value: UnixDomainSocket) -> Self {
        Self(value.0 .0 as _)
    }
}

#[derive(Debug, Default)]
pub struct CompletionPort {
    iocp_handle: HANDLE,
}

unsafe impl Send for CompletionPort {}
unsafe impl Sync for CompletionPort {}

#[allow(unused)]
impl CompletionPort {
    pub fn new(threads: u32) -> anyhow::Result<Self> {
        let iocp_handle =
            match unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, None, 0, threads) } {
                Ok(handle) => handle,
                Err(err) => {
                    return Err(anyhow!("could not create iocp: {err}"));
                }
            };

        Ok(Self { iocp_handle })
    }

    pub fn associate_handle(&self, handle: HANDLE, token: usize) -> anyhow::Result<()> {
        // This just returns the HANDLE of the existing iocp, so we can ignore the return value
        let _ = unsafe { CreateIoCompletionPort(handle, Some(self.iocp_handle), token, 0) }
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

        unsafe {
            GetQueuedCompletionStatus(
                self.iocp_handle,
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
                self.iocp_handle,
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
        unsafe { CloseHandle(self.iocp_handle) }.log_if_err();
    }
}
