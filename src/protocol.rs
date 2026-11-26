//! Formato dei pacchetti del protocollo IP Messenger.
//!
//! `Ver:PacketNo:SenderName:SenderHost:CommandNo:AdditionalSection`

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_PORT: u16 = 2425;

// ---- comandi ----
pub const NOOPERATION: u32 = 0x00;
pub const BR_ENTRY: u32 = 0x01;
pub const BR_EXIT: u32 = 0x02;
pub const ANSENTRY: u32 = 0x03;
pub const BR_ABSENCE: u32 = 0x04;
pub const SENDMSG: u32 = 0x20;
pub const RECVMSG: u32 = 0x21;
pub const READMSG: u32 = 0x30;
pub const DELMSG: u32 = 0x31;
pub const ANSREADMSG: u32 = 0x32;
pub const GETINFO: u32 = 0x40;
pub const SENDINFO: u32 = 0x41;
pub const GETABSENCEINFO: u32 = 0x50;
pub const SENDABSENCEINFO: u32 = 0x51;
pub const GETFILEDATA: u32 = 0x60;
pub const RELEASEFILES: u32 = 0x61;
pub const GETDIRFILES: u32 = 0x62;
pub const GETPUBKEY: u32 = 0x72;
pub const ANSPUBKEY: u32 = 0x73;
/// estensione POPMSG (ignorata da IP Messenger): scambio dell'elenco utenti
pub const POPMSG_GETLIST: u32 = 0xF0;
pub const POPMSG_ANSLIST: u32 = 0xF1;
/// messaggio di una chat di gruppo tra POPMSG
pub const POPMSG_CHAT: u32 = 0xF2;

// ---- opzioni generali ----
pub const ABSENCEOPT: u32 = 0x0000_0100;
pub const SERVEROPT: u32 = 0x0000_0200;
pub const DIALUPOPT: u32 = 0x0001_0000;
pub const FILEATTACHOPT: u32 = 0x0020_0000;
pub const ENCRYPTOPT: u32 = 0x0040_0000;
pub const UTF8OPT: u32 = 0x0080_0000;
pub const CAPUTF8OPT: u32 = 0x0100_0000;
pub const ENCEXTMSGOPT: u32 = 0x0400_0000;
pub const CLIPBOARDOPT: u32 = 0x0800_0000;
/// supporta i pacchetti in formato v5 (IPDict, "IP2:")
pub const CAPIPDICTOPT: u32 = 0x0200_0000;

// ---- opzioni di SENDMSG ----
pub const SENDCHECKOPT: u32 = 0x0000_0100;
pub const SECRETOPT: u32 = 0x0000_0200;
pub const BROADCASTOPT: u32 = 0x0000_0400;
pub const MULTICASTOPT: u32 = 0x0000_0800;
pub const AUTORETOPT: u32 = 0x0000_2000;
pub const RETRYOPT: u32 = 0x0000_4000;
pub const PASSWORDOPT: u32 = 0x0000_8000;
pub const NOLOGOPT: u32 = 0x0002_0000;
pub const NOADDLISTOPT: u32 = 0x0008_0000;
pub const READCHECKOPT: u32 = 0x0010_0000;
pub const SECRETEXOPT: u32 = READCHECKOPT | SECRETOPT;

// ---- tipi di file ----
pub const FILE_REGULAR: u32 = 0x1;
pub const FILE_DIR: u32 = 0x2;
pub const FILE_RETPARENT: u32 = 0x3;
/// immagine incollata dagli appunti (mostrata dentro il messaggio)
pub const FILE_CLIPBOARD: u32 = 0x20;
pub const FILE_CLIPBOARDPOS: u32 = 0x8;
pub const FILE_MTIME: u32 = 0x14;

pub const FILELIST_SEPARATOR: char = '\u{7}';

pub fn mode(cmd: u32) -> u32 {
    cmd & 0xFF
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn next_packet_no() -> u64 {
    if COUNTER.load(Ordering::Relaxed) == 0 {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(1);
        let _ = COUNTER.compare_exchange(0, now, Ordering::SeqCst, Ordering::SeqCst);
    }
    COUNTER.fetch_add(1, Ordering::SeqCst)
}

/// Decodifica testo: UTF-8 se dichiarato, altrimenti UTF-8, Shift-JIS o Windows-1252.
pub fn decode(data: &[u8], utf8: bool) -> String {
    if utf8 {
        return String::from_utf8_lossy(data).into_owned();
    }
    if let Ok(s) = std::str::from_utf8(data) {
        return s.to_string();
    }
    let (s, _, bad) = encoding_rs::SHIFT_JIS.decode(data);
    if !bad {
        return s.into_owned();
    }
    encoding_rs::WINDOWS_1252.decode(data).0.into_owned()
}

#[derive(Debug, Clone)]
pub struct Packet {
    pub packet_no: u64,
    pub user: String,
    pub host: String,
    pub command: u32,
    /// sezione aggiuntiva grezza
    pub extra: Vec<u8>,
    /// testo fino al primo `\0`
    pub message: String,
    /// sezioni successive al primo `\0`
    pub tail: Vec<String>,
}

impl Packet {
    pub fn mode(&self) -> u32 {
        mode(self.command)
    }
    pub fn has(&self, flag: u32) -> bool {
        self.command & flag != 0
    }
}

pub fn build(packet_no: u64, user: &str, host: &str, command: u32, extra: &str, tail: &[String]) -> Vec<u8> {
    let command = command | UTF8OPT;
    let mut out = format!("1:{packet_no}:{user}:{host}:{command}:{extra}").into_bytes();
    for t in tail {
        out.push(0);
        out.extend_from_slice(t.as_bytes());
    }
    // come IP Messenger: il pacchetto termina con '\0'
    out.push(0);
    out
}

/// Testo in chiaro da cifrare: messaggio e (con ENCEXTMSGOPT) elenco dei file,
/// ciascuno terminato da '\0' come fa IP Messenger.
pub fn encrypted_body(text: &str, file_list: &str) -> String {
    let mut s = String::with_capacity(text.len() + file_list.len() + 2);
    s.push_str(text);
    s.push('\0');
    if !file_list.is_empty() {
        s.push_str(file_list);
        s.push('\0');
    }
    s
}

pub fn parse(data: &[u8]) -> Option<Packet> {
    let mut parts = data.splitn(6, |&b| b == b':');
    let _ver = parts.next()?;
    let no = std::str::from_utf8(parts.next()?).ok()?.trim().parse::<u64>().ok()?;
    let user = parts.next()?;
    let host = parts.next()?;
    let command = std::str::from_utf8(parts.next()?).ok()?.trim().parse::<u32>().ok()?;
    let mut extra = parts.next().unwrap_or(&[]).to_vec();
    if extra.last() == Some(&0) {
        extra.pop(); // terminatore finale
    }
    let utf8 = command & UTF8OPT != 0;
    let mut sections = extra.split(|&b| b == 0);
    let message = decode(sections.next().unwrap_or(&[]), utf8);
    let tail = sections.map(|s| decode(s, utf8)).collect();
    Some(Packet {
        packet_no: no,
        user: decode(user, utf8),
        host: decode(host, utf8),
        command,
        extra,
        message,
        tail,
    })
}

// ---- ingresso ----

pub fn entry_extra(nick: &str, group: &str, user: &str, host: &str) -> (String, Vec<String>) {
    (
        nick.to_string(),
        vec![group.to_string(), format!("\nUN:{user}\nHN:{host}\nNN:{nick}\nGN:{group}\nPM:2\n")],
    )
}

/// Il mittente dell'annuncio è un POPMSG (sa scambiare l'elenco utenti)?
pub fn is_popmsg(p: &Packet) -> bool {
    p.tail.iter().skip(1).any(|t| t.contains("\nPM:"))
}

pub fn parse_entry(p: &Packet) -> (String, String) {
    let mut nick = p.message.clone();
    let mut group = p.tail.first().cloned().unwrap_or_default();
    for sec in p.tail.iter().skip(1) {
        for line in sec.split('\n') {
            if let Some(v) = line.strip_prefix("NN:") {
                if !v.is_empty() {
                    nick = v.to_string();
                }
            } else if let Some(v) = line.strip_prefix("GN:") {
                group = v.to_string();
            }
        }
    }
    (nick, group)
}

// ---- elenco file ----

#[derive(Debug, Clone, PartialEq)]
pub struct FileInfo {
    pub id: u32,
    pub name: String,
    pub size: u64,
    pub mtime: u64,
    pub attr: u32,
    /// posizione nel testo delle immagini incollate
    pub pos: Option<u32>,
}

impl FileInfo {
    pub fn is_dir(&self) -> bool {
        mode(self.attr) == FILE_DIR
    }
    pub fn is_clip(&self) -> bool {
        mode(self.attr) == FILE_CLIPBOARD
    }
}

pub fn escape_name(n: &str) -> String {
    n.replace(':', "::")
}

pub fn build_file_list(files: &[FileInfo]) -> String {
    let mut s = String::new();
    for f in files {
        let pos = match (f.is_clip(), f.pos) {
            (true, Some(p)) => format!(":{FILE_CLIPBOARDPOS:x}={p:x}"),
            _ => String::new(),
        };
        s.push_str(&format!("{}:{}:{:x}:{:x}:{:x}{pos}:", f.id, escape_name(&f.name), f.size, f.mtime, f.attr));
        s.push(FILELIST_SEPARATOR);
    }
    s
}

/// Divide su ':' singoli; '::' è un ':' letterale.
pub fn split_fields(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == ':' {
            if it.peek() == Some(&':') {
                it.next();
                cur.push(':');
                continue;
            }
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

pub fn parse_file_list(s: &str) -> Vec<FileInfo> {
    let mut out = Vec::new();
    for entry in s.split(FILELIST_SEPARATOR) {
        let entry = entry.trim_start_matches(':').trim_matches('\0');
        if entry.is_empty() {
            continue;
        }
        let f = split_fields(entry);
        if f.len() < 5 {
            continue;
        }
        let (Ok(id), Ok(size), Ok(mtime), Ok(attr)) = (
            f[0].parse::<u32>(),
            u64::from_str_radix(&f[2], 16),
            u64::from_str_radix(&f[3], 16),
            u32::from_str_radix(&f[4], 16),
        ) else {
            continue;
        };
        let pos = f[5..]
            .iter()
            .filter_map(|x| x.split_once('='))
            .find(|(k, _)| u32::from_str_radix(k, 16) == Ok(FILE_CLIPBOARDPOS))
            .and_then(|(_, v)| u32::from_str_radix(v, 16).ok());
        out.push(FileInfo { id, name: f[1].clone(), size, mtime, attr, pos });
    }
    out
}

// ---- intestazioni GETDIRFILES ----

pub fn dir_header(name: &str, size: u64, attr: u32, mtime: u64) -> Vec<u8> {
    let body = format!(":{}:{:x}:{:x}:{:x}={:x}:", escape_name(name), size, attr, FILE_MTIME, mtime);
    let total = body.len() + 4;
    format!("{total:04x}{body}").into_bytes()
}

/// (nome, dimensione, attributo, mtime)
pub fn parse_dir_header(h: &[u8]) -> Option<(String, u64, u32, u64)> {
    let text = decode(h, true);
    let f = split_fields(&text);
    if f.len() < 4 {
        return None;
    }
    let size = u64::from_str_radix(&f[2], 16).ok()?;
    let attr = u32::from_str_radix(&f[3], 16).ok()?;
    let mut mtime = 0;
    for ext in f.iter().skip(4) {
        if let Some((k, v)) = ext.split_once('=') {
            if u32::from_str_radix(k, 16).ok() == Some(FILE_MTIME) {
                mtime = u64::from_str_radix(v.split(',').next().unwrap_or("0"), 16).unwrap_or(0);
            }
        }
    }
    Some((f[1].clone(), size, attr, mtime))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let files = vec![
            FileInfo { id: 0, name: "a:b.txt".into(), size: 10, mtime: 5, attr: FILE_REGULAR, pos: None },
            FileInfo { id: 1, name: "dir".into(), size: 0, mtime: 5, attr: FILE_DIR, pos: None },
            FileInfo { id: 2, name: "clip.png".into(), size: 99, mtime: 5, attr: FILE_CLIPBOARD, pos: Some(12) },
        ];
        assert_eq!(parse_file_list(&build_file_list(&files)), files);
        let h = dir_header("x:y", 255, FILE_REGULAR, 99);
        assert_eq!(usize::from_str_radix(std::str::from_utf8(&h[..4]).unwrap(), 16).unwrap(), h.len());
        assert_eq!(parse_dir_header(&h).unwrap(), ("x:y".into(), 255, 1, 99));
        let p = parse(&build(7, "u", "h", SENDMSG, "ciao è", &["x".into()])).unwrap();
        assert_eq!(p.message, "ciao è");
        let raw = build(7, "u", "h", SENDMSG, "ciao", &[]);
        assert_eq!(raw.last(), Some(&0));
        assert!(parse(&raw).unwrap().tail.is_empty());
        assert_eq!(encrypted_body("ciao", ""), "ciao\0");
        assert_eq!(encrypted_body("ciao", "f"), "ciao\0f\0");
        assert_eq!(p.tail, vec!["x".to_string()]);
        let (legacy, _, _) = encoding_rs::SHIFT_JIS.encode("1:5:taro:pc:32:こんにちは");
        assert_eq!(parse(&legacy).unwrap().message, "こんにちは");
    }
}
