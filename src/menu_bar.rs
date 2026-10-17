//! Menu-bar icon: Loupe stays running in the background. Click shows the window.

use std::fs::{self, File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use anyhow::{anyhow, Context, Result};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::sources;

const SHOW: &str = "show";
const SETTINGS: &str = "settings";
const QUIT: &str = "quit";
const LOGIN_LABEL: &str = "app.loupe.gui";

static SHOW_FLAG: AtomicBool = AtomicBool::new(false);
static SETTINGS_FLAG: AtomicBool = AtomicBool::new(false);
static QUIT_FLAG: AtomicBool = AtomicBool::new(false);
static LOCK: Mutex<Option<File>> = Mutex::new(None);

thread_local! {
    static TRAY: RefCell<Option<TrayIcon>> = const { RefCell::new(None) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    Show,
    Settings,
    Quit,
}

/// True when this process owns the menu-bar seat. False when another Loupe
/// is already sitting there and was asked to show.
pub fn take_seat() -> Result<bool> {
    let dir = sources::state_dir();
    fs::create_dir_all(&dir)?;
    let lock_path = dir.join("gui.lock");
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("could not open {}", lock_path.display()))?;
    let rc = unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        ping_running()?;
        return Ok(false);
    }
    let sock = dir.join("gui.sock");
    let _ = fs::remove_file(&sock);
    let socket = UnixDatagram::bind(&sock)
        .with_context(|| format!("could not bind {}", sock.display()))?;
    socket.set_nonblocking(true)?;
    std::thread::Builder::new()
        .name("loupe-seat".into())
        .spawn(move || loop {
            let mut buf = [0u8; 32];
            if let Ok((n, _)) = socket.recv_from(&mut buf) {
                if &buf[..n] == SHOW.as_bytes() {
                    SHOW_FLAG.store(true, Ordering::SeqCst);
                } else if &buf[..n] == SETTINGS.as_bytes() {
                    SETTINGS_FLAG.store(true, Ordering::SeqCst);
                }
            } else {
                std::thread::sleep(std::time::Duration::from_millis(80));
            }
        })
        .context("could not listen for a second Loupe")?;
    *LOCK.lock().expect("lock") = Some(lock);
    Ok(true)
}

fn ping_running() -> Result<()> {
    let sock = sources::state_dir().join("gui.sock");
    let socket = UnixDatagram::unbound()?;
    socket
        .send_to(SHOW.as_bytes(), &sock)
        .with_context(|| format!("could not reach the running Loupe at {}", sock.display()))?;
    Ok(())
}

pub fn install_tray() -> Result<()> {
    let menu = Menu::new();
    let show = MenuItem::with_id(SHOW, "Show Loupe", true, None);
    let settings = MenuItem::with_id(SETTINGS, "Settings…", true, None);
    let quit = MenuItem::with_id(QUIT, "Quit Loupe", true, None);
    menu.append_items(&[
        &show,
        &settings,
        &PredefinedMenuItem::separator(),
        &quit,
    ])?;
    let icon = menu_bar_icon();
    let mut builder = TrayIconBuilder::new()
        .with_tooltip("Loupe")
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false);
    #[cfg(target_os = "macos")]
    {
        builder = builder.with_icon_templated(icon);
    }
    #[cfg(not(target_os = "macos"))]
    {
        builder = builder.with_icon(icon);
    }
    let tray = builder.build().map_err(|err| anyhow!("could not sit in the menu bar. {err}"))?;
    TRAY.with(|slot| *slot.borrow_mut() = Some(tray));
    Ok(())
}

pub fn poll_tray() -> Option<TrayCommand> {
    while let Ok(event) = TrayIconEvent::receiver().try_recv() {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            SHOW_FLAG.store(true, Ordering::SeqCst);
        }
    }
    while let Ok(event) = MenuEvent::receiver().try_recv() {
        match event.id.as_ref() {
            SHOW => SHOW_FLAG.store(true, Ordering::SeqCst),
            SETTINGS => SETTINGS_FLAG.store(true, Ordering::SeqCst),
            QUIT => QUIT_FLAG.store(true, Ordering::SeqCst),
            _ => {}
        }
    }
    if QUIT_FLAG.swap(false, Ordering::SeqCst) {
        return Some(TrayCommand::Quit);
    }
    if SETTINGS_FLAG.swap(false, Ordering::SeqCst) {
        return Some(TrayCommand::Settings);
    }
    if SHOW_FLAG.swap(false, Ordering::SeqCst) {
        return Some(TrayCommand::Show);
    }
    None
}

pub fn login_agent_path() -> PathBuf {
    home_dir().join("Library/LaunchAgents").join(format!("{LOGIN_LABEL}.plist"))
}

pub fn open_at_login_installed() -> bool {
    login_agent_path().is_file()
}

pub fn set_open_at_login(enabled: bool) -> Result<()> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = enabled;
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        let plist = login_agent_path();
        if enabled {
            let exe = std::env::current_exe().context("could not find the Loupe binary")?;
            if let Some(dir) = plist.parent() {
                fs::create_dir_all(dir)?;
            }
            let body = launch_agent_plist(&exe);
            fs::write(&plist, body)
                .with_context(|| format!("could not write {}", plist.display()))?;
            let uid = unsafe { libc::getuid() };
            let target = format!("gui/{uid}/{LOGIN_LABEL}");
            let _ = std::process::Command::new("launchctl")
                .args(["bootout", &target])
                .status();
            let status = std::process::Command::new("launchctl")
                .args(["bootstrap", &format!("gui/{uid}"), plist.to_str().unwrap_or("")])
                .status()
                .context("could not ask launchd to open Loupe at login")?;
            if !status.success() {
                anyhow::bail!("could not ask launchd to open Loupe at login.");
            }
        } else {
            let uid = unsafe { libc::getuid() };
            let target = format!("gui/{uid}/{LOGIN_LABEL}");
            let _ = std::process::Command::new("launchctl")
                .args(["bootout", &target])
                .status();
            let _ = fs::remove_file(&plist);
        }
        Ok(())
    }
}

fn launch_agent_plist(exe: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LOGIN_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
        <string>gui</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <false/>
</dict>
</plist>
"#,
        xml_escape(&exe.display().to_string())
    )
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Template-image magnifying glass: black on clear, so macOS recolors it.
fn menu_bar_icon() -> Icon {
    const S: u32 = 32;
    let mut rgba = vec![0u8; (S * S * 4) as usize];
    let cx = 13.0f32;
    let cy = 13.0f32;
    let r_out = 8.2f32;
    let r_in = 5.4f32;
    for y in 0..S {
        for x in 0..S {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let dx = px - cx;
            let dy = py - cy;
            let d = (dx * dx + dy * dy).sqrt();
            let mut cover = ring(d, r_in, r_out);
            // handle
            let hx = px - 19.0;
            let hy = py - 19.0;
            let along = (hx + hy) / std::f32::consts::SQRT_2;
            let across = (hx - hy) / std::f32::consts::SQRT_2;
            if along > 0.0 && along < 11.0 && across.abs() < 1.6 {
                cover = cover.max(1.0);
            }
            if cover > 0.04 {
                let a = (cover.clamp(0.0, 1.0) * 255.0) as u8;
                let i = ((y * S + x) * 4) as usize;
                rgba[i] = 0;
                rgba[i + 1] = 0;
                rgba[i + 2] = 0;
                rgba[i + 3] = a;
            }
        }
    }
    Icon::from_rgba(rgba, S, S).expect("menu bar icon")
}

fn ring(d: f32, inner: f32, outer: f32) -> f32 {
    let outer_c = (outer + 0.5 - d).clamp(0.0, 1.0);
    let inner_c = (d - (inner - 0.5)).clamp(0.0, 1.0);
    outer_c.min(inner_c)
}
