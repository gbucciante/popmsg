//! Visualizzazione del testo con le emoticon disegnate come immagini a colori
//! (uguali su Windows, macOS e Linux, senza dipendere dai font di sistema).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use fltk::{
    app, draw,
    enums::{Color, Event, Font, FrameType},
    frame::Frame,
    group::{Scroll, ScrollType},
    image::PngImage,
    prelude::*,
};

use crate::emoji::EMOJI_PNG;

const EMO: i32 = 20;

thread_local! {
    static CACHE: RefCell<HashMap<usize, PngImage>> = RefCell::new(HashMap::new());
}

pub fn emoji_image(i: usize, size: i32) -> Option<PngImage> {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if !c.contains_key(&i) {
            let img = PngImage::from_data(EMOJI_PNG[i].1).ok()?;
            c.insert(i, img);
        }
        let mut img = c.get(&i)?.clone();
        img.scale(size, size, true, true);
        Some(img)
    })
}

fn strip_vs(s: &str) -> String {
    s.chars().filter(|&c| c != '\u{FE0F}').collect()
}

enum Tok {
    Text(String),
    Emoji(usize),
    Newline,
}

fn match_at(rest: &str, k: &str) -> Option<usize> {
    let mut kc = k.chars().peekable();
    let mut end = 0;
    for (i, ch) in rest.char_indices() {
        if ch == '\u{FE0F}' {
            end = i + ch.len_utf8();
            continue;
        }
        match kc.peek() {
            None => break,
            Some(&c) if c == ch => {
                kc.next();
                end = i + ch.len_utf8();
            }
            Some(_) => return None,
        }
    }
    if kc.peek().is_none() && end > 0 { Some(end) } else { None }
}

fn tokenize(text: &str) -> Vec<Tok> {
    let keys: Vec<String> = EMOJI_PNG.iter().map(|(e, _)| strip_vs(e)).collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut rest = text;
    while !rest.is_empty() {
        let best = keys
            .iter()
            .enumerate()
            .filter_map(|(i, k)| match_at(rest, k).map(|e| (i, k.chars().count(), e)))
            .max_by_key(|x| x.1);
        if let Some((i, _, e)) = best {
            if !cur.is_empty() {
                out.push(Tok::Text(std::mem::take(&mut cur)));
            }
            out.push(Tok::Emoji(i));
            rest = &rest[e..];
            continue;
        }
        let ch = rest.chars().next().unwrap();
        rest = &rest[ch.len_utf8()..];
        match ch {
            '\n' => {
                if !cur.is_empty() {
                    out.push(Tok::Text(std::mem::take(&mut cur)));
                }
                out.push(Tok::Newline);
            }
            ' ' => {
                cur.push(' ');
                out.push(Tok::Text(std::mem::take(&mut cur)));
            }
            '\r' => {}
            '\t' => cur.push_str("    "),
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(Tok::Text(cur));
    }
    out
}

/// Dispone (e se `paint` disegna) il testo; restituisce l'altezza occupata.
fn layout(toks: &[Tok], x0: i32, y0: i32, w: i32, size: i32, paint: bool) -> i32 {
    draw::set_font(Font::Helvetica, size);
    let lh = (draw::height()).max(EMO + 2);
    let desc = draw::descent();
    let (mut x, mut y) = (x0, y0);
    let right = x0 + w;
    for t in toks {
        match t {
            Tok::Newline => {
                x = x0;
                y += lh;
            }
            Tok::Emoji(i) => {
                if x + EMO > right && x > x0 {
                    x = x0;
                    y += lh;
                }
                if paint {
                    if let Some(mut img) = emoji_image(*i, EMO) {
                        img.draw(x, y + (lh - EMO) / 2, EMO, EMO);
                    }
                }
                x += EMO + 1;
            }
            Tok::Text(s) => {
                let mut tw = draw::width(s) as i32;
                if x + tw > right && x > x0 {
                    x = x0;
                    y += lh;
                }
                // parola più lunga della riga: spezzala
                let mut s = s.as_str();
                while tw > w && !s.is_empty() {
                    let mut cut = s.len();
                    while cut > 0 && (draw::width(&s[..cut]) as i32) > right - x {
                        cut = s[..cut].char_indices().last().map(|(i, _)| i).unwrap_or(0);
                    }
                    if cut == 0 {
                        cut = s.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                    }
                    if paint {
                        draw::draw_text2(&s[..cut], x, y + lh - desc - (lh - draw::height()) / 2, 0, 0, fltk::enums::Align::Left);
                    }
                    s = &s[cut..];
                    x = x0;
                    y += lh;
                    tw = draw::width(s) as i32;
                }
                if paint {
                    draw::draw_text2(s, x, y + lh - desc - (lh - draw::height()) / 2, 0, 0, fltk::enums::Align::Left);
                }
                x += tw;
            }
        }
    }
    y + lh - y0
}

#[derive(Clone)]
pub struct RichView {
    pub scroll: Scroll,
    canvas: Frame,
    text: Rc<RefCell<String>>,
}

impl RichView {
    pub fn new() -> RichView {
        let mut scroll = Scroll::default();
        scroll.set_type(ScrollType::Vertical);
        scroll.set_frame(FrameType::DownBox);
        scroll.set_color(Color::White);
        scroll.set_scrollbar_size(12);
        let mut canvas = Frame::default();
        scroll.end();
        let text = Rc::new(RefCell::new(String::new()));
        {
            let t = text.clone();
            canvas.draw(move |f| {
                draw::set_draw_color(Color::Black);
                let toks = tokenize(&t.borrow());
                draw::push_clip(f.x(), f.y(), f.w(), f.h());
                layout(&toks, f.x() + 6, f.y() + 4, f.w() - 12, 14, true);
                draw::pop_clip();
            });
        }
        {
            let t = text.clone();
            canvas.handle(move |f, ev| match ev {
                // Ctrl+C copia tutto il testo del messaggio
                Event::Push => {
                    let _ = f.take_focus();
                    true
                }
                Event::Focus | Event::Unfocus => true,
                Event::KeyDown
                    if app::event_state().contains(fltk::enums::Shortcut::Ctrl)
                        && (app::event_key() == fltk::enums::Key::from_char('c')) =>
                {
                    app::copy(&t.borrow());
                    true
                }
                _ => false,
            });
        }
        let rv = RichView { scroll, canvas, text };
        {
            let mut me = rv.clone();
            rv.scroll.clone().resize_callback(move |_, _, _, _, _| me.relayout());
        }
        rv
    }

    pub fn set_text(&mut self, s: &str) {
        *self.text.borrow_mut() = s.to_string();
        self.relayout();
    }

    fn relayout(&mut self) {
        let s = &self.scroll;
        let w = s.w() - 4 - s.scrollbar_size();
        if w <= 0 {
            return;
        }
        // misura (serve un font impostato: fuori dal disegno va bene lo stesso)
        let toks = tokenize(&self.text.borrow());
        let h = layout(&toks, 0, 0, w - 12, 14, false) + 8;
        self.canvas.resize(s.x() + 2, s.y() + 2 - s.yposition(), w, h.max(s.h() - 4));
        self.scroll.redraw();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tok() {
        let t = tokenize("ciao 😊 ok ❤️ fine❤\nx");
        let em: Vec<usize> = t.iter().filter_map(|t| if let Tok::Emoji(i) = t { Some(*i) } else { None }).collect();
        assert_eq!(em.len(), 3);
        assert_eq!(EMOJI_PNG[em[0]].0, "😊");
        assert_eq!(em[1], em[2]);
    }
}
