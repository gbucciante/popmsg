//! Prova end-to-end: tre motori su porte diverse in locale.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use crate::crypto;
use crate::net::{Engine, Event, Message, Options};
use crate::protocol as p;

fn eng(nick: &str, port: u16, peers: &[u16], key: bool) -> (Engine, Receiver<Event>) {
    let (tx, rx) = channel();
    let tx = std::sync::Mutex::new(tx);
    let e = Engine::start(
        Options {
            nick: nick.into(),
            group: format!("G-{nick}"),
            port,
            extra: peers.iter().map(|p| format!("127.0.0.1:{p}")).collect(),
            key: if key { Some(crypto::generate()) } else { None },
            encryption: true,
        },
        move |ev| {
            let _ = tx.lock().unwrap().send(ev);
        },
    )
    .unwrap();
    (e, rx)
}

fn wait<T>(rx: &Receiver<Event>, mut f: impl FnMut(&Event) -> Option<T>) -> T {
    let end = Instant::now() + Duration::from_secs(8);
    while Instant::now() < end {
        if let Ok(ev) = rx.recv_timeout(Duration::from_millis(100)) {
            if let Some(v) = f(&ev) {
                return v;
            }
        }
    }
    panic!("timeout");
}

fn until(mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(8);
    while Instant::now() < end {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timeout");
}

fn msg(rx: &Receiver<Event>) -> Message {
    wait(rx, |e| if let Event::Message(m) = e { Some((**m).clone()) } else { None })
}

#[test]
fn end_to_end() {
    let (a, ra) = eng("Alice", 25301, &[25302, 25303], true);
    let (b, rb) = eng("Bruno", 25302, &[25301], true);
    a.broadcast_entry();
    until(|| a.users().iter().any(|u| u.nick == "Bruno") && b.users().iter().any(|u| u.nick == "Alice"));
    let ub = a.users().into_iter().find(|u| u.nick == "Bruno").unwrap();
    assert_eq!(ub.group, "G-Bruno");
    assert!(a.will_encrypt(&ub));

    // messaggio sigillato e cifrato + avviso di apertura
    let no = a.send_message(&[ub.clone()], "Ciao Bruno, è sigillato 😊", true, &[])[0];
    let m = msg(&rb);
    assert_eq!(m.text, "Ciao Bruno, è sigillato 😊");
    assert!(m.sealed() && m.encrypted);
    wait(&ra, |e| matches!(e, Event::Delivered(n) if *n == no).then_some(()));
    b.mark_read(&m);
    wait(&ra, |e| matches!(e, Event::Opened(n, _) if *n == no).then_some(()));

    // assenza
    b.set_absence(true, Some("Torno alle 15"));
    until(|| a.user(&ub.addr).map(|u| u.absent()).unwrap_or(false));
    a.request_absence(&ub);
    let t = wait(&ra, |e| if let Event::AbsenceInfo(_, t) = e { Some(t.clone()) } else { None });
    assert_eq!(t, "Torno alle 15");
    b.set_absence(false, None);

    // file e cartelle (elenco file dentro il messaggio cifrato)
    let src = std::env::temp_dir().join(format!("popmsg-src-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&src);
    std::fs::create_dir_all(src.join("cartella/sub")).unwrap();
    let big: Vec<u8> = (0..3_000_000u32).map(|i| (i * 7 % 251) as u8).collect();
    std::fs::write(src.join("doc.bin"), &big).unwrap();
    std::fs::write(src.join("cartella/uno.txt"), "uno").unwrap();
    std::fs::write(src.join("cartella/sub/due:tre.txt"), "due".repeat(1000)).unwrap();
    let att: Vec<PathBuf> = vec![src.join("doc.bin"), src.join("cartella")];
    a.send_message(&[ub.clone()], "allegati", false, &att);
    let m = msg(&rb);
    assert!(m.encrypted && m.command & p::ENCEXTMSGOPT != 0);
    assert_eq!(m.files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["doc.bin", "cartella"]);
    let dest = std::env::temp_dir().join(format!("popmsg-dst-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::create_dir_all(&dest).unwrap();
    let c = AtomicBool::new(false);
    let f = b.download(&m, &m.files[0], &dest, &mut |_, _, _| {}, &c).unwrap();
    assert_eq!(std::fs::read(f).unwrap(), big);
    let d = b.download(&m, &m.files[1], &dest, &mut |_, _, _| {}, &c).unwrap();
    assert_eq!(std::fs::read_to_string(d.join("uno.txt")).unwrap(), "uno");
    assert_eq!(std::fs::read_to_string(d.join("sub/due_tre.txt")).unwrap(), "due".repeat(1000));

    // destinatario senza cifratura: passa in chiaro
    let (c3, rc) = eng("Carla", 25303, &[25301], false);
    until(|| a.users().iter().any(|u| u.nick == "Carla"));
    let uc = a.users().into_iter().find(|u| u.nick == "Carla").unwrap();
    a.send_message(&[uc], "in chiaro", false, &[]);
    let m = msg(&rc);
    assert!(m.text == "in chiaro" && !m.encrypted);
    c3.stop();

    a.stop();
    until(|| !b.users().iter().any(|u| u.nick == "Alice"));
    b.stop();
}
