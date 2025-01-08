use anyhow::{anyhow, Context};
use core::time;
use std::path::Path;
use std::{io, mem, ptr};
use windows::core::PSTR;
use windows::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Networking::WinSock::{
    bind, closesocket, listen, AcceptEx, WSACleanup, WSAGetLastError, WSARecv, WSASocketW,
    WSAStartup, ADDRESS_FAMILY, AF_UNIX, SOCKADDR, SOCKADDR_UN, SOCKET, SOCKET_ERROR, SOCK_STREAM,
    SOMAXCONN, WSABUF, WSADATA, WSA_FLAG_OVERLAPPED, WSA_IO_PENDING,
};
use windows::Win32::System::Threading::INFINITE;
use windows::Win32::System::IO::{
    CreateIoCompletionPort, GetQueuedCompletionStatus, OVERLAPPED, OVERLAPPED_ENTRY,
};

pub struct UnixListener {
    pub socket: UnixDomainSocket,
}

impl UnixListener {
    pub fn bind(socket_path: &Path) -> anyhow::Result<Self> {
        let iresult = unsafe { WSAStartup(0x202, &mut WSADATA::default()) };
        if iresult != 0 {
            return Err(anyhow!("WSAStartup failure: {iresult}"));
        }

        let server_socket = UnixDomainSocket::new()?;
        server_socket.bind(socket_path)?;

        Ok(Self {
            socket: server_socket,
        })
    }

    pub fn listen(&self) -> anyhow::Result<()> {
        self.socket.listen()
    }

    pub fn accept(&mut self) -> anyhow::Result<UnixStream> {
        // TODO: why can i pass OVERLAPPED::default() here, but not in AcceptEx itself?
        let client_socket = self.socket.accept(&mut OVERLAPPED::default())?;

        Ok(UnixStream {
            socket: client_socket,
        })
    }

    pub fn shutdown(&self) {
        unsafe {
            closesocket(self.socket.0);
            WSACleanup();
        };
    }
}

unsafe impl Send for UnixListener {}
unsafe impl Sync for UnixListener {}

impl Drop for UnixListener {
    fn drop(&mut self) {
        unsafe {
            closesocket(self.socket.0);
            WSACleanup();
        };
    }
}

pub struct UnixStream {
    pub socket: UnixDomainSocket,
}

unsafe impl Send for UnixStream {}
unsafe impl Sync for UnixStream {}

impl UnixStream {
    pub fn read(&mut self, lpoutputbuffer: &mut [u8]) -> anyhow::Result<()> {
        self.socket
            .read(lpoutputbuffer, &mut OVERLAPPED::default(), &mut 0)
    }
}

impl Drop for UnixStream {
    fn drop(&mut self) {
        unsafe { closesocket(self.socket.0) };
    }
}

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

    pub fn listen(&self) -> anyhow::Result<()> {
        if unsafe { listen(self.0, SOMAXCONN as i32) } == SOCKET_ERROR {
            let last_error = io::Error::last_os_error();
            unsafe { closesocket(self.0) };
            return Err(anyhow!("could not listen to socket: {:?}", last_error));
        }

        Ok(())
    }

    pub fn accept(&mut self, lpoverlapped: &mut OVERLAPPED) -> anyhow::Result<UnixDomainSocket> {
        let client_socket = UnixDomainSocket::new()?;
        let mut bytes_received = 0u32;

        let unix_addr_len = mem::size_of_val(&SOCKADDR_UN::default().sun_path) as u32;

        if !unsafe {
            AcceptEx(
                self.0,
                client_socket.0,
                // We choose not to receive any data here, so buffer can be whatever
                &mut [0u8; 1] as *mut _ as *mut _,
                0,
                // We add 16 to the address length because MSDN says so
                unix_addr_len + 16,
                unix_addr_len + 16,
                &mut bytes_received,
                lpoverlapped,
            )
        }
        .as_bool()
        {
            // TODO: get_queued_completion_status crashes when i use io::Error::last_os_error()
            let last_error = unsafe { WSAGetLastError() };

            // WSA_IO_PENDING just means it will complete at a later time (async)
            if last_error != WSA_IO_PENDING {
                unsafe {
                    closesocket(self.0);
                    closesocket(client_socket.0);
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
            len: 8192,
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
}

impl Drop for UnixDomainSocket {
    fn drop(&mut self) {
        unsafe { closesocket(self.0) };
    }
}

#[derive(Debug, Default)]
pub struct CompletionPort {
    iocp_handle: HANDLE,
}

unsafe impl Send for CompletionPort {}
unsafe impl Sync for CompletionPort {}

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

    pub fn associate_handle(&self, handle: HANDLE) -> anyhow::Result<()> {
        // This just returns the HANDLE of the existing iocp, so we can ignore the return value
        let _ =
            unsafe { CreateIoCompletionPort(handle, Some(self.iocp_handle), handle.0 as usize, 0) }
                .map_err(|err| anyhow!("could not add handle to iocp: {err}"))?;

        Ok(())
    }

    pub fn get_queued_completion_status(
        &self,
        timeout: Option<time::Duration>,
    ) -> anyhow::Result<OVERLAPPED_ENTRY> {
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
            "could not get queued completion status: {:?}",
            io::Error::last_os_error(),
        ))?;

        let overlapped_entry = OVERLAPPED_ENTRY {
            lpCompletionKey: completion_key,
            lpOverlapped: lpoverlapped,
            Internal: usize::default(),
            dwNumberOfBytesTransferred: bytes_transferred,
        };

        Ok(overlapped_entry)
    }
}
