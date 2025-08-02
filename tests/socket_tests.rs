use std::{fs, thread};

use anyhow::{Context, anyhow};
use tacky_borders::config::Config;
use tacky_borders::iocp::{CompletionPort, UnixListener, UnixStream};
use windows::Win32::Networking::WinSock::{WSACleanup, WSADATA, WSAStartup};
use windows::Win32::System::IO::OVERLAPPED_ENTRY;

#[test]
fn test_socket_write_read() -> anyhow::Result<()> {
    // Start up the WinSock Service
    let iresult = unsafe { WSAStartup(0x202, &mut WSADATA::default()) };
    if iresult != 0 {
        return Err(anyhow!("WSAStartup failure: {iresult}"));
    }

    let socket_path = Config::get_dir()?.join("test.sock");
    let socket_path_clone = socket_path.clone();

    // If the socket file already exists, we cannot bind to it, so we must delete it first
    if fs::exists(&socket_path).context("could not check if test socket exists")? {
        fs::remove_file(&socket_path)?;
    }

    let port = CompletionPort::new(2)?;

    // Bind to the socket (synchronous)
    let listener = UnixListener::bind(&socket_path)?;
    port.associate_handle(listener.socket.to_handle(), listener.token())?;

    // Queue up an accept operation (asynchronous)
    let mut read_stream = listener.accept()?;
    port.associate_handle(read_stream.socket.to_handle(), read_stream.token())?;

    let mut text = "Hello World!".to_string();
    let mut text_clone = text.clone();

    thread::spawn(move || -> anyhow::Result<()> {
        let mut write_stream = UnixStream::connect(&socket_path_clone)?;
        let input_buffer = unsafe { text_clone.as_bytes_mut() };

        write_stream.write(input_buffer)?;

        Ok(())
    });

    // Wait for the listener to asynchronously accept the connection
    let mut entry = OVERLAPPED_ENTRY::default();
    port.poll_single(None, &mut entry)?;

    // Then, queue up a read operation and wait for that to asynchronously finish
    let output_buffer = vec![0u8; text.len()];
    read_stream.read(output_buffer)?;
    port.poll_single(None, &mut entry)?;

    fs::remove_file(&socket_path)?;
    unsafe { WSACleanup() };

    let correct_output = unsafe { text.as_bytes_mut() };
    assert!(read_stream.buffer == correct_output);

    Ok(())
}
