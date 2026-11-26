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
    /// impronta della password per aprire i messaggi sigillati ("" = nessuna)
    pub password: String,
    /// chiedi la password per tutti i messaggi sigillati, non solo per quelli col lucchetto
    pub password_all: bool,
    /// scambia automaticamente l'elenco utenti con gli altri POPMSG
    pub share_list: bool,
    /// usa il formato v5 di IP Messenger (IPDict) con chi lo supporta
    pub ipdict: bool,
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
            // dalla versione 2.1 i messaggi partono sigillati di base
            seal: b("seal2", true),
            encryption: b("encryption", true),
            offline: b("offline", true),
            download_dir: s("download_dir", &downloads_dir()),
            absence_text: s("absence_text", ABSENCE_PRESETS[0].1),
            password: s("password", ""),
            password_all: b("password_all", false),
            share_list: b("share_list", true),
            ipdict: b("ipdict", false),
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
            ("seal2", bl(self.seal).into()),
            ("encryption", bl(self.encryption).into()),
            ("offline", bl(self.offline).into()),
            ("download_dir", self.download_dir.clone()),
            ("absence_text", self.absence_text.clone()),
            ("password", self.password.clone()),
            ("password_all", bl(self.password_all).into()),
            ("share_list", bl(self.share_list).into()),
            ("ipdict", bl(self.ipdict).into()),
        ];
        let text: String = lines.iter().map(|(k, v)| format!("{k}={}\n", escape(v))).collect();
        let _ = fs::write(Self::path(), text);
    }
}

/// Impronta (con sale) della password per i messaggi sigillati.
pub fn password_hash(pw: &str) -> String {
    use sha2::{Digest, Sha256};
    let salt: [u8; 8] = rand::random();
    let salt = hex::encode(salt);
    let d = Sha256::digest(format!("{salt}:{pw}").as_bytes());
    format!("{salt}${}", hex::encode(d))
}

pub fn password_check(stored: &str, pw: &str) -> bool {
    use sha2::{Digest, Sha256};
    let Some((salt, h)) = stored.split_once('$') else { return false };
    hex::encode(Sha256::digest(format!("{salt}:{pw}").as_bytes())) == h
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
    /// 0 = normale, 1 = sigillato, 2 = sigillato con lucchetto
    pub sealed: u8,
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
                    sealed: r[3].parse().unwrap_or(0),
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
                q.sealed.to_string(),
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

pub fn outbox_add(key: &str, name: &str, text: &str, sealed: u8, files: &[String]) {
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

// ---- gruppi di invio personali (solo su questo computer) ----

/// (nome del gruppo, chiavi degli utenti)
pub fn groups_all() -> Vec<(String, Vec<String>)> {
    let mut v: Vec<(String, Vec<String>)> = read_rows("gruppi.txt")
        .into_iter()
        .filter(|r| !r.is_empty() && !r[0].is_empty())
        .map(|r| (r[0].clone(), r[1..].to_vec()))
        .collect();
    v.sort_by_key(|g| g.0.to_lowercase());
    v
}

pub fn group_save(name: &str, keys: &[String]) {
    let mut all = groups_all();
    all.retain(|g| !g.0.eq_ignore_ascii_case(name));
    all.push((name.to_string(), keys.to_vec()));
    write_rows("gruppi.txt", all.into_iter().map(|(n, k)| std::iter::once(n).chain(k).collect()));
}

pub fn group_delete(name: &str) {
    let mut all = groups_all();
    all.retain(|g| g.0 != name);
    write_rows("gruppi.txt", all.into_iter().map(|(n, k)| std::iter::once(n).chain(k).collect()));
}

// ---- aspetto della tabella utenti (larghezze, colonne nascoste, ordinamento) ----

pub const COLS: [&str; 5] = ["Utente", "Gruppo", "Tel", "Host", "Indirizzo IP"];
pub const COL_DEFAULT: [i32; 5] = [130, 110, 90, 80, 110];

#[derive(Clone)]
pub struct TableLayout {
    /// larghezza delle colonne; negativa = nascosta (con la larghezza da ripristinare)
    pub widths: [i32; 5],
    pub sort_col: usize,
    pub sort_asc: bool,
}

pub fn table_layout() -> TableLayout {
    let mut t = TableLayout { widths: COL_DEFAULT, sort_col: 1, sort_asc: true };
    if let Some(r) = read_rows("tabella.txt").into_iter().next() {
        for (i, w) in r.iter().take(5).enumerate() {
            if let Ok(v) = w.parse::<i32>() {
                if v.abs() >= 20 {
                    t.widths[i] = v;
                }
            }
        }
        t.sort_col = r.get(5).and_then(|v| v.parse().ok()).filter(|c: &usize| *c < 5).unwrap_or(1);
        t.sort_asc = r.get(6).map(|v| v != "0").unwrap_or(true);
    }
    t.widths[0] = t.widths[0].abs(); // "Utente" è sempre visibile
    t
}

pub fn save_table_layout(t: &TableLayout) {
    let mut r: Vec<String> = t.widths.iter().map(|w| w.to_string()).collect();
    r.push(t.sort_col.to_string());
    r.push(if t.sort_asc { "1".into() } else { "0".into() });
    write_rows("tabella.txt", std::iter::once(r));
}

// ---- emoticon preferite (le più usate) ----

const DEFAULT_FAV: &[&str] = &["😊", "👍", "😂", "❤️", "🙏", "😉", "👋", "😍", "🎉", "✅", "😢", "🤔"];

pub fn favorites(n: usize) -> Vec<String> {
    let mut used: Vec<(String, u64)> = read_rows("emoticon.txt")
        .into_iter()
        .filter(|r| r.len() >= 2)
        .map(|r| (r[0].clone(), r[1].parse().unwrap_or(0)))
        .collect();
    used.sort_by(|a, b| b.1.cmp(&a.1));
    let mut out: Vec<String> = used.into_iter().map(|u| u.0).take(n).collect();
    for d in DEFAULT_FAV {
        if out.len() >= n {
            break;
        }
        if !out.iter().any(|e| e == d) {
            out.push(d.to_string());
        }
    }
    out
}

pub fn emoji_used(e: &str) {
    let mut rows = read_rows("emoticon.txt");
    match rows.iter_mut().find(|r| r.first().map(|x| x == e).unwrap_or(false)) {
        Some(r) => {
            let n: u64 = r.get(1).and_then(|v| v.parse().ok()).unwrap_or(0) + 1;
            r.truncate(1);
            r.push(n.to_string());
        }
        None => rows.push(vec![e.to_string(), "1".into()]),
    }
    write_rows("emoticon.txt", rows.into_iter());
}

// ---- utenti aggiunti a mano, importazione ed esportazione dell'elenco ----

/// le voci aggiunte a mano non hanno il nome di login: chiave "@computer" o "@IP"
pub const MANUAL_PREFIX: &str = "@";

/// Aggiunge un utente che non ha ancora mai aperto POPMSG; restituisce la chiave.
/// Quando comparirà in rete verrà riconosciuto dal nome del computer o dall'IP.
pub fn known_add_manual(nick: &str, group: &str, host: &str, ip: &str) -> String {
    let id = if host.trim().is_empty() { ip.trim().to_string() } else { host.trim().to_string() };
    let key = format!("{MANUAL_PREFIX}{id}");
    let addr = ip
        .trim()
        .parse::<std::net::IpAddr>()
        .map(|i| std::net::SocketAddr::new(i, DEFAULT_PORT).to_string())
        .unwrap_or_default();
    known_update(&[Known {
        key: key.clone(),
        login: String::new(),
        host: id,
        nick: nick.trim().to_string(),
        group: group.trim().to_string(),
        addr,
    }]);
    key
}

/// Utente aggiunto a mano che corrisponde a chi è appena comparso in rete.
pub fn manual_match(host: &str, ip: std::net::IpAddr) -> Option<String> {
    with_known(|m| {
        m.values()
            .filter(|k| k.key.starts_with(MANUAL_PREFIX))
            .find(|k| k.host.eq_ignore_ascii_case(host) || k.host.parse::<std::net::IpAddr>().map(|a| a == ip).unwrap_or(false))
            .map(|k| k.key.clone())
    })
}

/// Sposta su una nuova chiave tutto ciò che riguarda un utente:
/// voce dell'elenco, Tel, messaggi in attesa, gruppi.
pub fn known_rekey(old: &str, new: &str) {
    if old == new {
        return;
    }
    with_known(|m| {
        if m.remove(old).is_some() {
            save_known(m);
        }
    });
    let tel = tel_get(old);
    if !tel.is_empty() {
        if tel_get(new).is_empty() {
            tel_set(new, &tel);
        }
        tel_set(old, "");
    }
    with_outbox(|v| {
        let mut changed = false;
        for q in v.iter_mut().filter(|q| q.key == old) {
            q.key = new.to_string();
            changed = true;
        }
        if changed {
            save_outbox(v);
        }
    });
    let groups = groups_all();
    if groups.iter().any(|g| g.1.iter().any(|k| k == old)) {
        let rows = groups.into_iter().map(|(n, ks)| {
            let mut ks: Vec<String> = ks.into_iter().map(|k| if k == old { new.to_string() } else { k }).collect();
            ks.dedup();
            std::iter::once(n).chain(ks).collect()
        });
        write_rows("gruppi.txt", rows);
    }
}

/// Elenco degli utenti conosciuti da mandare a un altro POPMSG (senza Tel).
pub fn known_list_text() -> String {
    known_all()
        .iter()
        .map(|k| [&k.key, &k.login, &k.host, &k.nick, &k.group, &k.addr].iter().map(|f| esc_field(f)).collect::<Vec<_>>().join("\t") + "\n")
        .collect()
}

/// Aggiunge gli utenti sconosciuti di un elenco ricevuto; restituisce quanti.
pub fn known_merge_text(text: &str) -> usize {
    let have: std::collections::HashSet<String> = known_all().into_iter().map(|k| k.key).collect();
    let new: Vec<Known> = text
        .lines()
        .map(|l| l.split('\t').map(unesc_field).collect::<Vec<_>>())
        .filter(|r| r.len() >= 6 && !r[0].is_empty() && !have.contains(&r[0]))
        // un utente aggiunto a mano da altri che qui conosciamo già con la chiave vera
        .filter(|r| !(r[0].starts_with(MANUAL_PREFIX) && known_all().iter().any(|k| k.host.eq_ignore_ascii_case(&r[2]))))
        .map(|r| Known { key: r[0].clone(), login: r[1].clone(), host: r[2].clone(), nick: r[3].clone(), group: r[4].clone(), addr: r[5].clone() })
        .collect();
    known_update(&new);
    new.len()
}

const EXPORT_HEADER: &str = "# POPMSG elenco utenti";

/// Scrive l'elenco degli utenti conosciuti in un file da passare agli altri PC,
/// con i numeri di Tel se richiesto.
pub fn known_export(path: &std::path::Path, with_tel: bool) -> std::io::Result<usize> {
    let all = known_all();
    let mut text = format!("{EXPORT_HEADER}\n");
    for k in &all {
        let tel = if with_tel { tel_get(&k.key) } else { String::new() };
        let r = [&k.key, &k.login, &k.host, &k.nick, &k.group, &k.addr, &tel];
        text.push_str(&r.iter().map(|f| esc_field(f)).collect::<Vec<_>>().join("\t"));
        text.push('\n');
    }
    fs::write(path, text)?;
    Ok(all.len())
}

/// Aggiunge gli utenti di un file esportato e i numeri di Tel che qui mancano;
/// restituisce (utenti nuovi, numeri di Tel aggiunti).
pub fn known_import(path: &std::path::Path) -> Result<(usize, usize), String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    if !text.starts_with(EXPORT_HEADER) {
        return Err("il file non è un elenco utenti di POPMSG".into());
    }
    let have: std::collections::HashSet<String> = known_all().into_iter().map(|k| k.key).collect();
    let rows: Vec<Vec<String>> = text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split('\t').map(unesc_field).collect::<Vec<_>>())
        .filter(|r| r.len() >= 6 && !r[0].is_empty())
        .collect();
    // i numeri già scritti su questo computer non vengono sovrascritti
    let mut tels = 0;
    for r in &rows {
        if let Some(t) = r.get(6).filter(|t| !t.trim().is_empty()) {
            if tel_get(&r[0]).is_empty() {
                tel_set(&r[0], t);
                tels += 1;
            }
        }
    }
    let new: Vec<Known> = rows
        .iter()
        .filter(|r| !have.contains(&r[0]))
        .map(|r| Known {
            key: r[0].clone(),
            login: r[1].clone(),
            host: r[2].clone(),
            nick: r[3].clone(),
            group: r[4].clone(),
            addr: r[5].clone(),
        })
        .collect();
    known_update(&new);
    Ok((new.len(), tels))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn manual_users_export_import() {
        let dir = std::env::temp_dir().join(format!("popmsg-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };
        let k = known_add_manual("Marta", "Commerciale", "", "10.0.0.7");
        assert_eq!(k, "@10.0.0.7");
        tel_set(&k, "123");
        outbox_add(&k, "Marta", "ciao", 0, &[]);
        group_save("G", &[k.clone()]);
        let ip: std::net::IpAddr = "10.0.0.7".parse().unwrap();
        assert_eq!(manual_match("pc-marta", ip).as_deref(), Some(k.as_str()));
        known_rekey(&k, "marta@pc-marta");
        assert!(manual_match("pc-marta", ip).is_none());
        assert_eq!(tel_get("marta@pc-marta"), "123");
        assert_eq!(outbox_all()[0].key, "marta@pc-marta");
        assert_eq!(groups_all()[0].1, vec!["marta@pc-marta".to_string()]);
        known_add_manual("Luca", "", "PC-LUCA", "");
        assert!(manual_match("pc-luca", "10.0.0.9".parse().unwrap()).is_some());
        let f = dir.join("export.txt");
        tel_set("@PC-LUCA", "333 1234567");
        assert_eq!(known_export(&f, true).unwrap(), 1);
        known_remove("@PC-LUCA");
        tel_set("@PC-LUCA", "");
        assert_eq!(known_import(&f).unwrap(), (1, 1));
        assert_eq!(tel_get("@PC-LUCA"), "333 1234567");
        assert_eq!(known_import(&f).unwrap(), (0, 0));
        known_export(&f, false).unwrap();
        tel_set("@PC-LUCA", "");
        assert_eq!(known_import(&f).unwrap(), (0, 0));
        let _ = fs::remove_dir_all(&dir);
    }
}
