#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

#[macro_use]
extern crate log;
extern crate sp_log;

use anyhow::{Context, anyhow};
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::process::ExitCode;
use std::sync::LazyLock;
use tacky_borders::colors::ColorBrushConfig;
use tacky_borders::config::WidthConfig;
use tacky_borders::iocp::UnixStream;
use tacky_borders::ipc::{IpcCommand, socket_path};
use tacky_borders::sys_tray_icon::create_tray_icon;
use tacky_borders::utils::{
    LogIfErr, imm_disable_ime, set_process_dpi_awareness_context, spawn_window_state_poller,
};
use tacky_borders::{
    APP_STATE, attach_console, create_borders_for_existing_windows, is_unwanted_instance,
    register_border_window_class, set_event_hook,
};
use windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, TranslateMessage,
};

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();

    // When invoked with arguments, act as an IPC client for a running instance
    // instead of starting as the border daemon itself.
    if !args.is_empty() {
        let args = pico_args::Arguments::from_vec(args);
        return match run_cli(args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                // eprintln because the logger isn't initialized in client mode
                eprintln!("error: {err:#}");
                ExitCode::FAILURE
            }
        };
    }

    run_daemon();
    ExitCode::SUCCESS
}

fn run_daemon() {
    if is_unwanted_instance() {
        return;
    }

    // Force initialization of our app state
    let _ = LazyLock::force(&APP_STATE);

    info!("starting tacky-borders");

    // xFFFFFFFF (-1) is used to disable IME windows for all threads in the current process.
    imm_disable_ime(0xFFFFFFFF)
        .ok()
        .context("could not disable ime")
        .log_if_err();

    set_process_dpi_awareness_context(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
        .context("could not make process dpi aware")
        .log_if_err();

    let hwineventhook = set_event_hook();

    // This owns the tray icon window, so it must be kept in scope
    let tray_icon_res = create_tray_icon(hwineventhook);
    if let Err(err) = tray_icon_res {
        error!("could not create tray icon: {err:#}");
    }

    register_border_window_class().log_if_err();
    create_borders_for_existing_windows().log_if_err();
    spawn_window_state_poller();

    unsafe {
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    info!("exiting tacky-borders");
}

const HELP_TEXT: &str = "\
tacky-borders - start or control a running instance

USAGE:
  tacky-borders                                       start the border daemon
  tacky-borders set-color [OPTIONS]                   change border color at runtime
  tacky-borders set-width <width> [--focused]         change border width at runtime
  tacky-borders reload                                reload config.yaml and recreate borders
  tacky-borders get-state                             print runtime state as json
  tacky-borders msg <json>                            send a raw json command
  tacky-borders help                                  show this help

OPTIONS for set-color:
  -a, --active   <color>    set the active (focused) border color
  -i, --inactive <color>    set the inactive (unfocused) border color
  -f, --focused             only update the currently focused window's border;
                            all other borders are left unchanged

<color> is a hex string like \"#RRGGBB\" or \"#RRGGBBAA\", \"accent\", or a JSON gradient object:
  '{\"colors\":[\"#ffffff\",\"#000000\"],\"direction\":\"90deg\"}'
";

fn run_cli(mut args: pico_args::Arguments) -> anyhow::Result<()> {
    // Console is disabled by default in release mode so we need to attach it
    let _ = attach_console();

    // Help flags have higher priority and should be handled separately
    if args.contains(["-h", "--help"]) {
        print!("{HELP_TEXT}");
        return Ok(());
    }

    let command_json = match args
        .subcommand()?
        .ok_or(anyhow!("missing subcommand"))?
        .as_str()
    {
        "set-color" | "set_color" => {
            let active = args.opt_value_from_fn(["-a", "--active"], parse_color_arg)?;
            let inactive = args.opt_value_from_fn(["-i", "--inactive"], parse_color_arg)?;

            if active.is_none() && inactive.is_none() {
                anyhow::bail!("set-color requires at least one of --active or --inactive");
            }

            let command = IpcCommand::SetColor {
                active,
                inactive,
                focused: args.contains(["-f", "--focused"]),
            };
            serde_json::to_string(&command)?
        }
        "set-width" | "set_width" => {
            let command = IpcCommand::SetWidth {
                width: args.free_from_fn(parse_width_arg)?,
                focused: args.contains(["-f", "--focused"]),
            };
            serde_json::to_string(&command)?
        }
        "reload" => serde_json::to_string(&IpcCommand::Reload)?,
        "get-state" | "get_state" => serde_json::to_string(&IpcCommand::GetState)?,
        "msg" => args.free_from_str()?,
        "help" => {
            print!("{HELP_TEXT}");
            return Ok(());
        }
        other => anyhow::bail!("unknown command '{other}'; see 'tacky-borders help'"),
    };

    let remaining = args.finish();
    if !remaining.is_empty() {
        anyhow::bail!("unknown arguments: {:?}", remaining);
    }

    let response = send_command(&command_json)?;
    println!("{}", response.trim_end());

    Ok(())
}

fn send_command(command_json: &str) -> anyhow::Result<String> {
    let socket_path = socket_path().context("could not get socket path")?;

    let stream = UnixStream::connect(&socket_path).with_context(|| {
        format!(
            "could not connect to {}; is tacky-borders running with the ipc server enabled?",
            socket_path.display()
        )
    })?;

    let message = format!("{command_json}\n");
    (&stream)
        .write_all(message.as_bytes())
        .context("could not send command")?;

    let mut reader = BufReader::new(&stream);
    let mut response = String::new();
    reader
        .read_line(&mut response)
        .context("could not read response")?;

    Ok(response)
}

fn parse_color_arg(s: &str) -> anyhow::Result<ColorBrushConfig> {
    // Try parsing as JSON object first (e.g. gradients), but fallback to treating
    // it as a string (e.g. hex codes) which need to be wrapped in double quotes.
    let color = serde_json::from_str(s).or_else(|_| serde_json::from_str(&format!("\"{s}\"")))?;
    Ok(color)
}

fn parse_width_arg(s: &str) -> anyhow::Result<WidthConfig> {
    let width: f32 = s.parse().context("width must be a number")?;
    Ok(WidthConfig::new(width))
}
