#![windows_subsystem = "windows"]
#![allow(dead_code)]
//! POPMSG: messaggistica LAN compatibile con IP Messenger.

mod config;
mod crypto;
mod emoji;
mod net;
#[cfg(test)]
mod net_tests;
mod protocol;
mod rich;
mod tray;
mod ui;

use fltk::{app, dialog, enums::Font};

#[derive(Debug, Clone)]
pub enum Msg {
    Net(net::Event),
    Tray(tray::TrayCmd),
    TrayClick,
}

fn main() {
    let app = app::App::default().with_scheme(app::Scheme::Base);
    if cfg!(windows) {
        Font::set_font(Font::Helvetica, "Segoe UI");
        Font::set_font(Font::HelveticaBold, "Segoe UI Bold");
    }
    app::set_font_size(13);
    app::set_visible_focus(false);
    let (tx, rx) = app::channel::<Msg>();

    let cfg = config::Config::load();
    let key = config::load_or_create_key();
    let tx2 = tx;
    let engine = match net::Engine::start(
        net::Options {
            nick: cfg.nick.clone(),
            group: cfg.group.clone(),
            port: cfg.port,
            extra: cfg.extra.clone(),
            key,
            encryption: cfg.encryption,
        },
        move |ev| tx2.send(Msg::Net(ev)),
    ) {
        Ok(e) => e,
        Err(e) => {
            dialog::alert_default(&format!(
                "Impossibile avviare POPMSG ({e}).\nForse è già in esecuzione POPMSG o IP Messenger su questo computer."
            ));
            return;
        }
    };
    engine.set_absence(false, Some(&cfg.absence_text));
    let ctl = ui::Ctl::new(engine, cfg);

    let show = std::env::args().any(|a| a == "--show");
    // l'icona di sistema va creata a ciclo eventi avviato (necessario su macOS)
    {
        let c = ctl.clone();
        app::add_timeout3(0.05, move |_| {
            ui::start_tray(&c, tx);
            if c.tray.borrow().is_none() || show {
                c.open_send(&[], "");
            }
        });
    }
    // controlla gli eventi dell'icona di sistema (Windows/macOS)
    {
        let c = ctl.clone();
        app::add_timeout3(0.2, move |h| {
            let msgs = c.tray.borrow().as_ref().map(|t| t.poll()).unwrap_or_default();
            for m in msgs {
                tx.send(m);
            }
            app::repeat_timeout3(0.2, h);
        });
    }
    let mut started = false;
    let _ = &app;
    loop {
        let _ = app::wait_for(0.25);
        while let Some(m) = rx.recv() {
            match m {
                Msg::Net(ev) => ui::handle_net(&ctl, ev),
                Msg::Tray(c) => ctl.tray_cmd(c),
                Msg::TrayClick => {
                    if !ctl.show_unread() && !cfg!(windows) {
                        ctl.open_send(&[], "");
                    }
                }
            }
        }
        if ctl.any_window() {
            started = true;
        } else if started && ctl.tray.borrow().is_none() {
            ctl.quit();
        }
    }
}
