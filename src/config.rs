//! Impostazioni (file chiave=valore), chiave RSA e registro dei messaggi.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use crate::protocol::DEFAULT_PORT;

pub const ABSENCE_PRESETS: &[(&str, &str)] = &[
    ("Assente", "Sono assente in questo momento."),
    ("In riunione", "Sono in riunione, ti rispondo appena posso."),
    ("In pausa", "Sono in pausa, torno a breve."),
    ("Fuori ufficio", "Sono fuori ufficio."),
];

pub fn config_dir() -> PathBuf {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    let dir = base.unwrap_or_else(|| PathBuf::from(".")).join(if cfg!(windows) || cfg!(target_os = "macos") {
        "POPMSG"
    } else {
        "popmsg"
    });
    let _ = fs::create_dir_all(&dir);
    dir
}

fn downloads_dir() -> String {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let d = home.join("Downloads");
    if d.is_dir() { d } else { home }.to_string_lossy().into_owned()
}

#[derive(Clone)]
pub struct Config {
    pub nick: String,
    pub group: String,
    pub port: u16,
    pub extra: Vec<String>,
    pub log_enabled: bool,
    pub log_path: String,
    pub sound: bool,
    pub popup: bool,
    pub quote: bool,
    pub seal: bool,
    pub encryption: bool,
    pub offline: bool,
    pub download_dir: String,
    pub absence_text: String,
}

impl Config {
    fn path() -> PathBuf {
        config_dir().join("popmsg.ini")
    }

    pub fn load() -> Config {
        let mut m = BTreeMap::new();
        if let Ok(t) = fs::read_to_string(Self::path()) {
            for line in t.lines() {
                if let Some((k, v)) = line.split_once('=') {
                    m.insert(k.trim().to_string(), unescape(v.trim()));
                }
            }
        }
        let s = |k: &str, d: &str| m.get(k).cloned().unwrap_or_else(|| d.to_string());
        let b = |k: &str, d: bool| m.get(k).map(|v| v == "1" || v == "true").unwrap_or(d);
        Config {
            nick: s("nick", &crate::net::login_name()),
            group: s("group", ""),
            port: m.get("port").and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_PORT),
            extra: s("extra", "").split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
            log_enabled: b("log", true),
            log_path: s("log_path", &config_dir().join("popmsg.log").to_string_lossy()),
            sound: b("sound", true),
            popup: b("popup", true),
            quote: b("quote", true),
            seal: b("seal", false),
            encryption: b("encryption", true),
            offline: b("offline", true),
            download_dir: s("download_dir", &downloads_dir()),
            absence_text: s("absence_text", ABSENCE_PRESETS[0].1),
        }
    }

    pub fn save(&self) {
        let bl = |b: bool| if b { "1" } else { "0" };
        let lines = [
            ("nick", self.nick.clone()),
            ("group", self.group.clone()),
            ("port", self.port.to_string()),
            ("extra", self.extra.join(",")),
            ("log", bl(self.log_enabled).into()),
            ("log_path", self.log_path.clone()),
            ("sound", bl(self.sound).into()),
            ("popup", bl(self.popup).into()),
            ("quote", bl(self.quote).into()),
            ("seal", bl(self.seal).into()),
            ("encryption", bl(self.encryption).into()),
            ("offline", bl(self.offline).into()),
            ("download_dir", self.download_dir.clone()),
            ("absence_text", self.absence_text.clone()),
        ];
        let text: String = lines.iter().map(|(k, v)| format!("{k}={}\n", escape(v))).collect();
        let _ = fs::write(Self::path(), text);
    }
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\n', "\\n")
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') => out.push('\n'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

// ---- rubrica locale dei numeri di telefono (solo su questo computer) ----

thread_local! {
    static BOOK: std::cell::RefCell<Option<BTreeMap<String, String>>> = const { std::cell::RefCell::new(None) };
}

fn book_path() -> PathBuf {
    config_dir().join("rubrica.ini")
}

fn with_book<R>(f: impl FnOnce(&mut BTreeMap<String, String>) -> R) -> R {
    BOOK.with(|b| {
        let mut b = b.borrow_mut();
        let m = b.get_or_insert_with(|| {
            let mut m = BTreeMap::new();
            if let Ok(t) = fs::read_to_string(book_path()) {
                for line in t.lines() {
                    if let Some((k, v)) = line.split_once('=') {
                        m.insert(unescape(k.trim()), unescape(v.trim()));
                    }
                }
            }
            m
        });
        f(m)
    })
}

pub fn tel_get(key: &str) -> String {
    with_book(|m| m.get(key).cloned().unwrap_or_default())
}

pub fn tel_set(key: &str, value: &str) {
    with_book(|m| {
        if value.trim().is_empty() {
            m.remove(key);
        } else {
            m.insert(key.to_string(), value.trim().to_string());
        }
        let text: String = m.iter().map(|(k, v)| format!("{}={}\n", escape(&k.replace('=', "_")), escape(v))).collect();
        let _ = fs::write(book_path(), text);
    })
}

/// Carica (o crea al primo avvio) la chiave RSA personale.
pub fn load_or_create_key() -> Option<rsa::RsaPrivateKey> {
    let p = config_dir().join("key.pem");
    if let Some(k) = fs::read_to_string(&p).ok().and_then(|s| crate::crypto::from_pem(&s)) {
        return Some(k);
    }
    let k = crate::crypto::generate();
    let _ = fs::write(&p, crate::crypto::to_pem(&k));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&p, fs::Permissions::from_mode(0o600));
    }
    Some(k)
}

pub fn write_log(path: &str, dir: &str, who: &str, text: &str, flags: &[&str], files: &[String]) {
    let when = chrono::Local::now().format("%d/%m/%Y %H:%M:%S");
    let mut s = format!("{}\n {dir}: {who}", "=".repeat(38));
    if !flags.is_empty() {
        s.push_str(&format!("  ({})", flags.join(", ")));
    }
    s.push_str(&format!("\n  alle {when}\n"));
    if !files.is_empty() {
        s.push_str(&format!("  Allegati: {}\n", files.join(", ")));
    }
    s.push_str(&format!("{}\n{}\n\n", "-".repeat(38), text.trim_end_matches('\n')));
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(s.as_bytes());
    }
}

#[cfg(windows)]
pub fn autostart_enabled() -> bool {
    std::process::Command::new("reg")
        .args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", "/v", "POPMSG"])
        .creation_flags(0x0800_0000)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(windows)]
pub fn set_autostart(on: bool) {
    let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    let exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let mut c = std::process::Command::new("reg");
    if on {
        c.args(["add", key, "/v", "POPMSG", "/t", "REG_SZ", "/d", &format!("\"{exe}\""), "/f"]);
    } else {
        c.args(["delete", key, "/v", "POPMSG", "/f"]);
    }
    let _ = c.creation_flags(0x0800_0000).output();
}

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(not(windows))]
pub fn autostart_enabled() -> bool {
    false
}

#[cfg(not(windows))]
pub fn set_autostart(_on: bool) {}

// ---- utenti conosciuti (per mostrarli in grigio quando non sono in linea) ----

#[derive(Clone, PartialEq)]
pub struct Known {
    pub key: String,
    pub login: String,
    pub host: String,
    pub nick: String,
    pub group: String,
    pub addr: String,
}

fn esc_field(s: &str) -> String {
    escape(s).replace('\t', "\\t")
}

fn unesc_field(s: &str) -> String {
    unescape(&s.replace("\\t", "\t"))
}

fn read_rows(name: &str) -> Vec<Vec<String>> {
    fs::read_to_string(config_dir().join(name))
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.split('\t').map(unesc_field).collect())
        .collect()
}

fn write_rows(name: &str, rows: impl Iterator<Item = Vec<String>>) {
    let text: String = rows
        .map(|r| r.iter().map(|f| esc_field(f)).collect::<Vec<_>>().join("\t") + "\n")
        .collect();
    let _ = fs::write(config_dir().join(name), text);
}

thread_local! {
    static KNOWN: std::cell::RefCell<Option<BTreeMap<String, Known>>> = const { std::cell::RefCell::new(None) };
    static OUTBOX: std::cell::RefCell<Option<Vec<Queued>>> = const { std::cell::RefCell::new(None) };
}

fn with_known<R>(f: impl FnOnce(&mut BTreeMap<String, Known>) -> R) -> R {
    KNOWN.with(|k| {
        let mut k = k.borrow_mut();
        let m = k.get_or_insert_with(|| {
            read_rows("utenti.txt")
                .into_iter()
                .filter(|r| r.len() >= 6)
                .map(|r| {
                    (r[0].clone(), Known {
                        key: r[0].clone(),
                        login: r[1].clone(),
                        host: r[2].clone(),
                        nick: r[3].clone(),
                        group: r[4].clone(),
                        addr: r[5].clone(),
                    })
                })
                .collect()
        });
        f(m)
    })
}

fn save_known(m: &BTreeMap<String, Known>) {
    write_rows(
        "utenti.txt",
        m.values().map(|k| vec![k.key.clone(), k.login.clone(), k.host.clone(), k.nick.clone(), k.group.clone(), k.addr.clone()]),
    );
}

pub fn known_all() -> Vec<Known> {
    with_known(|m| m.values().cloned().collect())
}

pub fn known_update(list: &[Known]) {
    with_known(|m| {
        let mut changed = false;
        for k in list {
            if m.get(&k.key) != Some(k) {
                m.insert(k.key.clone(), k.clone());
                changed = true;
            }
        }
        if changed {
            save_known(m);
        }
    })
}

pub fn known_remove(key: &str) {
    with_known(|m| {
        if m.remove(key).is_some() {
            save_known(m);
        }
    })
}

// ---- messaggi in attesa per utenti non in linea ----

#[derive(Clone)]
pub struct Queued {
    pub id: u64,
    pub key: String,
    pub name: String,
    pub text: String,
    pub sealed: bool,
    pub written: String,
    pub files: Vec<String>,
}

fn with_outbox<R>(f: impl FnOnce(&mut Vec<Queued>) -> R) -> R {
    OUTBOX.with(|o| {
        let mut o = o.borrow_mut();
        let v = o.get_or_insert_with(|| {
            read_rows("in_attesa.txt")
                .into_iter()
                .filter(|r| r.len() >= 6)
                .map(|r| Queued {
                    id: r[0].parse().unwrap_or(0),
                    key: r[1].clone(),
                    name: r[2].clone(),
                    sealed: r[3] == "1",
                    written: r[4].clone(),
                    text: r[5].clone(),
                    files: r[6..].to_vec(),
                })
                .collect()
        });
        f(v)
    })
}

fn save_outbox(v: &[Queued]) {
    write_rows(
        "in_attesa.txt",
        v.iter().map(|q| {
            let mut r = vec![
                q.id.to_string(),
                q.key.clone(),
                q.name.clone(),
                if q.sealed { "1".into() } else { "0".into() },
                q.written.clone(),
                q.text.clone(),
            ];
            r.extend(q.files.iter().cloned());
            r
        }),
    );
}

pub fn outbox_all() -> Vec<Queued> {
    with_outbox(|v| v.clone())
}

pub fn outbox_add(key: &str, name: &str, text: &str, sealed: bool, files: &[String]) {
    with_outbox(|v| {
        let id = v.iter().map(|q| q.id).max().unwrap_or(0) + 1;
        v.push(Queued {
            id,
            key: key.to_string(),
            name: name.to_string(),
            text: text.to_string(),
            sealed,
            written: chrono::Local::now().format("%d/%m/%Y %H:%M").to_string(),
            files: files.to_vec(),
        });
        save_outbox(v);
    })
}

pub fn outbox_remove(id: u64) {
    with_outbox(|v| {
        v.retain(|q| q.id != id);
        save_outbox(v);
    })
}
