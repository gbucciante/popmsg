//! Icona nella barra di sistema: tray-icon su Windows/macOS, StatusNotifierItem su Linux.

use fltk::app::Sender;

use crate::config::ABSENCE_PRESETS;
use crate::Msg;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrayCmd {
    Open,
    Present,
    Absence(usize),
    CustomAbsence,
    Busy,
    Update,
    Log,
    Settings,
    About,
    Quit,
}

#[derive(Clone, Copy, PartialEq)]
pub enum State {
    Normal,
    Away,
    Unread,
}

pub const ICON: &[u8] = include_bytes!("../assets/icon64.png");
pub const AWAY: &[u8] = include_bytes!("../assets/away64.png");
pub const UNREAD: &[u8] = include_bytes!("../assets/unread64.png");

/// Decodifica un PNG in RGBA (via FLTK, nessuna libreria in più).
pub fn rgba(png: &[u8]) -> (Vec<u8>, u32, u32) {
    use fltk::prelude::ImageExt;
    let img = fltk::image::PngImage::from_data(png).expect("png");
    let (w, h) = (img.data_w() as u32, img.data_h() as u32);
    let d = img.to_rgb_data();
    let depth = d.len() / (w * h) as usize;
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for px in d.chunks(depth) {
        match depth {
            4 => out.extend_from_slice(px),
            3 => out.extend_from_slice(&[px[0], px[1], px[2], 255]),
            2 => out.extend_from_slice(&[px[0], px[0], px[0], px[1]]),
            _ => out.extend_from_slice(&[px[0], px[0], px[0], 255]),
        }
    }
    (out, w, h)
}

fn png_for(s: State) -> &'static [u8] {
    match s {
        State::Normal => ICON,
        State::Away => AWAY,
        State::Unread => UNREAD,
    }
}

// ------------------------------------------------------------ Windows / macOS
#[cfg(any(windows, target_os = "macos"))]
mod imp {
    use super::*;
    use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    pub struct Tray {
        icon: TrayIcon,
        ids: Vec<(tray_icon::menu::MenuId, TrayCmd)>,
        absence: Vec<CheckMenuItem>,
    }

    fn icon(s: State) -> Icon {
        let (d, w, h) = rgba(png_for(s));
        Icon::from_rgba(d, w, h).expect("icona")
    }

    pub fn start(_tx: Sender<Msg>) -> Option<Tray> {
        let menu = Menu::new();
        let mut ids = Vec::new();
        let mut item = |label: &str, cmd: TrayCmd| {
            let it = MenuItem::new(label, true, None);
            ids.push((it.id().clone(), cmd));
            it
        };
        let title = MenuItem::new(format!("POPMSG {}", crate::net::APP_VERSION), false, None);
        let open = item("Invia messaggio…", TrayCmd::Open);
        let log = item("Registro messaggi", TrayCmd::Log);
        let upd = item("Cerca aggiornamenti…", TrayCmd::Update);
        let settings = item("Impostazioni…", TrayCmd::Settings);
        let about = item("Informazioni su POPMSG", TrayCmd::About);
        let quit = item("Esci", TrayCmd::Quit);
        let sub = Submenu::new("Stato", true);
        let mut absence = Vec::new();
        let present = CheckMenuItem::new("Presente", true, true, None);
        ids.push((present.id().clone(), TrayCmd::Present));
        sub.append(&present).ok();
        sub.append(&PredefinedMenuItem::separator()).ok();
        absence.push(present);
        for (i, (label, _)) in ABSENCE_PRESETS.iter().enumerate() {
            let c = CheckMenuItem::new(*label, true, false, None);
            ids.push((c.id().clone(), TrayCmd::Absence(i)));
            sub.append(&c).ok();
            absence.push(c);
        }
        let custom = CheckMenuItem::new("Personalizzata…", true, false, None);
        ids.push((custom.id().clone(), TrayCmd::CustomAbsence));
        sub.append(&custom).ok();
        absence.push(custom);
        sub.append(&PredefinedMenuItem::separator()).ok();
        let busy = CheckMenuItem::new("Occupato…", true, false, None);
        ids.push((busy.id().clone(), TrayCmd::Busy));
        sub.append(&busy).ok();
        absence.push(busy);
        menu.append_items(&[
            &title,
            &PredefinedMenuItem::separator(),
            &open,
            &PredefinedMenuItem::separator(),
            &sub,
            &log,
            &upd,
            &settings,
            &about,
            &PredefinedMenuItem::separator(),
            &quit,
        ])
        .ok();
        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip("POPMSG")
            .with_icon(icon(State::Normal))
            .build()
            .ok()?;
        Some(Tray { icon, ids, absence })
    }

    impl Tray {
        /// Da chiamare periodicamente dal ciclo principale.
        pub fn poll(&self) -> Vec<Msg> {
            let mut out = Vec::new();
            while let Ok(ev) = MenuEvent::receiver().try_recv() {
                if let Some((_, c)) = self.ids.iter().find(|(id, _)| *id == ev.id) {
                    out.push(Msg::Tray(*c));
                }
            }
            while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
                match ev {
                    TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } => out.push(Msg::Tray(TrayCmd::Open)),
                    TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } => {
                        out.push(Msg::TrayClick)
                    }
                    _ => {}
                }
            }
            out
        }

        pub fn set_state(&self, s: State, absence_index: Option<usize>) {
            let _ = self.icon.set_icon(Some(icon(s)));
            let tip = match s {
                State::Normal => "POPMSG",
                State::Away => "POPMSG (assente)",
                State::Unread => "POPMSG: messaggi non letti",
            };
            let _ = self.icon.set_tooltip(Some(tip));
            // 0 = presente, 1..=N preset, N+1 personalizzata
            let sel = absence_index.map(|i| i + 1).unwrap_or(0);
            for (i, c) in self.absence.iter().enumerate() {
                c.set_checked(i == sel);
            }
        }
    }
}

// ------------------------------------------------------------------- Linux
#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use ksni::blocking::{Handle, TrayMethods};
    use ksni::menu::{MenuItem, RadioGroup, RadioItem, StandardItem, SubMenu};

    pub struct SniTray {
        tx: Sender<Msg>,
        state: State,
        sel: usize,
    }

    impl ksni::Tray for SniTray {
        fn id(&self) -> String {
            "popmsg".into()
        }
        fn title(&self) -> String {
            "POPMSG".into()
        }
        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            let (d, w, h) = rgba(png_for(self.state));
            let mut argb = Vec::with_capacity(d.len());
            for px in d.chunks(4) {
                argb.extend_from_slice(&[px[3], px[0], px[1], px[2]]);
            }
            vec![ksni::Icon { width: w as i32, height: h as i32, data: argb }]
        }
        fn tool_tip(&self) -> ksni::ToolTip {
            ksni::ToolTip { title: "POPMSG".into(), ..Default::default() }
        }
        fn activate(&mut self, _x: i32, _y: i32) {
            self.tx.send(Msg::TrayClick);
        }
        fn menu(&self) -> Vec<MenuItem<Self>> {
            let item = |label: &str, c: TrayCmd| -> MenuItem<Self> {
                StandardItem {
                    label: label.into(),
                    activate: Box::new(move |t: &mut Self| t.tx.send(Msg::Tray(c))),
                    ..Default::default()
                }
                .into()
            };
            let mut opts = vec![RadioItem { label: "Presente".into(), ..Default::default() }];
            for (l, _) in ABSENCE_PRESETS {
                opts.push(RadioItem { label: (*l).into(), ..Default::default() });
            }
            opts.push(RadioItem { label: "Personalizzata…".into(), ..Default::default() });
            opts.push(RadioItem { label: "Occupato…".into(), ..Default::default() });
            let n = ABSENCE_PRESETS.len();
            vec![
                StandardItem { label: format!("POPMSG {}", crate::net::APP_VERSION), enabled: false, ..Default::default() }.into(),
                MenuItem::Separator,
                item("Invia messaggio…", TrayCmd::Open),
                MenuItem::Separator,
                SubMenu {
                    label: "Stato".into(),
                    submenu: vec![RadioGroup {
                        selected: self.sel,
                        select: Box::new(move |t: &mut Self, i| {
                            let c = match i {
                                0 => TrayCmd::Present,
                                i if i <= n => TrayCmd::Absence(i - 1),
                                i if i == n + 1 => TrayCmd::CustomAbsence,
                                _ => TrayCmd::Busy,
                            };
                            t.tx.send(Msg::Tray(c));
                        }),
                        options: opts,
                    }
                    .into()],
                    ..Default::default()
                }
                .into(),
                item("Registro messaggi", TrayCmd::Log),
                item("Cerca aggiornamenti…", TrayCmd::Update),
                item("Impostazioni…", TrayCmd::Settings),
                item("Informazioni su POPMSG", TrayCmd::About),
                MenuItem::Separator,
                item("Esci", TrayCmd::Quit),
            ]
        }
    }

    pub struct Tray(Handle<SniTray>);

    pub fn start(tx: Sender<Msg>) -> Option<Tray> {
        SniTray { tx, state: State::Normal, sel: 0 }.spawn().ok().map(Tray)
    }

    impl Tray {
        pub fn poll(&self) -> Vec<Msg> {
            Vec::new()
        }
        pub fn set_state(&self, s: State, absence_index: Option<usize>) {
            self.0.update(|t| {
                t.state = s;
                t.sel = absence_index.map(|i| i + 1).unwrap_or(0);
            });
        }
    }
}

pub use imp::{start, Tray};
