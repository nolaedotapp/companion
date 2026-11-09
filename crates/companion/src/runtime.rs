use crate::{
    native,
    updates::UpdateMenu,
    window::{self, WindowEvent, WindowProcess},
};
use nolae_agent::{Agent, Command};
use nolae_core::ipc::{ClientMessage, ServerMessage};
use std::{process::Command as Process, sync::mpsc};
use tray_icon::{
    Icon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem},
};

fn icon() -> Result<Icon, Box<dyn std::error::Error>> {
    let image = image::load_from_memory_with_format(
        include_bytes!("../../../assets/nolae.ico"),
        image::ImageFormat::Ico,
    )?
    .into_rgba8();
    let (width, height) = image.dimensions();
    Ok(Icon::from_rgba(image.into_raw(), width, height)?)
}

fn restart() -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let executable = std::env::current_exe().map_err(|_| "Could not locate Companion")?;
    let mut command = Process::new(executable);
    command
        .arg("--restart")
        .creation_flags(windows::Win32::System::Threading::CREATE_NO_WINDOW.0);
    #[cfg(debug_assertions)]
    command.args(
        std::env::args()
            .skip(1)
            .filter(|argument| argument.starts_with("--preview")),
    );
    command
        .spawn()
        .map(|_| ())
        .map_err(|_| "Could not restart Companion. Close and reopen it.".into())
}

fn open_window(
    active: &mut Option<WindowProcess>,
    agent: &Agent,
    sender: &mpsc::SyncSender<WindowEvent>,
    waker: native::Waker,
) {
    if let Some(window) = active.as_mut()
        && window.running()
        && window.send(ServerMessage::Activate)
    {
        return;
    }
    *active = None;
    match WindowProcess::start(agent.snapshot(), sender.clone(), waker) {
        Ok(window) => *active = Some(window),
        Err(error) => native::show_error(&error),
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let background = std::env::args().any(|argument| argument == "--background");
    let Some(_instance) = native::Instance::acquire(!background)? else {
        return Ok(());
    };
    let event_loop = native::EventLoop::new()?;
    let waker = event_loop.waker();
    window::clean_old_versions();
    #[cfg(debug_assertions)]
    let agent = if std::env::args().any(|argument| argument == "--preview") {
        let option = |name: &str| {
            std::env::args().find_map(|argument| argument.strip_prefix(name).map(str::to_owned))
        };
        Agent::preview(
            option("--preview-update=").as_deref(),
            option("--preview-connection=").as_deref(),
        )
    } else {
        Agent::start()
    };
    #[cfg(not(debug_assertions))]
    let agent = Agent::start();
    agent.set_wake_handler(move || waker.wake());
    let (sender, receiver) = mpsc::sync_channel(32);
    let menu_sender = sender.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let _ = menu_sender.try_send(WindowEvent::Menu(event.id.as_ref().to_owned()));
        waker.wake();
    }));
    let tray_sender = sender.clone();
    TrayIconEvent::set_event_handler(Some(move |event| {
        if matches!(
            event,
            TrayIconEvent::DoubleClick {
                button: tray_icon::MouseButton::Left,
                ..
            }
        ) {
            let _ = tray_sender.try_send(WindowEvent::Show);
            waker.wake();
        }
    }));
    let show = MenuItem::with_id("show", "Open Companion", true, None);
    let quit = MenuItem::with_id("quit", "Quit Companion", true, None);
    let menu = Menu::new();
    let mut updates = UpdateMenu::new();
    menu.append_items(&[
        &show,
        &updates.summary,
        &updates.check,
        &updates.changelog,
        &updates.restart,
        &quit,
    ])?;
    let tray = TrayIconBuilder::new()
        .with_guid(native::TRAY_GUID)
        .with_icon(icon()?)
        .with_tooltip("Nolae Companion")
        .with_menu(Box::new(menu))
        .build()?;
    let notifications = !cfg!(debug_assertions)
        || !std::env::args().any(|argument| argument == "--preview")
        || std::env::args().any(|argument| argument == "--preview-notification");
    let mut active = None;
    let mut last = agent.snapshot();
    updates.refresh(&last, &tray, notifications);
    if !background {
        open_window(&mut active, &agent, &sender, waker);
    }
    let mut quitting = false;
    while !quitting {
        let Some(show_requested) = event_loop.wait()? else {
            break;
        };
        if show_requested {
            open_window(&mut active, &agent, &sender, waker);
        }
        while let Ok(event) = receiver.try_recv() {
            match event {
                WindowEvent::Show => open_window(&mut active, &agent, &sender, waker),
                WindowEvent::Closed(id) => {
                    if active.as_ref().is_some_and(|window| window.id() == id) {
                        active = None;
                    }
                }
                WindowEvent::Command(id, message) => {
                    if !active.as_ref().is_some_and(|window| window.id() == id) {
                        continue;
                    }
                    match message {
                        ClientMessage::Command(Command::Quit) => quitting = true,
                        ClientMessage::Command(command) => agent.send(command),
                        ClientMessage::Restart => match restart() {
                            Ok(()) => quitting = true,
                            Err(error) => {
                                if let Some(window) = &active {
                                    window.send(ServerMessage::Error(error));
                                }
                            }
                        },
                    }
                }
                WindowEvent::Menu(id) => match id.as_str() {
                    "show" | "update" => open_window(&mut active, &agent, &sender, waker),
                    "check" => agent.send(Command::CheckUpdates),
                    "changelog" => native::open_url(
                        &agent
                            .snapshot()
                            .update
                            .available
                            .map(|update| update.changelog_url())
                            .unwrap_or("https://github.com/nolaedotapp/companion/releases".into()),
                    ),
                    "restart" => match restart() {
                        Ok(()) => quitting = true,
                        Err(error) => native::show_error(&error),
                    },
                    "quit" => quitting = true,
                    _ => {}
                },
            }
        }
        let snapshot = agent.snapshot();
        if snapshot != last {
            updates.refresh(&snapshot, &tray, notifications);
            if let Some(window) = &active
                && !window.send(ServerMessage::Snapshot(snapshot.clone()))
            {
                active = None;
            }
            last = snapshot;
        }
    }
    agent.send(Command::Quit);
    drop(active);
    MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
    TrayIconEvent::set_event_handler(None::<fn(TrayIconEvent)>);
    Ok(())
}

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|argument| argument == "--probe") {
        let detected = nolae_agent::probe()?;
        println!("{{\"mediaAccess\":true,\"spotifyDetected\":{detected}}}");
        return Ok(());
    }
    if std::env::args().any(|argument| argument == "--version") {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if std::env::args().any(|argument| argument == "--quit" || argument == "--uninstall") {
        native::request_stop()?;
        if std::env::args().any(|argument| argument == "--uninstall") {
            nolae_agent::uninstall()?;
        }
        return Ok(());
    }
    if std::env::args().any(|argument| argument == "--restart") {
        native::request_stop()?;
    }
    run()
}
