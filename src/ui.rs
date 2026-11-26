//! Finestre di POPMSG (FLTK): invio, ricezione, impostazioni, registro.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use fltk::{
    app, button::{Button, CheckButton, ReturnButton}, dialog, enums::{Align, Event, Font, FrameType, Key, Shortcut},
    draw, frame::Frame, group::{Flex, Pack, PackType}, input::Input, menu::MenuButton, prelude::*,
    text::{TextBuffer, TextDisplay, TextEditor, WrapMode}, window::Window,
};

use crate::config::{self, Config, ABSENCE_PRESETS};
use crate::net::{Addr, Engine, Message, User};
use crate::protocol as p;
use crate::tray::{self, State, TrayCmd, Tray};
use crate::usertable::{Row, UserTable};

fn icon() -> Option<fltk::image::PngImage> {
    fltk::image::PngImage::from_data(include_bytes!("../assets/popmsg.png")).ok()
}

fn setup_window(w: &mut Window) {
    w.set_icon(icon());
    w.set_callback(|w| {
        // Esc non deve chiudere le finestre con un'azione in corso: chiudi solo con la X o Esc
        w.hide();
        app::delete_widget(w.clone());
    });
}

fn time_label(t: std::time::SystemTime) -> String {
    let dt: chrono::DateTime<chrono::Local> = t.into();
    const DAYS: [&str; 7] = ["lunedì", "martedì", "mercoledì", "giovedì", "venerdì", "sabato", "domenica"];
    use chrono::Datelike;
    format!("{} {}", DAYS[dt.weekday().num_days_from_monday() as usize], dt.format("%d/%m %H:%M:%S"))
}

fn human(n: u64) -> String {
    let f = n as f64;
    if n < 1024 {
        format!("{n} B")
    } else if f < 1024.0 * 1024.0 {
        format!("{:.1} KB", f / 1024.0)
    } else if f < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} MB", f / 1048576.0)
    } else {
        format!("{:.1} GB", f / 1073741824.0)
    }
}

/// Le etichette FLTK interpretano '@' e '&': li rendiamo letterali.
fn lbl(s: &str) -> String {
    s.replace('@', "@@").replace('&', "&&")
}

pub fn notice(title: &str, text: &str) {
    let mut w = Window::default().with_size(340, 120).with_label(title).center_screen();
    let mut col = Flex::default_fill().column();
    col.set_margin(10);
    let mut f = Frame::default().with_label(&lbl(text));
    f.set_align(Align::Wrap | Align::Inside | Align::Left);
    let mut row = Flex::default().row();
    Frame::default();
    let mut ok = ReturnButton::default().with_label("OK");
    row.fixed(&ok, 90);
    row.end();
    col.fixed(&row, 28);
    col.end();
    w.end();
    setup_window(&mut w);
    let mut w2 = w.clone();
    ok.set_callback(move |_| w2.do_callback());
    w.show();
}

/// Inserisce un'emoticon nel testo e la conta tra le più usate.
fn put_emoji(ed: &mut TextEditor, e: &str) {
    ed.insert(e);
    let _ = ed.take_focus();
    config::emoji_used(e);
}

/// Scelta delle emoticon: categorie in alto, griglia scorrevole sotto.
fn emoji_picker(ed: TextEditor) {
    use crate::emoji::{EMOJI, GROUPS};
    const COLS_N: i32 = 10;
    const CELL: i32 = 32;
    let gw = COLS_N * CELL + 14;
    let mut w = Window::default().with_size(gw + 8, 330).with_label("Emoticon");
    w.set_pos(app::event_x_root() - gw / 2, app::event_y_root() - 340);
    let mut col = Flex::default_fill().column();
    col.set_margin(4);
    col.set_pad(4);
    let mut tabs = Flex::default().row();
    tabs.set_pad(1);
    let mut tab_btns = Vec::new();
    for (g, name) in GROUPS.iter().enumerate() {
        let first = EMOJI.iter().position(|x| x.0 as usize == g).unwrap_or(0);
        let mut b = Button::default();
        b.set_image(crate::rich::emoji_image(first, 22));
        b.set_tooltip(name);
        b.set_frame(FrameType::FlatBox);
        b.set_down_frame(FrameType::DownBox);
        tab_btns.push(b);
    }
    tabs.end();
    col.fixed(&tabs, 30);
    let mut title = Frame::default().with_label(GROUPS[0]);
    title.set_align(Align::Inside | Align::Left);
    title.set_label_size(12);
    col.fixed(&title, 16);
    let mut scroll = fltk::group::Scroll::default();
    scroll.set_type(fltk::group::ScrollType::Vertical);
    scroll.set_scrollbar_size(12);
    scroll.set_color(fltk::enums::Color::White);
    scroll.set_frame(FrameType::DownBox);
    let mut canvas = Frame::default();
    scroll.end();
    col.end();
    w.end();
    // stato: gruppo corrente (indici delle emoticon) e cella sotto il mouse
    let items: Rc<RefCell<Vec<usize>>> = Rc::new(RefCell::new(Vec::new()));
    let hover: Rc<RefCell<Option<usize>>> = Rc::new(RefCell::new(None));
    let show_group = {
        let (items, canvas, scroll, title) = (items.clone(), canvas.clone(), scroll.clone(), title.clone());
        move |g: usize| {
            let (mut canvas, mut scroll, mut title) = (canvas.clone(), scroll.clone(), title.clone());
            *items.borrow_mut() = (0..EMOJI.len()).filter(|&i| EMOJI[i].0 as usize == g).collect();
            let n = items.borrow().len() as i32;
            let rows = (n + COLS_N - 1) / COLS_N;
            scroll.scroll_to(0, 0);
            canvas.resize(scroll.x() + 2, scroll.y() + 2, COLS_N * CELL, (rows * CELL).max(scroll.h() - 4));
            title.set_label(GROUPS[g]);
            scroll.redraw();
        }
    };
    {
        let (items, hover, scroll) = (items.clone(), hover.clone(), scroll.clone());
        canvas.draw(move |f| {
            let items = items.borrow();
            let (top, bottom) = (scroll.y(), scroll.y() + scroll.h());
            draw::push_clip(scroll.x() + 2, scroll.y() + 2, scroll.w() - 4, scroll.h() - 4);
            for (k, &i) in items.iter().enumerate() {
                let (r, c) = (k as i32 / COLS_N, k as i32 % COLS_N);
                let (x, y) = (f.x() + c * CELL, f.y() + r * CELL);
                if y + CELL < top || y > bottom {
                    continue; // disegna solo le celle visibili
                }
                if *hover.borrow() == Some(k) {
                    draw::draw_rect_fill(x, y, CELL, CELL, fltk::enums::Color::from_rgb(220, 232, 250));
                }
                if let Some(mut img) = crate::rich::emoji_image(i, 24) {
                    img.draw(x + (CELL - 24) / 2, y + (CELL - 24) / 2, 24, 24);
                }
            }
            draw::pop_clip();
        });
    }
    {
        let (items, hover) = (items.clone(), hover.clone());
        let mut ed = ed.clone();
        canvas.handle(move |f, ev| {
            let cell = || {
                let (c, r) = ((app::event_x() - f.x()) / CELL, (app::event_y() - f.y()) / CELL);
                let k = (r * COLS_N + c) as usize;
                (c >= 0 && c < COLS_N && app::event_x() >= f.x() && k < items.borrow().len()).then_some(k)
            };
            match ev {
                Event::Enter => true,
                Event::Move => {
                    let k = cell();
                    if k != *hover.borrow() {
                        *hover.borrow_mut() = k;
                        f.set_tooltip("");
                        f.redraw();
                    }
                    true
                }
                Event::Leave => {
                    *hover.borrow_mut() = None;
                    f.redraw();
                    true
                }
                Event::Push => {
                    if let Some(k) = cell() {
                        let e = EMOJI[items.borrow()[k]].1;
                        put_emoji(&mut ed, e);
                    }
                    true
                }
                _ => false,
            }
        });
    }
    let show_group = Rc::new(show_group);
    for (g, b) in tab_btns.iter_mut().enumerate() {
        let f = show_group.clone();
        b.set_callback(move |_| f(g));
    }
    w.make_resizable(false);
    setup_window(&mut w);
    w.show();
    show_group(0);
}

// ======================================================== finestra di invio
pub struct SendWin {
    pub win: Window,
    list: UserTable,
    count: Frame,
    buf: TextBuffer,
    seal: CheckButton,
    lock: CheckButton,
    att: Rc<RefCell<Vec<PathBuf>>>,
    att_lbl: Frame,
}

impl SendWin {
    pub fn new(ctl: &Rc<Ctl>, select: &[String], text: &str) -> Rc<SendWin> {
        let mut win = Window::default().with_size(640, 430).with_label("Invia messaggio").center_screen();
        win.make_resizable(true);
        let mut col = Flex::default_fill().column();
        col.set_margin(6);
        col.set_pad(4);

        let mut top = Flex::default().row();
        let list = UserTable::new();
        let mut side = Flex::default().column();
        let mut l1 = Frame::default().with_label("Utenti");
        l1.set_align(Align::Inside | Align::Bottom);
        let mut count = Frame::default().with_label("0");
        count.set_label_size(22);
        count.set_label_font(Font::HelveticaBold);
        let mut refresh = Button::default().with_label("Aggiorna");
        Frame::default();
        side.fixed(&l1, 22);
        side.fixed(&count, 34);
        side.fixed(&refresh, 28);
        side.end();
        top.fixed(&side, 80);
        top.end();

        let buf = TextBuffer::default();
        let mut ed = TextEditor::default();
        ed.set_buffer(buf.clone());
        ed.wrap_mode(WrapMode::AtBounds, 0);
        ed.set_text_size(14);
        ed.set_scrollbar_size(12);

        // riga delle emoticon preferite (le più usate)
        let mut favs = Flex::default().row();
        favs.set_pad(1);
        for e in config::favorites(14) {
            let Some(i) = crate::rich::emoji_index(&e) else { continue };
            let mut b = Button::default();
            b.set_image(crate::rich::emoji_image(i, 20));
            b.set_frame(FrameType::FlatBox);
            b.set_down_frame(FrameType::DownBox);
            b.set_tooltip("Emoticon preferita");
            favs.fixed(&b, 28);
            let mut ed = ed.clone();
            b.set_callback(move |_| put_emoji(&mut ed, &e));
        }
        Frame::default();
        favs.end();

        let mut att_lbl = Frame::default();
        att_lbl.set_align(Align::Inside | Align::Left);
        att_lbl.set_label_size(12);

        let mut bottom = Flex::default().row();
        let mut attach = MenuButton::default().with_label("Allega");
        let mut groups = MenuButton::default().with_label("Gruppi");
        groups.set_tooltip("Seleziona, crea o elimina i tuoi gruppi di destinatari");
        let mut emo = Button::default();
        emo.set_image(crate::rich::emoji_index("😊").and_then(|i| crate::rich::emoji_image(i, 20)));
        emo.set_tooltip("Emoticon");
        let mut seal = CheckButton::default().with_label("Sigilla");
        seal.set_value(ctl.cfg.borrow().seal);
        Frame::default();
        let mut send = ReturnButton::default().with_label("Invia");
        send.set_label_font(Font::HelveticaBold);
        bottom.fixed(&attach, 80);
        bottom.fixed(&groups, 80);
        bottom.fixed(&emo, 40);
        let mut lock = CheckButton::default().with_label("Lucchetto");
        lock.set_tooltip("Il destinatario deve inserire la sua password di POPMSG / IP Messenger per aprirlo");
        if !seal.value() {
            lock.deactivate();
        }
        {
            let mut l = lock.clone();
            seal.set_callback(move |s| if s.value() { l.activate() } else { l.set_value(false); l.deactivate() });
        }
        bottom.fixed(&seal, 75);
        bottom.fixed(&lock, 95);
        bottom.fixed(&send, 120);
        bottom.end();

        col.fixed(&favs, 26);
        col.fixed(&att_lbl, 20);
        att_lbl.hide();
        col.fixed(&bottom, 32);
        col.end();
        win.end();
        setup_window(&mut win);

        let sw = Rc::new(SendWin {
            win: win.clone(),
            list,
            count,
            buf,
            seal,
            lock,
            att: Rc::new(RefCell::new(Vec::new())),
            att_lbl,
        });

        // azioni
        {
            let c = ctl.clone();
            refresh.set_callback(move |_| c.refresh_all());
        }
        {
            let ed2 = ed.clone();
            emo.set_callback(move |_| emoji_picker(ed2.clone()));
        }
        attach.add_choice("File…|Cartella…|Rimuovi allegati");
        // gruppi: il menu viene ricostruito ogni volta che si apre
        {
            #[derive(Clone)]
            enum GAct {
                Select(Vec<String>),
                Save,
                Delete(String),
            }
            let acts: Rc<RefCell<Vec<(String, GAct)>>> = Rc::new(RefCell::new(Vec::new()));
            {
                let acts = acts.clone();
                groups.handle(move |m, ev| {
                    // il menu si ricostruisce quando il mouse entra nel pulsante (prima del clic)
                    if ev == Event::Enter || ev == Event::Focus || (ev == Event::Push && m.size() == 0) {
                        m.clear();
                        let mut a = acts.borrow_mut();
                        a.clear();
                        let clean = |n: &str| lbl(n).replace(['/', '|', '\\', '_'], "-");
                        let list = config::groups_all();
                        let mut last = -1;
                        for (name, keys) in &list {
                            let path = format!("{}  ({})", clean(name), keys.len());
                            last = m.add_choice(&path);
                            a.push((path, GAct::Select(keys.clone())));
                        }
                        if last >= 0 {
                            if let Some(mut it) = m.at(last) {
                                it.set_flag(fltk::menu::MenuFlag::MenuDivider);
                            }
                        }
                        m.add_choice("Salva selezione come gruppo…");
                        a.push(("Salva selezione come gruppo…".into(), GAct::Save));
                        for (name, _) in &list {
                            let path = format!("Elimina gruppo/{}", clean(name));
                            m.add_choice(&path);
                            a.push((path, GAct::Delete(name.clone())));
                        }
                    }
                    matches!(ev, Event::Enter)
                });
            }
            let (s, c) = (sw.clone(), ctl.clone());
            groups.set_callback(move |m| {
                let Some(item) = m.value().ge(&0).then(|| m.at(m.value())).flatten() else { return };
                let Ok(path) = m.item_pathname(Some(&item)) else { return };
                let path = path.trim_start_matches('/').to_string();
                let act = acts.borrow().iter().find(|(p, _)| *p == path).map(|x| x.1.clone());
                match act {
                    Some(GAct::Select(keys)) => s.select_keys(&c, &keys),
                    Some(GAct::Save) => {
                        let keys = s.list.selected_keys();
                        if keys.is_empty() {
                            dialog::alert_default("Seleziona prima gli utenti da mettere nel gruppo.");
                        } else if let Some(name) = dialog::input_default(&format!("Nome del nuovo gruppo ({} utenti):", keys.len()), "") {
                            let name = name.trim().replace('\t', " ");
                            if !name.is_empty() {
                                config::group_save(&name, &keys);
                            }
                        }
                    }
                    Some(GAct::Delete(n)) => {
                        if dialog::choice2_default(&format!("Eliminare il gruppo «{n}»?"), "No", "Elimina", "") == Some(1) {
                            config::group_delete(&n);
                        }
                    }
                    None => {}
                }
            });
        }
        {
            let s = sw.clone();
            let mut col = col.clone();
            attach.set_callback(move |m| {
                match m.value() {
                    0 => {
                        let mut d = dialog::NativeFileChooser::new(dialog::NativeFileChooserType::BrowseMultiFile);
                        d.set_title("Allega file");
                        d.show();
                        s.add_attachments(&mut col, d.filenames());
                    }
                    1 => {
                        let mut d = dialog::NativeFileChooser::new(dialog::NativeFileChooserType::BrowseDir);
                        d.set_title("Allega cartella");
                        d.show();
                        s.add_attachments(&mut col, d.filenames());
                    }
                    _ => {
                        s.att.borrow_mut().clear();
                        s.add_attachments(&mut col, vec![]);
                    }
                }
            });
        }
        let do_send: Rc<dyn Fn()> = {
            let s = sw.clone();
            let c = ctl.clone();
            Rc::new(move || {
                let users: Vec<(User, bool)> = s.list.selected_keys().iter().filter_map(|k| c.target(k)).collect();
                let text = s.buf.text().trim_end().to_string();
                let att = s.att.borrow().clone();
                if users.is_empty() {
                    dialog::alert_default("Seleziona almeno un destinatario.");
                    return;
                }
                if text.trim().is_empty() && att.is_empty() {
                    return;
                }
                c.send_targets(&users, &text, if !s.seal.value() { 0 } else if s.lock.value() { 2 } else { 1 }, &att);
                let mut w = s.win.clone();
                w.do_callback();
            })
        };
        {
            let f = do_send.clone();
            send.set_callback(move |_| f());
        }
        {
            let f = do_send.clone();
            ed.handle(move |_, ev| {
                if ev == Event::KeyDown
                    && matches!(app::event_key(), Key::Enter | Key::KPEnter)
                    && app::event_state().contains(Shortcut::Ctrl)
                {
                    f();
                    return true;
                }
                false
            });
        }
        // trascina file nella finestra
        {
            let s = sw.clone();
            let mut col = col.clone();
            let mut w = win.clone();
            w.handle(move |_, ev| match ev {
                Event::DndEnter | Event::DndDrag | Event::DndRelease => true,
                Event::Paste => {
                    let paths: Vec<PathBuf> = app::event_text()
                        .lines()
                        .map(|l| l.trim().trim_start_matches("file://"))
                        .filter(|l| !l.is_empty())
                        .map(|l| PathBuf::from(percent_decode(l)))
                        .filter(|p| p.exists())
                        .collect();
                    if paths.is_empty() {
                        false
                    } else {
                        s.add_attachments(&mut col, paths);
                        true
                    }
                }
                _ => false,
            });
        }
        // menu col tasto destro sulla lista utenti
        {
            let (s, c) = (Rc::downgrade(&sw), ctl.clone());
            sw.list.set_menu(move || {
                let Some(s) = s.upgrade() else { return };
                let menu = fltk::menu::MenuItem::new(&["Modifica Tel…", "Aggiungi utente…", "Importa elenco utenti…", "Esporta elenco utenti…", "Aggiorna elenco", "Messaggio di assenza…", "Versione del programma…", "Togli dalla lista (se non in linea)", "Impostazioni…", "Registro messaggi", "Informazioni su POPMSG"]);
                if let Some(item) = menu.popup(app::event_x(), app::event_y()) {
                    let targets: Vec<(User, bool)> = s.list.selected_keys().iter().filter_map(|k| c.target(k)).collect();
                    let sel: Vec<User> = targets.iter().filter(|t| t.1).map(|t| t.0.clone()).collect();
                    let all: Vec<User> = targets.iter().map(|t| t.0.clone()).collect();
                    match item.label().unwrap_or_default().as_str() {
                        "Modifica Tel…" => {
                            if let Some(u) = all.first() {
                                let cur = config::tel_get(&u.book_key());
                                if let Some(v) = dialog::input_default(&format!("Tel di {} (salvato solo su questo computer):", u.name()), &cur) {
                                    config::tel_set(&u.book_key(), &v);
                                    s.reload(&c);
                                }
                            } else {
                                dialog::alert_default("Seleziona prima un utente.");
                            }
                        }
                        "Aggiorna elenco" => c.refresh_all(),
                        "Aggiungi utente…" => add_user_dialog(&c),
                        "Esporta elenco utenti…" => {
                            let Some(choice) = dialog::choice2_default("Esportare anche i numeri di Tel?", "Annulla", "Senza Tel", "Con i numeri di Tel") else { return };
                            if choice == 0 {
                                return;
                            }
                            let with_tel = choice == 2;
                            let mut d = dialog::NativeFileChooser::new(dialog::NativeFileChooserType::BrowseSaveFile);
                            d.set_title("Esporta elenco utenti");
                            d.set_preset_file("utenti-popmsg.txt");
                            d.set_option(dialog::NativeFileChooserOptions::SaveAsConfirm);
                            d.show();
                            let f = d.filename();
                            if !f.as_os_str().is_empty() {
                                match config::known_export(&f, with_tel) {
                                    Ok(n) => notice("POPMSG", &format!("Elenco di {n} utenti salvato in\n{}\n\nSugli altri PC: tasto destro sulla lista → Importa elenco utenti…", f.display())),
                                    Err(e) => dialog::alert_default(&format!("Impossibile salvare il file: {e}")),
                                }
                            }
                        }
                        "Importa elenco utenti…" => {
                            let mut d = dialog::NativeFileChooser::new(dialog::NativeFileChooserType::BrowseFile);
                            d.set_title("Importa elenco utenti");
                            d.show();
                            let f = d.filename();
                            if !f.as_os_str().is_empty() {
                                match config::known_import(&f) {
                                    Ok((n, t)) => {
                                        let mut msg = format!("{n} nuovi utenti aggiunti all'elenco, {t} numeri di Tel aggiunti.");
                                        if t > 0 {
                                            msg.push_str("\n(I numeri già presenti su questo computer non sono stati cambiati.)");
                                        }
                                        if !c.cfg.borrow().offline {
                                            msg.push_str("\nPer vedere gli utenti quando non sono in linea attiva l'opzione in Impostazioni.");
                                        }
                                        notice("POPMSG", &msg);
                                        c.probe_known();
                                        s.reload(&c);
                                    }
                                    Err(e) => dialog::alert_default(&format!("Impossibile importare: {e}")),
                                }
                            }
                        }
                        "Impostazioni…" => c.tray_cmd(TrayCmd::Settings),
                        "Registro messaggi" => c.tray_cmd(TrayCmd::Log),
                        "Informazioni su POPMSG" => c.tray_cmd(TrayCmd::About),
                        "Messaggio di assenza…" => sel.iter().for_each(|u| c.engine.request_absence(u)),
                        "Togli dalla lista (se non in linea)" => {
                            for t in targets.iter().filter(|t| !t.1) {
                                config::known_remove(&t.0.book_key());
                            }
                            s.reload(&c);
                        }
                        _ => sel.iter().for_each(|u| c.engine.request_version(u)),
                    }
                }
            });
        }
        sw.reload(ctl);
        sw.list.select_keys(select);
        if !text.is_empty() {
            sw.buf.clone().set_text(text);
            ed.set_insert_position(text.len() as i32);
        }
        win.show();
        let _ = ed.take_focus();
        sw
    }

    /// Seleziona i membri di un gruppo (quelli non in linea solo se l'opzione è attiva).
    fn select_keys(&self, ctl: &Ctl, keys: &[String]) {
        self.reload(ctl);
        let found = self.list.select_keys(keys);
        if found < keys.len() {
            notice("POPMSG", &format!("{} membri del gruppo non sono raggiungibili ora e non sono stati selezionati.", keys.len() - found));
        }
    }

    fn add_attachments(&self, col: &mut Flex, paths: Vec<PathBuf>) {
        {
            let mut a = self.att.borrow_mut();
            for p in paths {
                if !a.contains(&p) {
                    a.push(p);
                }
            }
        }
        let a = self.att.borrow();
        let mut f = self.att_lbl.clone();
        if a.is_empty() {
            f.set_label("");
            f.hide();
        } else {
            f.show();
            let names: Vec<String> = a
                .iter()
                .map(|p| {
                    let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if p.is_dir() { format!("{n}/") } else { n }
                })
                .collect();
            f.set_label(&lbl(&format!("Allegati: {}", names.join(",  "))));
            col.fixed(&f, 20);
        }
        col.layout();
        self.win.clone().redraw();
    }

    pub fn reload(&self, ctl: &Ctl) {
        let users = ctl.targets();
        let port = ctl.engine.port();
        let rows: Vec<Row> = users
            .iter()
            .map(|(u, online)| {
                let ip = if !*online {
                    String::new()
                } else if u.addr.port() == port {
                    u.addr.ip().to_string()
                } else {
                    u.addr.to_string()
                };
                let name = if !*online {
                    format!("{} (non in linea)", u.name())
                } else if u.absent() {
                    format!("{} (assente)", u.name())
                } else {
                    u.name().to_string()
                };
                let tel = config::tel_get(&u.book_key());
                Row { key: u.book_key(), cells: [name, u.group.clone(), tel, u.host.clone(), ip], online: *online }
            })
            .collect();
        self.list.set_rows(rows);
        let mut c = self.count.clone();
        c.set_label(&users.iter().filter(|u| u.1).count().to_string());
    }
}

/// Aggiunge a mano un utente che non ha ancora aperto POPMSG.
fn add_user_dialog(ctl: &Rc<Ctl>) {
    let mut w = Window::default().with_size(400, 250).with_label("Aggiungi utente").center_screen();
    let mut col = Flex::default_fill().column();
    col.set_margin(10);
    col.set_pad(6);
    let mut info = Frame::default().with_label(
        "Per chi non ha ancora aperto POPMSG. Quando lo aprirà verrà riconosciuto dal nome del computer (o dall'indirizzo IP) e riceverà i messaggi in attesa.",
    );
    info.set_align(Align::Wrap | Align::Inside | Align::Left);
    info.set_label_size(12);
    col.fixed(&info, 48);
    let field = |label: &str, col: &mut Flex| {
        let mut row = Flex::default().row();
        let l = Frame::default().with_label(label).with_align(Align::Inside | Align::Right);
        let inp = Input::default();
        row.fixed(&l, 130);
        row.end();
        col.fixed(&row, 26);
        inp
    };
    let mut name = field("Nome:", &mut col);
    let group = field("Gruppo:", &mut col);
    let host = field("Nome del computer:", &mut col);
    let ip = field("Indirizzo IP:", &mut col);
    Frame::default();
    let mut row = Flex::default().row();
    Frame::default();
    let mut cancel = Button::default().with_label("Annulla");
    let mut ok = ReturnButton::default().with_label("Aggiungi");
    row.fixed(&cancel, 90);
    row.fixed(&ok, 100);
    row.end();
    col.fixed(&row, 28);
    col.end();
    w.end();
    w.make_modal(true);
    setup_window(&mut w);
    {
        let w = w.clone();
        cancel.set_callback(move |_| w.clone().do_callback());
    }
    {
        let (w, c) = (w.clone(), ctl.clone());
        let (name, group, host, ip) = (name.clone(), group.clone(), host.clone(), ip.clone());
        ok.set_callback(move |_| {
            let (n, h, i) = (name.value(), host.value(), ip.value());
            if n.trim().is_empty() || (h.trim().is_empty() && i.trim().is_empty()) {
                dialog::alert_default("Scrivi il nome e almeno il nome del computer o l'indirizzo IP.");
                return;
            }
            if !i.trim().is_empty() && i.trim().parse::<std::net::IpAddr>().is_err() {
                dialog::alert_default("L'indirizzo IP non è valido (esempio: 192.168.1.20).");
                return;
            }
            config::known_add_manual(&n, &group.value(), &h, &i);
            if !c.cfg.borrow().offline {
                notice("POPMSG", "Utente aggiunto. Per vederlo quando non è in linea attiva l'opzione in Impostazioni.");
            }
            c.probe_known();
            c.users_changed();
            w.clone().do_callback();
        });
    }
    w.show();
    let _ = name.take_focus();
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() + 0 && i + 2 <= b.len() - 1 {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ====================================================== finestra ricevuta
pub fn recv_window(ctl: &Rc<Ctl>, m: Message) {
    let mut win = Window::default().with_size(420, 320).with_label("Messaggio ricevuto").center_screen();
    win.make_resizable(true);
    let mut col = Flex::default_fill().column();
    col.set_margin(6);
    col.set_pad(4);

    let kind = if m.auto_reply() {
        "Risposta automatica da"
    } else if m.command & p::BROADCASTOPT != 0 {
        "Messaggio a tutti da"
    } else if m.multi() {
        "Messaggio di gruppo da"
    } else {
        "Messaggio da"
    };
    let s = &m.sender;
    let detail: Vec<&str> = [s.group.as_str(), s.host.as_str()].into_iter().filter(|x| !x.is_empty()).collect();
    let mut h1 = Frame::default().with_label(&lbl(&format!("{kind}  {}{}", s.name(), if detail.is_empty() { String::new() } else { format!("  ({})", detail.join("/")) })));
    h1.set_align(Align::Inside | Align::Left);
    h1.set_label_font(Font::HelveticaBold);
    let enc = if m.encrypted { "    [cifrato]" } else { "" };
    let mut h2 = Frame::default().with_label(&lbl(&format!("{}{enc}", time_label(m.received_at))));
    h2.set_align(Align::Inside | Align::Left);
    h2.set_label_size(12);
    col.fixed(&h1, 20);
    col.fixed(&h2, 18);

    let buf = TextBuffer::default();
    let mut body = Flex::default().column();
    let rv = crate::rich::RichView::new();
    let mut td = rv.scroll.clone();
    let mut open = Button::default().with_label(if m.command & p::PASSWORDOPT != 0 { "Apri\n(sigillato con lucchetto)" } else { "Apri\n(messaggio sigillato)" });
    open.set_label_size(16);
    open.set_label_font(Font::HelveticaBold);
    body.end();
    let _ = &buf;
    if m.sealed() {
        td.hide();
    } else {
        open.hide();
        rv.clone().set_text(&m.text);
    }

    // allegati
    let mut files = Pack::default();
    files.set_type(PackType::Vertical);
    files.set_spacing(2);
    for f in &m.files {
        let label = if f.is_dir() { format!("Cartella:  {}/", f.name) } else { format!("File:  {}   ({})", f.name, human(f.size)) };
        let mut b = Button::default().with_size(0, 26).with_label(&lbl(&label));
        b.set_align(Align::Inside | Align::Left);
        b.set_tooltip("Salva…");
        let c = ctl.clone();
        let msg = m.clone();
        let info = f.clone();
        b.set_callback(move |b| save_file(&c, &msg, &info, b));
    }
    files.end();
    col.fixed(&files, (m.files.len() as i32 * 28).max(1));
    if m.sealed() || m.files.is_empty() {
        files.hide();
    }

    let mut bottom = Flex::default().row();
    let mut close = Button::default().with_label("Chiudi");
    Frame::default();
    let mut reply = ReturnButton::default().with_label("Rispondi");
    let mut quote = CheckButton::default().with_label("Cita");
    quote.set_value(ctl.cfg.borrow().quote);
    bottom.fixed(&close, 90);
    bottom.fixed(&reply, 110);
    bottom.fixed(&quote, 60);
    bottom.end();
    col.fixed(&bottom, 30);
    col.end();
    win.end();
    setup_window(&mut win);

    let opened = Rc::new(RefCell::new(!m.sealed()));
    let do_open: Rc<dyn Fn()> = {
        let (td, open, buf, opened, c, msg) = (td.clone(), open.clone(), rv.clone(), opened.clone(), ctl.clone(), m.clone());
        let (mut col, files) = (col.clone(), files.clone());
        Rc::new(move || {
            if *opened.borrow() {
                return;
            }
            // lucchetto: serve la password impostata su questo computer
            let (stored, all) = {
                let cfg = c.cfg.borrow();
                (cfg.password.clone(), cfg.password_all)
            };
            if !stored.is_empty() && (msg.command & p::PASSWORDOPT != 0 || all) {
                let Some(pw) = dialog::password_default("Messaggio sigillato con lucchetto.\nInserisci la tua password:", "") else { return };
                if !config::password_check(&stored, &pw) {
                    dialog::alert_default("Password errata.");
                    return;
                }
            }
            *opened.borrow_mut() = true;
            let (mut open, mut buf, mut td, mut col) = (open.clone(), buf.clone(), td.clone(), col.clone());
            open.hide();
            buf.set_text(&msg.text);
            td.show();
            if !msg.files.is_empty() {
                let mut f = files.clone();
                f.show();
            }
            col.layout();
            c.engine.mark_read(&msg);
        })
    };
    {
        let f = do_open.clone();
        open.set_callback(move |_| f());
    }
    {
        let mut w = win.clone();
        close.set_callback(move |_| w.do_callback());
    }
    {
        let (c, msg, f, mut w, op) = (ctl.clone(), m.clone(), do_open.clone(), win.clone(), opened.clone());
        reply.set_callback(move |_| {
            f();
            if !*op.borrow() {
                return; // password non inserita
            }
            let q = if quote.value() && !msg.text.trim().is_empty() {
                msg.text.trim_end().lines().map(|l| format!("> {l}\n")).collect::<String>()
            } else {
                String::new()
            };
            w.do_callback();
            c.open_send(&[msg.sender.book_key()], &q);
        });
    }
    win.show();
    if m.sealed() {
        let _ = open.take_focus();
    } else {
        let _ = reply.take_focus();
    }
}

fn save_file(ctl: &Rc<Ctl>, m: &Message, info: &p::FileInfo, b: &mut Button) {
    let mut d = dialog::NativeFileChooser::new(dialog::NativeFileChooserType::BrowseDir);
    d.set_title(&format!("Salva «{}» nella cartella…", info.name));
    let _ = d.set_directory(&ctl.cfg.borrow().download_dir);
    d.show();
    let Some(dest) = d.filenames().into_iter().next() else { return };
    ctl.cfg.borrow_mut().download_dir = dest.to_string_lossy().into_owned();
    let eng = ctl.engine.clone();
    let (m, info) = (m.clone(), info.clone());
    let mut b = b.clone();
    b.deactivate();
    let orig = b.label();
    let n_files = m.files.len();
    let done = ctl.downloaded.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    std::thread::spawn(move || {
        let mut last = std::time::Instant::now();
        let mut prog = |got: u64, tot: u64, _name: &str| {
            if last.elapsed().as_millis() > 150 {
                last = std::time::Instant::now();
                let t = if tot > 0 { format!("{}%", got * 100 / tot) } else { human(got) };
                if !b.was_deleted() {
                    b.set_label(&format!("{orig}   ... {t}"));
                }
                app::awake();
            }
        };
        let res = eng.download(&m, &info, &dest, &mut prog, &cancel);
        match res {
            Ok(path) if !b.was_deleted() => {
                b.set_label(&format!("{orig}   - salvato, clic per aprire"));
                b.set_tooltip(&path.to_string_lossy());
                let pth = path.clone();
                b.set_callback(move |_| open_path(&pth));
                b.activate();
                let all = {
                    let mut d = done.lock().unwrap();
                    let e = d.entry(m.packet_no).or_insert(0);
                    *e += 1;
                    *e >= n_files
                };
                if all {
                    eng.release_files(&m);
                }
            }
            Ok(_) => {
                eng.release_files(&m);
            }
            Err(e) if !b.was_deleted() => {
                b.set_label(&format!("{orig}   - errore: {e}"));
                b.activate();
            }
            Err(_) => {}
        }
        app::awake();
    });
}

fn open_path(p: &Path) {
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(p).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(p).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(p).spawn();
}

// ============================================================ impostazioni
fn settings_dialog(ctl: &Rc<Ctl>) -> bool {
    let cfg = ctl.cfg.borrow().clone();
    let mut win = Window::default().with_size(500, 620).with_label("Impostazioni di POPMSG").center_screen();
    let mut col = Flex::default_fill().column();
    col.set_margin(10);
    col.set_pad(6);
    let row = |label: &str, value: &str, col: &mut Flex| -> Input {
        let mut r = Flex::default().row();
        let l = Frame::default().with_label(label).with_align(Align::Inside | Align::Left);
        let mut i = Input::default();
        i.set_value(value);
        r.fixed(&l, 150);
        r.end();
        col.fixed(&r, 26);
        i
    };
    let nick = row("Nome visualizzato:", &cfg.nick, &mut col);
    let group = row("Gruppo:", &cfg.group, &mut col);
    let dl = row("Cartella per i file:", &cfg.download_dir, &mut col);
    let extra = row("Indirizzi aggiuntivi:", &cfg.extra.join(", "), &mut col);
    let port = row("Porta (riavvio):", &cfg.port.to_string(), &mut col);
    let check = |label: &str, v: bool, col: &mut Flex| -> CheckButton {
        let mut c = CheckButton::default().with_label(label);
        c.set_value(v);
        col.fixed(&c, 22);
        c
    };
    let popup = check("Apri subito la finestra quando arriva un messaggio", cfg.popup, &mut col);
    let sound = check("Suono alla ricezione", cfg.sound, &mut col);
    let quote = check("Cita il messaggio quando rispondi", cfg.quote, &mut col);
    let seal = check("Sigilla i messaggi per impostazione predefinita", cfg.seal, &mut col);
    let enc = check("Cifra i messaggi (RSA 2048 + AES 256)", cfg.encryption, &mut col);
    let offline = check("Mostra in grigio chi non è in linea e tieni i messaggi in attesa", cfg.offline, &mut col);
    let share = check("Scambia automaticamente l'elenco utenti con gli altri POPMSG", cfg.share_list, &mut col);
    let log = check("Registra i messaggi su file", cfg.log_enabled, &mut col);
    let auto = check("Avvia POPMSG all'accesso a Windows", config::autostart_enabled(), &mut col);
    // password per aprire i messaggi sigillati
    let mut pr = Flex::default().row();
    let pl = Frame::default()
        .with_label(if cfg.password.is_empty() { "Password sigillati: nessuna" } else { "Password sigillati: impostata" })
        .with_align(Align::Inside | Align::Left);
    let mut pbtn = Button::default().with_label("Imposta…");
    pr.fixed(&pbtn, 90);
    pr.end();
    col.fixed(&pr, 26);
    let pw_all = check("Chiedi la password per aprire tutti i messaggi sigillati", cfg.password_all, &mut col);
    let new_pw: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    {
        let (np, mut pl) = (new_pw.clone(), pl.clone());
        pbtn.set_callback(move |_| {
            let Some(a) = dialog::password_default("Nuova password per aprire i messaggi sigillati\n(vuota = nessuna password):", "") else { return };
            if !a.is_empty() {
                let Some(b) = dialog::password_default("Ripeti la password:", "") else { return };
                if a != b {
                    dialog::alert_default("Le due password non coincidono.");
                    return;
                }
            }
            pl.set_label(if a.is_empty() { "Password sigillati: nessuna" } else { "Password sigillati: impostata" });
            *np.borrow_mut() = Some(a);
        });
    }
    if !cfg!(windows) {
        let mut a = auto.clone();
        a.hide();
    }
    let fp = ctl.engine.fingerprint().unwrap_or_else(|| "-".into());
    let mut f = Frame::default().with_label(&format!("Impronta della tua chiave: {fp}\nRegistro: {}", lbl(&cfg.log_path)));
    f.set_align(Align::Inside | Align::Left | Align::Wrap);
    f.set_label_size(11);
    let mut b = Flex::default().row();
    Frame::default();
    let mut cancel = Button::default().with_label("Annulla");
    let mut ok = ReturnButton::default().with_label("OK");
    b.fixed(&cancel, 90);
    b.fixed(&ok, 90);
    b.end();
    col.fixed(&b, 30);
    col.end();
    win.end();
    win.set_icon(icon());
    win.make_modal(true);
    let result = Rc::new(RefCell::new(false));
    {
        let mut w = win.clone();
        cancel.set_callback(move |_| w.hide());
    }
    {
        let (mut w, r) = (win.clone(), result.clone());
        ok.set_callback(move |_| {
            *r.borrow_mut() = true;
            w.hide();
        });
    }
    win.show();
    while win.shown() {
        app::wait();
    }
    if !*result.borrow() {
        return false;
    }
    let mut c = ctl.cfg.borrow_mut();
    c.nick = nick.value().trim().to_string();
    c.group = group.value().trim().to_string();
    c.download_dir = dl.value().trim().to_string();
    c.extra = extra.value().split([',', ';', ' ']).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    if let Ok(pv) = port.value().trim().parse::<u16>() {
        c.port = pv;
    }
    c.popup = popup.value();
    c.sound = sound.value();
    c.quote = quote.value();
    c.seal = seal.value();
    c.encryption = enc.value();
    c.offline = offline.value();
    c.log_enabled = log.value();
    c.share_list = share.value();
    c.password_all = pw_all.value();
    if let Some(pw) = new_pw.borrow().as_ref() {
        c.password = if pw.is_empty() { String::new() } else { config::password_hash(pw) };
    }
    if cfg!(windows) && auto.value() != config::autostart_enabled() {
        config::set_autostart(auto.value());
    }
    c.save();
    true
}

/// Messaggi del registro che contengono tutte le parole cercate (maiuscole indifferenti).
fn log_filter(all: &str, query: &str) -> (String, usize) {
    let sep = format!("{}\n", "=".repeat(38));
    let words: Vec<String> = query.split_whitespace().map(|w| w.to_lowercase()).collect();
    let hits: Vec<&str> = all
        .split(sep.as_str())
        .filter(|e| !e.trim().is_empty())
        .filter(|e| {
            let low = e.to_lowercase();
            words.iter().all(|w| low.contains(w.as_str()))
        })
        .collect();
    let n = hits.len();
    (hits.into_iter().map(|e| format!("{sep}{e}")).collect(), n)
}

fn log_viewer(path: &str) {
    let mut win = Window::default().with_size(680, 540).with_label("Registro messaggi").center_screen();
    win.make_resizable(true);
    let mut col = Flex::default_fill().column();
    col.set_margin(6);
    col.set_pad(4);
    let mut top = Flex::default().row();
    let mut l = Frame::default().with_label("Cerca:");
    l.set_align(Align::Inside | Align::Right);
    let mut search = Input::default();
    search.set_tooltip("Parole, nome, gruppo o data (es. 25/11/2026): mostra solo i messaggi che le contengono tutte");
    let mut find = ReturnButton::default().with_label("Cerca");
    let mut next = Button::default().with_label("Successivo");
    let mut all = Button::default().with_label("Tutto");
    top.fixed(&l, 50);
    top.fixed(&find, 80);
    top.fixed(&next, 90);
    top.fixed(&all, 60);
    top.end();
    col.fixed(&top, 28);
    let mut status = Frame::default();
    status.set_align(Align::Inside | Align::Left);
    status.set_label_size(12);
    col.fixed(&status, 16);
    let mut buf = TextBuffer::default();
    let mut td = TextDisplay::default();
    td.set_buffer(buf.clone());
    td.wrap_mode(WrapMode::AtBounds, 0);
    td.set_text_size(13);
    col.end();
    win.end();
    setup_window(&mut win);
    let text = Rc::new(std::fs::read_to_string(path).unwrap_or_else(|_| "(Il registro è vuoto.)".into()));
    buf.set_text(&text);
    let len = buf.length();
    td.set_insert_position(len);
    td.show_insert_position();
    // evidenzia la prossima parola trovata a partire dalla posizione attuale
    let highlight: Rc<dyn Fn(bool)> = {
        let (buf, td, search) = (buf.clone(), td.clone(), search.clone());
        Rc::new(move |from_start: bool| {
            let Some(w) = search.value().split_whitespace().next().map(|w| w.to_string()) else { return };
            let mut td = td.clone();
            let start = if from_start { 0 } else { td.insert_position() + 1 };
            let pos = buf.search_forward(start.min(buf.length()), &w, false).or_else(|| buf.search_forward(0, &w, false));
            if let Some(pos) = pos {
                let mut b = buf.clone();
                b.select(pos, pos + w.len() as i32);
                td.set_insert_position(pos);
                td.show_insert_position();
            }
        })
    };
    {
        let (text, mut buf, mut status, search, h) = (text.clone(), buf.clone(), status.clone(), search.clone(), highlight.clone());
        find.set_callback(move |_| {
            let q = search.value();
            if q.trim().is_empty() {
                return;
            }
            let (t, n) = log_filter(&text, &q);
            if n == 0 {
                status.set_label(&lbl(&format!("Nessun messaggio contiene «{}».", q.trim())));
                return;
            }
            buf.set_text(&t);
            status.set_label(&lbl(&format!("{n} messaggi contengono «{}».  «Tutto» mostra di nuovo l'intero registro.", q.trim())));
            h(true);
        });
    }
    {
        let h = highlight.clone();
        next.set_callback(move |_| h(false));
    }
    {
        let (text, mut buf, mut status, mut td) = (text.clone(), buf.clone(), status.clone(), td.clone());
        all.set_callback(move |_| {
            buf.set_text(&text);
            status.set_label("");
            td.set_insert_position(buf.length());
            td.show_insert_position();
        });
    }
    win.show();
    let _ = search.take_focus();
}

// ============================================================ controllore
pub struct Ctl {
    pub engine: Engine,
    pub cfg: RefCell<Config>,
    send_wins: RefCell<Vec<Rc<SendWin>>>,
    sent: RefCell<HashMap<u64, (User, String, u8, Vec<PathBuf>)>>,
    unread: RefCell<Vec<Message>>,
    pub tray: RefCell<Option<Tray>>,
    absence_sel: RefCell<Option<usize>>,
    downloaded: Arc<std::sync::Mutex<HashMap<u64, usize>>>,
    inflight: RefCell<HashMap<u64, u64>>,
}

impl Ctl {
    pub fn new(engine: Engine, cfg: Config) -> Rc<Ctl> {
        Rc::new(Ctl {
            engine,
            cfg: RefCell::new(cfg),
            send_wins: RefCell::new(Vec::new()),
            sent: RefCell::new(HashMap::new()),
            unread: RefCell::new(Vec::new()),
            tray: RefCell::new(None),
            absence_sel: RefCell::new(None),
            downloaded: Arc::new(std::sync::Mutex::new(HashMap::new())),
            inflight: RefCell::new(HashMap::new()),
        })
    }

    pub fn any_window(&self) -> bool {
        app::windows().map(|w| w.iter().any(|w| w.shown())).unwrap_or(false)
    }

    pub fn open_send(self: &Rc<Self>, select: &[String], text: &str) {
        let w = SendWin::new(self, select, text);
        self.send_wins.borrow_mut().retain(|s| !s.win.was_deleted() && s.win.shown());
        self.send_wins.borrow_mut().push(w);
    }

    /// Annuncia la presenza in broadcast e direttamente agli utenti noti non in linea.
    pub fn refresh_all(&self) {
        self.engine.refresh();
        self.probe_known();
    }

    pub fn probe_known(&self) {
        let addrs: Vec<Addr> = config::known_all().iter().filter_map(|k| k.addr.parse().ok()).collect();
        self.engine.probe(&addrs);
    }

    pub fn users_changed(&self) {
        let online = self.engine.users();
        // utenti aggiunti a mano: quando compaiono, prendono la loro chiave vera
        for u in &online {
            if let Some(old) = config::manual_match(&u.host, std::net::IpAddr::V4(*u.addr.ip())) {
                config::known_rekey(&old, &u.book_key());
            }
        }
        config::known_update(
            &online
                .iter()
                .map(|u| config::Known {
                    key: u.book_key(),
                    login: u.login.clone(),
                    host: u.host.clone(),
                    nick: u.nick.trim_end_matches(" [Assente]").to_string(),
                    group: u.group.clone(),
                    addr: u.addr.to_string(),
                })
                .collect::<Vec<_>>(),
        );
        self.flush_outbox(&online);
        self.send_wins.borrow_mut().retain(|s| !s.win.was_deleted() && s.win.shown());
        for s in self.send_wins.borrow().iter() {
            s.reload(self);
        }
    }

    /// Utenti in linea più quelli visti in passato (non in linea).
    pub fn targets(&self) -> Vec<(User, bool)> {
        let online = self.engine.users();
        let keys: std::collections::HashSet<String> = online.iter().map(|u| u.book_key()).collect();
        let mut out: Vec<(User, bool)> = online.into_iter().map(|u| (u, true)).collect();
        if !self.cfg.borrow().offline {
            return out;
        }
        for k in config::known_all() {
            if keys.contains(&k.key) {
                continue;
            }
            let addr = k.addr.parse().unwrap_or_else(|_| Addr::new(std::net::Ipv4Addr::UNSPECIFIED, 0));
            out.push((User { addr, login: k.login, host: k.host, nick: k.nick, group: k.group, command: 0 }, false));
        }
        out
    }

    pub fn target(&self, key: &str) -> Option<(User, bool)> {
        self.targets().into_iter().find(|t| t.0.book_key() == key)
    }

    pub fn send_targets(&self, targets: &[(User, bool)], text: &str, sealed: u8, att: &[PathBuf]) {
        let online: Vec<User> = targets.iter().filter(|t| t.1).map(|t| t.0.clone()).collect();
        if !online.is_empty() {
            self.send(&online, text, sealed, att);
        }
        let offline: Vec<&User> = targets.iter().filter(|t| !t.1).map(|t| &t.0).collect();
        if offline.is_empty() {
            return;
        }
        let files: Vec<String> = att.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        for u in &offline {
            config::outbox_add(&u.book_key(), u.name(), text, sealed, &files);
        }
        let names: Vec<&str> = offline.iter().map(|u| u.name()).collect();
        notice(
            "POPMSG",
            &format!(
                "{} non {} in linea.\nIl messaggio resta in attesa su questo computer e verrà consegnato appena {} POPMSG (anche se nel frattempo chiudi e riapri il programma).",
                names.join(", "),
                if names.len() > 1 { "sono" } else { "è" },
                if names.len() > 1 { "apriranno" } else { "aprirà" }
            ),
        );
        let cfg = self.cfg.borrow();
        if cfg.log_enabled {
            config::write_log(&cfg.log_path, "A", &names.join(", "), text, &["in attesa"], &[]);
        }
    }

    fn flush_outbox(&self, online: &[User]) {
        for q in config::outbox_all() {
            if self.inflight.borrow().values().any(|id| *id == q.id) {
                continue;
            }
            let Some(u) = online.iter().find(|u| u.book_key() == q.key) else { continue };
            let text = format!("[Messaggio scritto il {} mentre non eri in linea]\n{}", q.written, q.text);
            let files: Vec<PathBuf> = q.files.iter().map(PathBuf::from).filter(|p| p.exists()).collect();
            let nos = self.engine.send_message(std::slice::from_ref(u), &text, q.sealed, &files);
            for no in nos {
                self.inflight.borrow_mut().insert(no, q.id);
                self.sent.borrow_mut().insert(no, (u.clone(), text.clone(), q.sealed, files.clone()));
            }
        }
    }

    pub fn on_delivered(&self, no: u64) {
        if let Some(id) = self.inflight.borrow_mut().remove(&no) {
            config::outbox_remove(id);
        }
    }

    pub fn send(&self, users: &[User], text: &str, sealed: u8, att: &[PathBuf]) {
        let nos = self.engine.send_message(users, text, sealed, att);
        let mut sent = self.sent.borrow_mut();
        for (no, u) in nos.iter().zip(users) {
            sent.insert(*no, (u.clone(), text.to_string(), sealed, att.to_vec()));
        }
        let cfg = self.cfg.borrow();
        if cfg.log_enabled {
            let who: Vec<String> = users.iter().map(|u| format!("{} ({}/{})", u.name(), u.group, u.host)).collect();
            let mut flags = vec![];
            if sealed == 2 {
                flags.push("sigillato con lucchetto");
            } else if sealed == 1 {
                flags.push("sigillato");
            }
            if users.iter().all(|u| self.engine.will_encrypt(u)) {
                flags.push("cifrato");
            }
            let files: Vec<String> = att.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
            config::write_log(&cfg.log_path, "A", &who.join(", "), text, &flags, &files);
        }
    }

    pub fn on_message(self: &Rc<Self>, m: Message) {
        let (log, path, sound, popup) = {
            let c = self.cfg.borrow();
            (c.log_enabled, c.log_path.clone(), c.sound, c.popup)
        };
        if log {
            let s = &m.sender;
            let mut flags = vec![];
            if m.sealed() {
                flags.push("sigillato");
            }
            if m.encrypted {
                flags.push("cifrato");
            }
            if m.multi() {
                flags.push("multiplo");
            }
            let files: Vec<String> = m.files.iter().map(|f| f.name.clone()).collect();
            config::write_log(&path, "Da", &format!("{} ({}/{})", s.name(), s.group, s.host), &m.text, &flags, &files);
        }
        if sound {
            dialog::beep(dialog::BeepType::Message);
        }
        if popup || self.tray.borrow().is_none() {
            recv_window(self, m);
        } else {
            self.unread.borrow_mut().push(m);
            self.update_tray();
        }
    }

    pub fn show_unread(self: &Rc<Self>) -> bool {
        let list: Vec<Message> = self.unread.borrow_mut().drain(..).collect();
        let any = !list.is_empty();
        for m in list {
            recv_window(self, m);
        }
        self.update_tray();
        any
    }

    pub fn on_opened(&self, no: u64, name: &str) {
        if self.sent.borrow().contains_key(&no) {
            notice("POPMSG", &format!("Il messaggio sigillato è stato aperto da {name}.\n({})", chrono::Local::now().format("%H:%M:%S")));
        }
    }

    pub fn on_failed(&self, no: u64) {
        if self.inflight.borrow_mut().remove(&no).is_some() {
            return; // resta in attesa: riproveremo quando ricompare
        }
        let Some((u, text, sealed, att)) = self.sent.borrow().get(&no).cloned() else { return };
        let r = dialog::choice2_default(
            &format!("Il messaggio a {} non è stato consegnato.\nVuoi riprovare ora o lasciarlo in attesa finché torna in linea?", u.name()),
            "Annulla",
            "Riprova",
            if self.cfg.borrow().offline { "In attesa" } else { "" },
        );
        if r == Some(1) {
            let target = self.engine.user(&u.addr).unwrap_or(u);
            self.send(&[target], &text, sealed, &att);
        } else if r == Some(2) {
            let files: Vec<String> = att.iter().map(|p| p.to_string_lossy().into_owned()).collect();
            config::outbox_add(&u.book_key(), u.name(), &text, sealed, &files);
        }
    }

    pub fn tray_cmd(self: &Rc<Self>, c: TrayCmd) {
        match c {
            TrayCmd::Open => self.open_send(&[], ""),
            TrayCmd::Present => self.set_absence(None, None),
            TrayCmd::Absence(i) => self.set_absence(Some(i), Some(ABSENCE_PRESETS[i].1.to_string())),
            TrayCmd::CustomAbsence => {
                let cur = self.cfg.borrow().absence_text.clone();
                match dialog::input_default("Messaggio di risposta automatica:", &cur) {
                    Some(t) if !t.trim().is_empty() => self.set_absence(Some(ABSENCE_PRESETS.len()), Some(t)),
                    _ => self.update_tray(),
                }
            }
            TrayCmd::Log => log_viewer(&self.cfg.borrow().log_path.clone()),
            TrayCmd::Settings => {
                let old = self.cfg.borrow().clone();
                if settings_dialog(self) {
                    let c = self.cfg.borrow().clone();
                    self.engine.set_encryption(c.encryption);
                    self.engine.set_share_list(c.share_list);
                    self.engine.set_identity(&c.nick, &c.group, c.extra.clone());
                    if c.port != old.port {
                        notice("POPMSG", "La nuova porta sarà usata al prossimo avvio.");
                    }
                    self.users_changed();
                }
            }
            TrayCmd::About => notice(
                "Informazioni su POPMSG",
                &format!(
                    "POPMSG {}\n\nMessaggistica per reti locali, senza server.\nCompatibile con il protocollo di IP Messenger (porta 2425).\nCifratura RSA 2048 + AES 256.",
                    crate::net::APP_VERSION
                ),
            ),
            TrayCmd::Quit => self.quit(),
        }
    }

    fn set_absence(&self, sel: Option<usize>, text: Option<String>) {
        if let Some(t) = &text {
            let mut c = self.cfg.borrow_mut();
            c.absence_text = t.clone();
            c.save();
        }
        self.engine.set_absence(sel.is_some(), text.as_deref());
        *self.absence_sel.borrow_mut() = sel;
        self.update_tray();
    }

    pub fn update_tray(&self) {
        if let Some(t) = self.tray.borrow().as_ref() {
            let s = if !self.unread.borrow().is_empty() {
                State::Unread
            } else if self.engine.absent() {
                State::Away
            } else {
                State::Normal
            };
            t.set_state(s, *self.absence_sel.borrow());
        }
    }

    pub fn quit(&self) {
        self.cfg.borrow().save();
        self.engine.stop();
        std::thread::sleep(std::time::Duration::from_millis(100));
        std::process::exit(0);
    }
}

pub fn handle_net(ctl: &Rc<Ctl>, ev: crate::net::Event) {
    use crate::net::Event as E;
    match ev {
        E::UsersChanged => ctl.users_changed(),
        E::Message(m) => ctl.on_message(*m),
        E::Delivered(no) => ctl.on_delivered(no),
        E::Opened(no, name) => ctl.on_opened(no, &name),
        E::Failed(no, _) => ctl.on_failed(no),
        E::AbsenceInfo(n, t) => notice(&format!("Assenza di {n}"), &t),
        E::VersionInfo(n, t) => notice(&format!("Versione di {n}"), &t),
        E::ListRequest(a) => ctl.engine.send_list(a, &config::known_list_text()),
        E::ListReceived(_, text) => {
            if config::known_merge_text(&text) > 0 {
                ctl.probe_known();
                ctl.users_changed();
            }
        }
        E::Error(e) => eprintln!("{e}"),
    }
}

#[allow(dead_code)]
pub fn start_tray(ctl: &Rc<Ctl>, tx: app::Sender<crate::Msg>) {
    *ctl.tray.borrow_mut() = tray::start(tx);
}
