#![cfg_attr(windows, windows_subsystem = "windows")]

use dumpling::pipe;

use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use windui::icon::WindowIcon;
use windui::prelude::*;
use windui::render::image::Image;

struct Session {
    cancel: Option<CancellationToken>,
    url: String,
}

fn session() -> &'static Mutex<Session> {
    static SESSION: Mutex<Session> = Mutex::new(Session {
        cancel: None,
        url: String::new(),
    });
    &SESSION
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

fn main() {
    let mode = signal(0usize);
    let ticket = signal(String::new());
    let host_port = signal(String::from("8080"));
    let status = signal(String::from("Paste a ticket to connect"));
    let window_icon = app_icon();
    let tray_icon = app_icon();

    let tray = Tray::new()
        .tooltip("Dumpling")
        .icon_rgba(tray_icon.width(), tray_icon.height(), tray_icon.rgba())
        .on_left_click(|ctx| ctx.show_window())
        .on_double_click(|ctx| ctx.show_window())
        .menu(vec![
            TrayMenuItem::item("Show window", |ctx| ctx.show_window()),
            TrayMenuItem::item("Open in browser", |_| open_current()),
            TrayMenuItem::item("Disconnect", move |_| {
                stop();
                status.set("Disconnected".into());
            }),
            TrayMenuItem::separator(),
            TrayMenuItem::item("Exit", |ctx| {
                stop();
                ctx.quit();
            }),
        ]);

    let connect_page = Element::col().width_match().spacing(10).child(
        Element::row()
            .width_match()
            .spacing(8)
            .child(
                Element::text_input(ticket, "Paste ticket")
                    .weight(1.0)
                    .on_submit(move |ctx| {
                        if connect(ticket.get(), status) {
                            ctx.hide_window();
                        }
                    }),
            )
            .child(Element::button("Connect").on_click(move |ctx| {
                if connect(ticket.get(), status) {
                    ctx.hide_window();
                }
            })),
    );
    let share_page = Element::col().width_match().spacing(10).child(
        Element::row()
            .width_match()
            .spacing(8)
            .child(Element::text_input(host_port, "Service port to share").weight(1.0))
            .child(Element::button("Share").on_click(move |_| {
                if share(host_port.get(), ticket, status) {
                    mode.set(0);
                }
            })),
    );

    let ui = Element::col()
        .fill()
        .padding(20)
        .spacing(10)
        .bg(Color::hex(0xF7F4EE))
        .child(Element::label("Dumpling").font_size(22.0).width_match())
        .child(
            Element::tabs_pill(mode, vec![("Connect", connect_page), ("Share", share_page)])
                .width_match()
                .weight(1.0),
        )
        .child(Element::label_signal(status).font_size(13.0).width_match());

    App::new("Dumpling", 420, 240)
        .icon(window_icon)
        .bg(Color::hex(0xF7F4EE))
        .tray(tray)
        .hide_on_close()
        .content(ui)
        .run();
}

fn connect(ticket: String, status: Signal<String>) -> bool {
    let parsed = match pipe::parse_ticket(&ticket) {
        Ok(ticket) => ticket,
        Err(err) => {
            status.set(err);
            return false;
        }
    };
    stop();
    status.set("Looking for an available local port...".into());
    let cancel = CancellationToken::new();
    let (tx, rx) = mpsc::channel();
    let task_cancel = cancel.clone();
    runtime().spawn(async move {
        pipe::serve_client(
            parsed,
            "127.0.0.1:0".parse().expect("addr"),
            task_cancel,
            tx,
        )
        .await;
    });
    match rx.recv_timeout(Duration::from_secs(8)) {
        Ok(Ok(local)) => {
            let url = format!("http://{local}");
            {
                let mut guard = session().lock().expect("session");
                guard.cancel = Some(cancel);
                guard.url = url.clone();
            }
            open_browser(&url);
            status.set(format!("Opened {url}; the window is now in the tray"));
            true
        }
        Ok(Err(err)) => {
            cancel.cancel();
            status.set(err);
            false
        }
        Err(_) => {
            cancel.cancel();
            status.set("Startup timed out".into());
            false
        }
    }
}

fn share(port: String, ticket: Signal<String>, status: Signal<String>) -> bool {
    let port = port.trim();
    let Ok(port) = port.parse::<u16>() else {
        status.set("Port must be a number from 1 to 65535".into());
        return false;
    };
    if port == 0 {
        status.set("Port must be a number from 1 to 65535".into());
        return false;
    }
    let Ok(target) = format!("127.0.0.1:{port}").parse() else {
        status.set("Invalid port".into());
        return false;
    };
    stop();
    status.set(format!("Sharing 127.0.0.1:{port}..."));
    let cancel = CancellationToken::new();
    let (tx, rx) = mpsc::channel();
    let task_cancel = cancel.clone();
    runtime().spawn(async move {
        pipe::serve_host(target, task_cancel, tx).await;
    });
    match rx.recv_timeout(Duration::from_secs(8)) {
        Ok(Ok(text)) => {
            ticket.set(text);
            {
                let mut guard = session().lock().expect("session");
                guard.cancel = Some(cancel);
            }
            status.set("Ticket copied to the Connect page; copy it to the peer".into());
            true
        }
        Ok(Err(err)) => {
            cancel.cancel();
            status.set(err);
            false
        }
        Err(_) => {
            cancel.cancel();
            status.set("Sharing timed out".into());
            false
        }
    }
}

fn stop() {
    let mut guard = session().lock().expect("session");
    if let Some(cancel) = guard.cancel.take() {
        cancel.cancel();
    }
    guard.url.clear();
}

fn open_current() {
    let url = session().lock().expect("session").url.clone();
    if !url.is_empty() {
        open_browser(&url);
    }
}

fn open_browser(url: &str) {
    let result = if cfg!(windows) {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
    if let Ok(mut child) = result {
        let _ = child.wait();
    }
}

fn app_icon() -> WindowIcon {
    let bytes = decode_b64(include_str!("../assets/dumpling-32.png.b64"));
    let image = Image::from_png_bytes(&bytes).expect("decode icon");
    WindowIcon::from_image(&image).expect("window icon")
}

fn decode_b64(input: &str) -> Vec<u8> {
    fn val(c: u8) -> u8 {
        match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => 0,
        }
    }
    let clean: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        if chunk.len() < 2 {
            break;
        }
        let a = val(chunk[0]);
        let b = val(chunk[1]);
        out.push((a << 2) | (b >> 4));
        if chunk.len() > 2 && chunk[2] != b'=' {
            let c = val(chunk[2]);
            out.push((b << 4) | (c >> 2));
            if chunk.len() > 3 && chunk[3] != b'=' {
                out.push((c << 6) | val(chunk[3]));
            }
        }
    }
    out
}
