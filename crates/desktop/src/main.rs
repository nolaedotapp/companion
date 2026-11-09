#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod client;
mod native;
mod theme;
mod ui;

use client::Client;
use gpui::{
    App, AppContext, Application, AssetSource, Bounds, Global, KeyBinding, SharedString,
    TitlebarOptions, WindowBounds, WindowHandle, WindowOptions, actions, px, size,
};
use nolae_core::ipc::ServerMessage;
use ui::CompanionView;

actions!(
    nolae,
    [
        PastePairingCode,
        CheckForUpdates,
        ToggleSettings,
        CloseSettings,
        FocusNext,
        FocusPrevious,
        Quit
    ]
);

struct Runtime {
    client: Client,
    window: WindowHandle<CompanionView>,
}
impl Global for Runtime {}

struct Assets;
impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        Ok(match path {
            "logo.png" => Some(std::borrow::Cow::Borrowed(include_bytes!(
                "../../../assets/logo-on-dark.png"
            ))),
            "minimize.svg" => Some(std::borrow::Cow::Borrowed(include_bytes!(
                "../../../assets/icons/minimize.svg"
            ))),
            "close.svg" => Some(std::borrow::Cow::Borrowed(include_bytes!(
                "../../../assets/icons/close.svg"
            ))),
            "settings.svg" => Some(std::borrow::Cow::Borrowed(include_bytes!(
                "../../../assets/icons/settings.svg"
            ))),
            _ => None,
        })
    }
    fn list(&self, _: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(vec![
            "logo.png".into(),
            "minimize.svg".into(),
            "close.svg".into(),
            "settings.svg".into(),
        ])
    }
}

fn run(client: Client, incoming: Option<async_channel::Receiver<ServerMessage>>) {
    let background =
        cfg!(debug_assertions) && std::env::args().any(|argument| argument == "--background");
    Application::new()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            cx.text_system()
                .add_fonts(vec![
                    std::borrow::Cow::Borrowed(include_bytes!(
                        "../../../assets/fonts/Manrope-Regular.ttf"
                    )),
                    std::borrow::Cow::Borrowed(include_bytes!(
                        "../../../assets/fonts/Manrope-Bold.ttf"
                    )),
                ])
                .expect("Could not load the bundled Manrope font");
            cx.bind_keys([
                KeyBinding::new("ctrl-v", PastePairingCode, None),
                KeyBinding::new("ctrl-u", CheckForUpdates, None),
                KeyBinding::new("ctrl-,", ToggleSettings, None),
                KeyBinding::new("escape", CloseSettings, None),
                KeyBinding::new("ctrl-q", Quit, None),
                KeyBinding::new("tab", FocusNext, None),
                KeyBinding::new("shift-tab", FocusPrevious, None),
            ]);
            let bounds =
                Bounds::centered(None, size(px(theme::WIDTH), px(theme::INITIAL_HEIGHT)), cx);
            let client_for_view = client.clone();
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        is_resizable: false,
                        is_movable: true,
                        show: !background,
                        focus: !background,
                        titlebar: Some(TitlebarOptions {
                            title: Some("Nolae Companion".into()),
                            appears_transparent: true,
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    |window, cx| {
                        window.on_window_should_close(cx, |_, cx| {
                            cx.quit();
                            true
                        });
                        cx.new(|cx| CompanionView::new(client_for_view, window, cx))
                    },
                )
                .expect("Could not open Nolae Companion");
            cx.set_global(Runtime { client, window });
            if !background {
                cx.activate(true);
            }
            if let Some(incoming) = incoming {
                cx.spawn(async move |cx| {
                    while let Ok(message) = incoming.recv().await {
                        let closing = matches!(message, ServerMessage::Shutdown);
                        if cx
                            .update(|cx| {
                                let window = cx.global::<Runtime>().window;
                                match message {
                                    ServerMessage::Snapshot(snapshot) => {
                                        cx.global::<Runtime>()
                                            .client
                                            .set_snapshot(snapshot.clone());
                                        let _ = window.update(cx, |view, _, cx| {
                                            view.set_snapshot(snapshot, cx)
                                        });
                                    }
                                    ServerMessage::Activate => {
                                        let _ = window
                                            .update(cx, |_, window, _| native::activate(window));
                                    }
                                    ServerMessage::Error(message) => {
                                        let _ = window
                                            .update(cx, |view, _, cx| view.set_error(message, cx));
                                    }
                                    ServerMessage::Shutdown | ServerMessage::Hello { .. } => {
                                        cx.quit()
                                    }
                                }
                            })
                            .is_err()
                            || closing
                        {
                            return;
                        }
                    }
                    let _ = cx.update(|cx| cx.quit());
                })
                .detach();
            }
        });
}

fn main() {
    #[cfg(debug_assertions)]
    if std::env::args().any(|argument| argument == "--preview")
        && !std::env::args().any(|argument| argument == "--core-ui")
    {
        let option = |name: &str| {
            std::env::args().find_map(|argument| argument.strip_prefix(name).map(str::to_owned))
        };
        run(
            Client::preview(
                option("--preview-update=").as_deref(),
                option("--preview-connection=").as_deref(),
            ),
            None,
        );
        return;
    }
    if !std::env::args().any(|argument| argument == "--core-ui") {
        if let Err(error) = native::open_core() {
            native::show_error(&error);
        }
        return;
    }
    native::secure_stdio();
    match Client::connect() {
        Ok((client, incoming)) => run(client, Some(incoming)),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
