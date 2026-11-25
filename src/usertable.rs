//! Tabella degli utenti nella finestra di invio: griglia con colonne
//! ridimensionabili, nascondibili (tasto destro sull'intestazione) e ordinabili
//! (clic sull'intestazione; un secondo clic inverte l'ordine).

use std::cell::RefCell;
use std::cmp::Ordering;
use std::rc::Rc;

use fltk::{
    app, draw,
    enums::{Align, Color, Event, Font, FrameType},
    menu::{MenuFlag, MenuItem},
    prelude::*,
    table::{TableContext, TableResizeFlag, TableRow, TableRowSelectFlag, TableRowSelectMode},
};

use crate::config::{self, TableLayout, COLS};

pub struct Row {
    /// chiave della rubrica (login@host[:porta])
    pub key: String,
    pub cells: [String; 5],
    pub online: bool,
}

struct Inner {
    rows: Vec<Row>,
    layout: TableLayout,
    /// colonne visibili, nell'ordine (indici in COLS)
    vis: Vec<usize>,
}

#[derive(Clone)]
pub struct UserTable {
    pub table: TableRow,
    inner: Rc<RefCell<Inner>>,
    on_menu: Rc<RefCell<Option<Box<dyn Fn()>>>>,
}

const FONT_SIZE: i32 = 13;

fn cmp_ip(a: &str, b: &str) -> Ordering {
    use std::net::{IpAddr, SocketAddr};
    let key = |s: &str| -> Option<(IpAddr, u16)> {
        s.parse::<SocketAddr>().map(|a| (a.ip(), a.port())).ok().or_else(|| s.parse::<IpAddr>().ok().map(|i| (i, 0)))
    };
    match (key(a), key(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn cmp_text(a: &str, b: &str) -> Ordering {
    // i campi vuoti vanno in fondo
    match (a.is_empty(), b.is_empty()) {
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        _ => a.to_lowercase().cmp(&b.to_lowercase()),
    }
}

impl Inner {
    fn sort(&mut self) {
        let (c, asc) = (self.layout.sort_col, self.layout.sort_asc);
        self.rows.sort_by(|a, b| {
            let mut o = if c == 4 { cmp_ip(&a.cells[4], &b.cells[4]) } else { cmp_text(&a.cells[c], &b.cells[c]) };
            if !asc {
                o = o.reverse();
            }
            // chi è in linea sempre prima; a parità, per nome
            (!a.online).cmp(&!b.online).then(o).then_with(|| cmp_text(&a.cells[0], &b.cells[0]))
        });
    }
}

impl UserTable {
    pub fn new() -> UserTable {
        let mut t = TableRow::default();
        t.set_type(TableRowSelectMode::Multi);
        t.set_col_header(true);
        t.set_col_header_height(22);
        t.set_col_resize(true);
        t.set_col_resize_min(30);
        t.set_row_height_all(20);
        t.set_scrollbar_size(12);
        t.set_color(Color::White);
        t.set_frame(FrameType::DownBox);
        t.end();
        let layout = config::table_layout();
        let ut = UserTable {
            table: t,
            inner: Rc::new(RefCell::new(Inner { rows: vec![], layout, vis: vec![] })),
            on_menu: Rc::new(RefCell::new(None)),
        };
        ut.apply_columns();
        ut.install_draw();
        ut.install_handle();
        ut
    }

    /// Azione per il tasto destro sulle righe (menu degli utenti).
    pub fn set_menu<F: Fn() + 'static>(&self, f: F) {
        *self.on_menu.borrow_mut() = Some(Box::new(f));
    }

    fn apply_columns(&self) {
        let mut t = self.table.clone();
        let mut inn = self.inner.borrow_mut();
        let vis: Vec<usize> = (0..COLS.len()).filter(|&c| c == 0 || inn.layout.widths[c] > 0).collect();
        t.set_cols(vis.len() as i32);
        for (i, &c) in vis.iter().enumerate() {
            t.set_col_width(i as i32, inn.layout.widths[c].abs().max(30));
        }
        inn.vis = vis;
        t.redraw();
    }

    fn save_widths(&self) {
        let t = self.table.clone();
        let mut inn = self.inner.borrow_mut();
        let vis = inn.vis.clone();
        let mut changed = false;
        for (i, &c) in vis.iter().enumerate() {
            let w = t.col_width(i as i32);
            if w > 0 && w != inn.layout.widths[c] {
                inn.layout.widths[c] = w;
                changed = true;
            }
        }
        if changed {
            config::save_table_layout(&inn.layout);
        }
    }

    fn install_draw(&self) {
        let inner = self.inner.clone();
        let mut t = self.table.clone();
        t.draw_cell(move |t, ctx, row, col, x, y, w, h| {
            let inn = inner.borrow();
            match ctx {
                TableContext::StartPage => draw::set_font(Font::Helvetica, FONT_SIZE),
                TableContext::ColHeader => {
                    let Some(&c) = inn.vis.get(col as usize) else { return };
                    draw::push_clip(x, y, w, h);
                    draw::draw_box(FrameType::ThinUpBox, x, y, w, h, Color::Background);
                    draw::set_draw_color(Color::Black);
                    draw::set_font(Font::HelveticaBold, FONT_SIZE);
                    draw::draw_text2(COLS[c], x + 5, y, w - 18, h, Align::Left | Align::Inside | Align::Clip);
                    if inn.layout.sort_col == c {
                        // triangolo: su = crescente, giù = decrescente
                        let (cx, cy) = (x + w - 10, y + h / 2);
                        draw::set_draw_color(Color::from_rgb(90, 90, 90));
                        if inn.layout.sort_asc {
                            draw::draw_polygon(cx - 4, cy + 2, cx + 4, cy + 2, cx, cy - 3);
                        } else {
                            draw::draw_polygon(cx - 4, cy - 2, cx + 4, cy - 2, cx, cy + 3);
                        }
                    }
                    draw::set_font(Font::Helvetica, FONT_SIZE);
                    draw::pop_clip();
                }
                TableContext::Cell => {
                    let (Some(r), Some(&c)) = (inn.rows.get(row as usize), inn.vis.get(col as usize)) else { return };
                    let sel = t.row_selected(row);
                    draw::push_clip(x, y, w, h);
                    let bg = if sel { Color::Selection } else { Color::White };
                    draw::draw_rect_fill(x, y, w, h, bg);
                    let fg = match (sel, r.online) {
                        (true, true) => Color::White,
                        (true, false) => Color::from_rgb(225, 225, 225),
                        (false, true) => Color::Black,
                        (false, false) => Color::from_rgb(150, 150, 150),
                    };
                    draw::set_draw_color(fg);
                    draw::set_font(Font::Helvetica, FONT_SIZE);
                    draw::draw_text2(&r.cells[c], x + 5, y, w - 8, h, Align::Left | Align::Inside | Align::Clip);
                    // griglia
                    draw::set_draw_color(Color::from_rgb(222, 222, 222));
                    draw::draw_line(x + w - 1, y, x + w - 1, y + h - 1);
                    draw::draw_line(x, y + h - 1, x + w - 1, y + h - 1);
                    draw::pop_clip();
                }
                _ => {}
            }
        });
    }

    fn install_handle(&self) {
        let me = self.clone();
        let mut t = self.table.clone();
        t.super_handle_first(false);
        // dopo un clic gestito da noi, ignora trascinamento e rilascio
        let mut swallow = false;
        t.handle(move |t, ev| {
            // la selezione cambia solo alcune celle: ridisegnare tutta la tabella
            // evita residui grafici (è piccola, costa poco)
            if matches!(ev, Event::Push | Event::Drag | Event::Released | Event::KeyDown | Event::Shortcut) {
                t.redraw();
            }
            match ev {
            Event::Drag if swallow => true,
            Event::Released if swallow => {
                swallow = false;
                true
            }
            Event::Push => {
                swallow = true;
                let Some((ctx, row, col, flag)) = t.cursor2rowcol() else {
                    swallow = false;
                    return false;
                };
                let right = app::event_mouse_button() == app::MouseButton::Right;
                match ctx {
                    TableContext::ColHeader if right => {
                        me.column_menu();
                        true
                    }
                    TableContext::ColHeader if flag == TableResizeFlag::None => {
                        let c = me.inner.borrow().vis.get(col as usize).copied();
                        if let Some(c) = c {
                            me.sort_by(c);
                        }
                        true
                    }
                    TableContext::Cell if right => {
                        if !t.row_selected(row) {
                            t.select_all_rows(TableRowSelectFlag::Deselect);
                            let _ = t.select_row(row, TableRowSelectFlag::Select);
                            t.redraw();
                        }
                        let f = me.on_menu.clone();
                        app::add_timeout3(0.0, move |_| {
                            if let Some(f) = f.borrow().as_ref() {
                                f();
                            }
                        });
                        true
                    }
                    _ => {
                        swallow = false;
                        false
                    }
                }
            }
            Event::Released => {
                me.save_widths();
                false
            }
            _ => false,
            }
        });
    }

    /// Clic sull'intestazione: ordina per quella colonna, o inverte l'ordine.
    fn sort_by(&self, c: usize) {
        let sel = self.selected_keys();
        {
            let mut inn = self.inner.borrow_mut();
            if inn.layout.sort_col == c {
                inn.layout.sort_asc = !inn.layout.sort_asc;
            } else {
                inn.layout.sort_col = c;
                inn.layout.sort_asc = true;
            }
            inn.sort();
            config::save_table_layout(&inn.layout);
        }
        self.reselect(&sel);
    }

    fn column_menu(&self) {
        self.save_widths();
        let menu = MenuItem::new(&COLS);
        {
            let inn = self.inner.borrow();
            for i in 0..COLS.len() {
                if let Some(mut it) = menu.at(i as i32) {
                    it.set_flag(MenuFlag::Toggle);
                    if i == 0 || inn.layout.widths[i] > 0 {
                        it.set();
                    }
                    if i == 0 {
                        it.deactivate(); // "Utente" resta sempre visibile
                    }
                }
            }
        }
        let Some(item) = menu.popup(app::event_x(), app::event_y()) else { return };
        let label = item.label().unwrap_or_default();
        let Some(c) = COLS.iter().position(|n| label == *n) else { return };
        if c == 0 {
            return;
        }
        {
            let mut inn = self.inner.borrow_mut();
            let w = inn.layout.widths[c];
            inn.layout.widths[c] = if w > 0 { -w } else { w.abs().max(config::COL_DEFAULT[c]) };
            config::save_table_layout(&inn.layout);
        }
        self.apply_columns();
    }

    /// Sostituisce le righe mantenendo la selezione.
    pub fn set_rows(&self, rows: Vec<Row>) {
        let sel = self.selected_keys();
        {
            let mut inn = self.inner.borrow_mut();
            inn.rows = rows;
            inn.sort();
        }
        let n = self.inner.borrow().rows.len() as i32;
        let mut t = self.table.clone();
        t.set_rows(n);
        self.reselect(&sel);
    }

    fn reselect(&self, keys: &[String]) {
        let mut t = self.table.clone();
        t.select_all_rows(TableRowSelectFlag::Deselect);
        let idx: Vec<i32> = {
            let inn = self.inner.borrow();
            inn.rows.iter().enumerate().filter(|(_, r)| keys.contains(&r.key)).map(|(i, _)| i as i32).collect()
        };
        for i in idx {
            let _ = t.select_row(i, TableRowSelectFlag::Select);
        }
        t.redraw();
    }

    pub fn selected_keys(&self) -> Vec<String> {
        let mut t = self.table.clone();
        let inn = self.inner.borrow();
        inn.rows.iter().enumerate().filter(|(i, _)| t.row_selected(*i as i32)).map(|(_, r)| r.key.clone()).collect()
    }

    /// Seleziona esattamente queste righe; restituisce quante ne ha trovate.
    pub fn select_keys(&self, keys: &[String]) -> usize {
        self.reselect(keys);
        let first = self.inner.borrow().rows.iter().position(|r| keys.contains(&r.key));
        if let Some(i) = first {
            let mut t = self.table.clone();
            if (i as i32) < t.row_position() || i as i32 > t.row_position() + t.h() / 20 - 3 {
                t.set_row_position(i as i32);
            }
        }
        self.selected_keys().len()
    }
}
