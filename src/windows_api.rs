use windows::core::Param;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{MonitorFromWindow, HMONITOR, MONITOR_FROM_FLAGS};
use windows::Win32::Networking::WinSock::{
    WSACleanup, WSAGetLastError, WSAStartup, SEND_RECV_FLAGS, SOCKADDR, SOCKET,
    WINSOCK_SOCKET_TYPE, WSADATA, WSA_ERROR,
};
use windows::Win32::UI::WindowsAndMessaging::IsZoomed;

pub fn wsa_startup(wversionrequested: u16, lpwsadata: *mut WSADATA) -> i32 {
    unsafe { WSAStartup(wversionrequested, lpwsadata) }
}

pub fn wsa_cleanup() -> i32 {
    unsafe { WSACleanup() }
}

pub fn wsa_get_last_error() -> WSA_ERROR {
    unsafe { WSAGetLastError() }
}

pub fn socket(
    af: i32,
    r#type: WINSOCK_SOCKET_TYPE,
    protocol: i32,
) -> windows::core::Result<SOCKET> {
    // NOTE: below is from the Windows API; not a recursive function
    unsafe { windows::Win32::Networking::WinSock::socket(af, r#type, protocol) }
}

pub fn closesocket<P0>(s: P0) -> i32
where
    P0: Param<SOCKET>,
{
    // NOTE: below is from the Windows API; not a recursive function
    unsafe { windows::Win32::Networking::WinSock::closesocket(s) }
}

pub fn bind<P0>(s: P0, name: *const SOCKADDR, namelen: i32) -> i32
where
    P0: Param<SOCKET>,
{
    // NOTE: below is from the Windows API; not a recursive function
    unsafe { windows::Win32::Networking::WinSock::bind(s, name, namelen) }
}

pub fn listen<P0>(s: P0, backlog: i32) -> i32
where
    P0: Param<SOCKET>,
{
    // NOTE: below is from the Windows API; not a recursive function
    unsafe { windows::Win32::Networking::WinSock::listen(s, backlog) }
}

pub fn accept<P0>(
    s: P0,
    addr: Option<*mut SOCKADDR>,
    addrlen: Option<*mut i32>,
) -> windows::core::Result<SOCKET>
where
    P0: Param<SOCKET>,
{
    // NOTE: below is from the Windows API; not a recursive function
    unsafe { windows::Win32::Networking::WinSock::accept(s, addr, addrlen) }
}

pub fn recv<P0>(s: P0, buf: &mut [u8], flags: SEND_RECV_FLAGS) -> i32
where
    P0: Param<SOCKET>,
{
    // NOTE: below is from the Windows API; not a recursive function
    unsafe { windows::Win32::Networking::WinSock::recv(s, buf, flags) }
}

pub fn monitor_from_window<P0>(hwnd: P0, dwflags: MONITOR_FROM_FLAGS) -> HMONITOR
where
    P0: Param<HWND>,
{
    unsafe { MonitorFromWindow(hwnd, dwflags) }
}

pub fn is_zoomed<P0>(hwnd: P0) -> bool
where
    P0: Param<HWND>,
{
    unsafe { IsZoomed(hwnd) }.as_bool()
}
