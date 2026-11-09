use crate::client::Client;
use crate::{
    CheckForUpdates, CloseSettings, FocusNext, FocusPrevious, PastePairingCode, Quit, Runtime,
    ToggleSettings, theme::*,
};
use gpui::{
    Bounds, Context, Div, FocusHandle, FontWeight, ImageSource, Pixels, Render, Resource,
    SharedString, Stateful, Window, WindowControlArea, div, img, prelude::*, px, rgb, size, svg,
};
use nolae_core::companion::{
    AgentSnapshot, Command, Connection, UpdateActivity, UpdateRequirement,
};

#[derive(Clone, Copy, PartialEq)]
enum Action {
    Pair,
    Startup,
    NotifyOptionalUpdates,
    CheckUpdate,
    InstallUpdate,
    Restart,
    Changelog,
    Later,
    Disconnect,
    OpenQueue,
    Settings,
    Guide,
    Minimize,
    Close,
}

#[derive(Clone, Copy)]
enum ButtonStyle {
    Primary,
    Secondary,
    Compact,
    Link,
    Danger,
}

pub(crate) struct CompanionView {
    client: Client,
    snapshot: AgentSnapshot,
    focus: FocusHandle,
    error: Option<String>,
    settings: bool,
}

fn fit_to_content(children: Vec<Bounds<Pixels>>, window: &mut Window, _: &mut gpui::App) {
    let Some(content) = children.last() else {
        return;
    };
    let height = f32::from(content.bottom()).clamp(120., MAX_HEIGHT).ceil();
    if (f32::from(window.viewport_size().height) - height).abs() >= 1. {
        window.resize(size(px(WIDTH), px(height)));
    }
}

fn pressable(
    element: Stateful<Div>,
    on_press: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> Stateful<Div> {
    let on_press = std::rc::Rc::new(on_press);
    let on_key = on_press.clone();
    element
        .on_click(move |_, window, cx| on_press(window, cx))
        .on_key_down(move |event, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                on_key(window, cx);
                cx.stop_propagation();
            }
        })
}

impl CompanionView {
    pub(crate) fn new(client: Client, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window);
        Self {
            snapshot: client.snapshot(),
            client,
            focus,
            error: None,
            settings: cfg!(debug_assertions)
                && std::env::args().any(|arg| arg == "--preview")
                && std::env::args().any(|arg| arg == "--preview-settings"),
        }
    }

    pub(crate) fn set_snapshot(&mut self, snapshot: AgentSnapshot, cx: &mut Context<Self>) {
        if self.snapshot != snapshot {
            self.snapshot = snapshot;
            cx.notify();
        }
    }

    pub(crate) fn set_error(&mut self, error: String, cx: &mut Context<Self>) {
        self.error = Some(error);
        self.settings = false;
        cx.notify();
    }

    fn paired(&self) -> bool {
        !matches!(
            self.snapshot.connection,
            Connection::Unpaired | Connection::ReconnectRequired | Connection::Connecting
        )
    }

    fn disabled(&self, action: Action) -> bool {
        match action {
            Action::Pair => self.snapshot.busy || self.snapshot.update.required(),
            Action::CheckUpdate | Action::InstallUpdate | Action::Restart => {
                self.snapshot.update.activity != UpdateActivity::Idle
            }
            _ => false,
        }
    }

    fn act(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled(action) {
            return;
        }
        self.error = None;
        match action {
            Action::Pair => {
                let code = cx
                    .read_from_clipboard()
                    .and_then(|item| item.text())
                    .unwrap_or_default();
                let code = code.trim();
                if code.len() != 24
                    || !code
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
                {
                    self.error = Some("Copy a pairing code from your Nolae queue first.".into());
                } else {
                    self.client.send(Command::Pair(code.into()));
                }
            }
            Action::Startup => self
                .client
                .send(Command::Autostart(!self.snapshot.autostart)),
            Action::NotifyOptionalUpdates => self.client.send(Command::NotifyOptionalUpdates(
                !self.snapshot.notify_optional_updates,
            )),
            Action::CheckUpdate => self.client.send(Command::CheckUpdates),
            Action::InstallUpdate => self.client.send(Command::InstallUpdate),
            Action::Restart => {
                if let Err(error) = self.client.restart() {
                    self.error = Some(error);
                }
            }
            Action::Changelog => {
                if let Some(update) = &self.snapshot.update.available {
                    cx.open_url(&update.changelog_url());
                }
            }
            Action::Later | Action::Close => cx.quit(),
            Action::Minimize => window.minimize_window(),
            Action::Disconnect => {
                self.client.send(Command::Disconnect);
                self.settings = false;
                self.focus.focus(window);
            }
            Action::OpenQueue => cx.open_url("https://nolae.app/queue"),
            Action::Settings => self.settings = !self.settings,
            Action::Guide => cx.open_url("https://docs.nolae.app/spotify-quota"),
        }
        cx.notify();
    }

    fn icon_button(
        &self,
        id: &'static str,
        icon: &'static str,
        action: Action,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let active = action == Action::Settings && self.settings;
        let hover = if action == Action::Close {
            DANGER_HOVER
        } else {
            SURFACE_2
        };
        let button = div()
            .id(id)
            .tab_index(0)
            .size(px(28.))
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .rounded_md()
            .border_1()
            .border_color(rgb(BACKGROUND))
            .cursor_pointer()
            .when(active, |button| button.bg(rgb(SURFACE_2)))
            .focus(|style| style.border_color(rgb(FOREGROUND)))
            .hover(move |style| style.bg(rgb(hover)))
            .when(action == Action::Minimize, |button| {
                button.window_control_area(WindowControlArea::Min)
            })
            .when(action == Action::Close, |button| {
                button.window_control_area(WindowControlArea::Close)
            })
            .child(svg().path(icon).size(px(14.)).text_color(rgb(if active {
                FOREGROUND
            } else {
                SECONDARY_TEXT
            })));
        let view = cx.entity().downgrade();
        pressable(button, move |window, cx| {
            let _ = view.update(cx, |view, cx| view.act(action, window, cx));
        })
    }

    fn titlebar(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .items_center()
            .h(px(44.))
            .pl(px(16.))
            .pr(px(8.))
            .gap(px(2.))
            .flex_shrink_0()
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .window_control_area(WindowControlArea::Drag)
                    .child(
                        img(ImageSource::Resource(Resource::Embedded("logo.png".into())))
                            .w(px(LOGO_WIDTH))
                            .h(px(LOGO_HEIGHT)),
                    ),
            )
            .child(self.icon_button("settings-toggle", "settings.svg", Action::Settings, cx))
            .child(self.icon_button("window-minimize", "minimize.svg", Action::Minimize, cx))
            .child(self.icon_button("window-close", "close.svg", Action::Close, cx))
    }

    fn button(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        action: Action,
        style: ButtonStyle,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let disabled = self.disabled(action);
        let (fill, ink, hover, pressed) = match style {
            ButtonStyle::Primary => (PRIMARY, ON_PRIMARY, PRIMARY_HOVER, PRIMARY_PRESSED),
            ButtonStyle::Secondary | ButtonStyle::Compact => {
                (SURFACE_2, FOREGROUND, SURFACE_HOVER, SURFACE_1)
            }
            ButtonStyle::Link => (BACKGROUND, SECONDARY_TEXT, SURFACE_1, SURFACE_2),
            ButtonStyle::Danger => (BACKGROUND, WARNING, DANGER_HOVER, SURFACE_1),
        };
        let compact = matches!(
            style,
            ButtonStyle::Compact | ButtonStyle::Link | ButtonStyle::Danger
        );
        let button = div()
            .id(id)
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .h(px(if compact { 30. } else { 36. }))
            .px(px(if compact { 10. } else { 16. }))
            .rounded_full()
            .text_size(px(13.))
            .font_weight(FontWeight::BOLD)
            .bg(rgb(fill))
            .text_color(rgb(ink))
            .border_1()
            .border_color(rgb(fill))
            .child(label.into());
        if disabled {
            return button.opacity(0.5);
        }
        let view = cx.entity().downgrade();
        pressable(
            button
                .tab_index(0)
                .cursor_pointer()
                .focus(|style| style.border_color(rgb(FOREGROUND)))
                .hover(move |style| style.bg(rgb(hover)))
                .active(move |style| style.bg(rgb(pressed))),
            move |window, cx| {
                let _ = view.update(cx, |view, cx| view.act(action, window, cx));
            },
        )
    }

    fn switch(
        &self,
        id: &'static str,
        label: &'static str,
        caption: &'static str,
        enabled: bool,
        action: Action,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let view = cx.entity().downgrade();
        pressable(
            div()
                .id(id)
                .tab_index(0)
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .px(px(12.))
                .py(px(10.))
                .rounded(px(10.))
                .border_1()
                .border_color(rgb(SURFACE_1))
                .cursor_pointer()
                .focus(|style| style.border_color(rgb(FOREGROUND)))
                .hover(|style| style.bg(rgb(SURFACE_2)))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(div().text_size(px(13.)).child(label))
                        .child(
                            div()
                                .text_size(px(11.5))
                                .line_height(px(16.))
                                .text_color(rgb(MUTED_TEXT))
                                .child(caption),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .w(px(34.))
                        .h(px(20.))
                        .p(px(3.))
                        .rounded_full()
                        .bg(rgb(if enabled { PRIMARY } else { SURFACE_HOVER }))
                        .when(enabled, |track| track.justify_end())
                        .child(div().size(px(14.)).rounded_full().bg(rgb(if enabled {
                            ON_PRIMARY
                        } else {
                            SECONDARY_TEXT
                        }))),
                ),
            move |window, cx| {
                let _ = view.update(cx, |view, cx| view.act(action, window, cx));
            },
        )
    }

    fn status(&self) -> (&'static str, u32) {
        match self.snapshot.connection {
            Connection::Unpaired => ("Not paired", MUTED_TEXT),
            Connection::Connecting => ("Connecting…", INFO),
            Connection::Connected => ("Connected", SUCCESS),
            Connection::Offline => ("Reconnecting", INFO),
            Connection::ReconnectRequired => ("Pair again", WARNING),
            Connection::UpdateRequired => ("Detection paused", WARNING),
        }
    }

    fn error_line(&self) -> Option<Div> {
        self.error.clone().map(|error| {
            div()
                .text_size(px(12.))
                .line_height(px(17.))
                .text_color(rgb(WARNING))
                .child(error)
        })
    }

    fn hint(text: &'static str) -> Div {
        div()
            .px(px(2.))
            .text_size(px(11.5))
            .line_height(px(16.))
            .text_color(rgb(MUTED_TEXT))
            .child(text)
    }

    fn home(&self, cx: &mut Context<Self>) -> Option<Div> {
        if self.snapshot.update.required() {
            return self.error_line();
        }
        let (status, color) = self.status();
        let track = (self.snapshot.connection == Connection::Connected)
            .then(|| {
                self.snapshot
                    .track_title
                    .clone()
                    .zip(self.snapshot.artist.clone())
            })
            .flatten();
        let banner = self.snapshot.update.available.is_some();
        let page = div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .px(px(14.))
                    .py(px(12.))
                    .rounded(px(14.))
                    .bg(rgb(SURFACE_1))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(div().size(px(6.)).rounded_full().bg(rgb(color)))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .line_height(px(16.))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(rgb(color))
                                    .child(status),
                            ),
                    )
                    .map(|card| match track {
                        Some((title, artist)) => card.child(
                            div()
                                .flex()
                                .flex_col()
                                .min_w_0()
                                .child(
                                    div()
                                        .text_size(px(15.))
                                        .line_height(px(20.))
                                        .font_weight(FontWeight::BOLD)
                                        .truncate()
                                        .child(title),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.5))
                                        .line_height(px(17.))
                                        .text_color(rgb(SECONDARY_TEXT))
                                        .truncate()
                                        .child(artist),
                                ),
                        ),
                        None => card.child(
                            div()
                                .text_size(px(13.))
                                .line_height(px(18.))
                                .text_color(rgb(SECONDARY_TEXT))
                                .child(if self.snapshot.connection == Connection::Connected {
                                    "Open Spotify desktop on this PC.".into()
                                } else {
                                    self.snapshot.detail.clone()
                                }),
                        ),
                    })
                    .children(self.error_line()),
            )
            .child(if self.paired() {
                div().flex().child(
                    self.button(
                        "main-action",
                        "Open Nolae",
                        Action::OpenQueue,
                        if banner {
                            ButtonStyle::Secondary
                        } else {
                            ButtonStyle::Primary
                        },
                        cx,
                    )
                    .flex_1(),
                )
            } else {
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        self.button(
                            "main-action",
                            if self.snapshot.busy {
                                "Connecting…"
                            } else {
                                "Paste pairing code"
                            },
                            Action::Pair,
                            ButtonStyle::Primary,
                            cx,
                        )
                        .flex_1(),
                    )
                    .child(self.button(
                        "secondary-action",
                        "Open queue",
                        Action::OpenQueue,
                        ButtonStyle::Secondary,
                        cx,
                    ))
            })
            .child(Self::hint(if self.paired() {
                "Close anytime. Detection keeps running in the tray."
            } else {
                "Get a code from Queue → Local playback in Nolae."
            }));
        Some(page)
    }

    fn update_banner(&self, cx: &mut Context<Self>) -> Option<Div> {
        let update = &self.snapshot.update;
        let notice = update.available.as_ref()?;
        let pending = update.installed_version.is_some() && !update.can_download();
        let required = update.required();
        let title = if pending {
            format!(
                "v{} is ready",
                update
                    .installed_version
                    .as_deref()
                    .unwrap_or(&notice.version)
            )
        } else if required {
            format!("Update to v{} required", notice.version)
        } else {
            format!("v{} available", notice.version)
        };
        let description = if update.activity == UpdateActivity::Installing {
            "Downloading and verifying. Companion won't restart on its own."
        } else if pending && required {
            "Restart Companion to finish updating and resume detection."
        } else if pending {
            "Restart Companion whenever you're ready to finish updating."
        } else {
            match notice.requirement {
                Some(UpdateRequirement::Critical) => {
                    "This security update is required. Detection is paused until you update and restart."
                }
                Some(UpdateRequirement::Outdated) => {
                    "Three newer releases are out. Detection is paused until you update and restart."
                }
                None => "Update now or later. Companion keeps checking for newer releases.",
            }
        };
        let label: SharedString = match update.activity {
            UpdateActivity::Installing => "Updating…".into(),
            UpdateActivity::Checking => "Checking…".into(),
            UpdateActivity::Idle if pending => "Restart Companion".into(),
            UpdateActivity::Idle => format!("Update to v{}", notice.version).into(),
        };
        let view = cx.entity().downgrade();
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .px(px(14.))
                .py(px(12.))
                .rounded(px(14.))
                .bg(rgb(SURFACE_1))
                .border_1()
                .border_color(rgb(if required { WARNING } else { BORDER }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(px(13.5))
                                .font_weight(FontWeight::BOLD)
                                .child(title),
                        )
                        .child(pressable(
                            div()
                                .id("changelog")
                                .tab_index(0)
                                .flex_shrink_0()
                                .cursor_pointer()
                                .text_size(px(12.))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(PRIMARY))
                                .focus(|style| style.underline())
                                .hover(|style| style.underline())
                                .child("Changelog"),
                            move |window, cx| {
                                let _ = view
                                    .update(cx, |view, cx| view.act(Action::Changelog, window, cx));
                            },
                        )),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .line_height(px(17.))
                        .text_color(rgb(SECONDARY_TEXT))
                        .child(description),
                )
                .when_some(update.error.clone(), |banner, error| {
                    banner.child(
                        div()
                            .text_size(px(12.))
                            .line_height(px(17.))
                            .text_color(rgb(WARNING))
                            .child(error),
                    )
                })
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .pt(px(2.))
                        .child(
                            self.button(
                                "update-action",
                                label,
                                if pending {
                                    Action::Restart
                                } else {
                                    Action::InstallUpdate
                                },
                                ButtonStyle::Primary,
                                cx,
                            )
                            .flex_1(),
                        )
                        .when(!required && !pending, |row| {
                            row.child(self.button(
                                "update-later",
                                "Later",
                                Action::Later,
                                ButtonStyle::Secondary,
                                cx,
                            ))
                        }),
                ),
        )
    }

    fn preferences(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .p(px(4.))
                    .rounded(px(14.))
                    .bg(rgb(SURFACE_1))
                    .child(self.switch(
                        "startup",
                        "Start with Windows",
                        "Ready in the tray when you sign in.",
                        self.snapshot.autostart,
                        Action::Startup,
                        cx,
                    ))
                    .child(self.switch(
                        "auto-update",
                        "Update notifications",
                        "Required updates always notify you.",
                        self.snapshot.notify_optional_updates,
                        Action::NotifyOptionalUpdates,
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px(px(14.))
                    .py(px(10.))
                    .rounded(px(14.))
                    .bg(rgb(SURFACE_1))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::BOLD)
                                    .child(format!("Version {}", env!("CARGO_PKG_VERSION"))),
                            )
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .line_height(px(16.))
                                    .text_color(rgb(MUTED_TEXT))
                                    .child(self.snapshot.update.status()),
                            ),
                    )
                    .child(self.button(
                        "check-update",
                        "Check",
                        Action::CheckUpdate,
                        ButtonStyle::Compact,
                        cx,
                    )),
            )
            .children(self.error_line())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(self.button(
                        "guide",
                        "How it works",
                        Action::Guide,
                        ButtonStyle::Link,
                        cx,
                    ))
                    .when(self.paired(), |row| {
                        row.child(self.button(
                            "disconnect",
                            "Disconnect PC",
                            Action::Disconnect,
                            ButtonStyle::Danger,
                            cx,
                        ))
                    }),
            )
    }
}

impl Render for CompanionView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .overflow_hidden()
            .bg(rgb(BACKGROUND))
            .text_color(rgb(FOREGROUND))
            .font_family("Manrope")
            .text_size(px(13.))
            .line_height(px(18.))
            .flex()
            .flex_col()
            .on_children_prepainted(fit_to_content)
            .on_action(cx.listener(|view, _: &PastePairingCode, window, cx| {
                view.act(Action::Pair, window, cx)
            }))
            .on_action(cx.listener(|view, _: &CheckForUpdates, window, cx| {
                view.act(Action::CheckUpdate, window, cx)
            }))
            .on_action(cx.listener(|view, _: &ToggleSettings, window, cx| {
                view.act(Action::Settings, window, cx)
            }))
            .on_action(cx.listener(|view, _: &CloseSettings, window, cx| {
                if view.settings {
                    view.act(Action::Settings, window, cx)
                } else {
                    cx.quit()
                }
            }))
            .on_action(|_: &FocusNext, window, _| window.focus_next())
            .on_action(|_: &FocusPrevious, window, _| window.focus_prev())
            .on_action(|_: &Quit, _, cx| {
                cx.global::<Runtime>().client.send(Command::Quit);
                cx.quit();
            })
            .child(self.titlebar(cx))
            .child(
                div()
                    .id("body")
                    .track_focus(&self.focus)
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .gap(px(10.))
                    .px(px(14.))
                    .pt(px(2.))
                    .pb(px(14.))
                    .children(self.update_banner(cx))
                    .children(if self.settings {
                        Some(self.preferences(cx))
                    } else {
                        self.home(cx)
                    }),
            )
    }
}
