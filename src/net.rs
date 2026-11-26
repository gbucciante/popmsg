//! Motore di rete: UDP per presenza e messaggi, TCP per i file.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rsa::RsaPrivateKey;

use crate::crypto::{self, PeerKey};
use crate::protocol::{self as p, FileInfo, Packet};

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const RETRY_INTERVAL: Duration = Duration::from_millis(1500);
const RETRY_COUNT: u32 = 4;
const KEY_WAIT: Duration = Duration::from_secs(4);

pub type Addr = SocketAddrV4;

#[derive(Debug, Clone)]
pub struct User {
    pub addr: Addr,
    pub login: String,
    pub host: String,
    pub nick: String,
    pub group: String,
    pub command: u32,
}

impl User {
    /// Chiave stabile per la rubrica locale (non cambia se cambia l'IP).
    pub fn book_key(&self) -> String {
        if self.addr.port() == p::DEFAULT_PORT || self.addr.port() == 0 {
            format!("{}@{}", self.login, self.host)
        } else {
            format!("{}@{}:{}", self.login, self.host, self.addr.port())
        }
    }
    pub fn absent(&self) -> bool {
        self.command & p::ABSENCEOPT != 0
    }
    pub fn name(&self) -> &str {
        if self.nick.is_empty() { &self.login } else { &self.nick }
    }
    pub fn encrypts(&self) -> bool {
        self.command & p::ENCRYPTOPT != 0
    }
}

#[derive(Debug, Clone)]
pub struct Message {
    pub packet_no: u64,
    pub sender: User,
    pub text: String,
    pub command: u32,
    pub received_at: SystemTime,
    pub files: Vec<FileInfo>,
    pub encrypted: bool,
    pub decrypt_error: String,
}

impl Message {
    pub fn sealed(&self) -> bool {
        self.command & p::SECRETOPT != 0
    }
    pub fn multi(&self) -> bool {
        self.command & (p::BROADCASTOPT | p::MULTICASTOPT) != 0
    }
    pub fn auto_reply(&self) -> bool {
        self.command & p::AUTORETOPT != 0
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    UsersChanged,
    Message(Box<Message>),
    Delivered(u64),
    Opened(u64, String),
    Failed(u64, Addr),
    AbsenceInfo(String, String),
    VersionInfo(String, String),
    /// un altro POPMSG chiede il nostro elenco utenti
    ListRequest(Addr),
    /// elenco utenti ricevuto da un altro POPMSG
    ListReceived(Addr, String),
    Error(String),
}

struct Pending {
    data: Vec<u8>,
    addr: Addr,
    tries: u32,
    last: Instant,
}

struct Queued {
    no: u64,
    cmd: u32,
    text: String,
    file_list: String,
    since: Instant,
}

struct Shared {
    path: PathBuf,
    info: FileInfo,
}

struct State {
    login: String,
    host: String,
    nick: String,
    group: String,
    port: u16,
    extra: Vec<String>,
    absent: bool,
    absence_text: String,
    users: BTreeMap<Addr, User>,
    pending: HashMap<u64, Pending>,
    recent: HashMap<(Addr, u64), Instant>,
    shared: HashMap<u64, HashMap<u32, Arc<Shared>>>,
    key: Option<Arc<RsaPrivateKey>>,
    encryption: bool,
    peer_keys: HashMap<Addr, PeerKey>,
    key_wait: HashMap<Addr, Vec<Queued>>,
    local_ips: HashSet<Ipv4Addr>,
    share_list: bool,
    /// POPMSG a cui abbiamo già chiesto l'elenco in questa sessione
    listed: HashSet<Addr>,
}

type Notify = Arc<dyn Fn(Event) + Send + Sync>;

#[derive(Clone)]
pub struct Engine {
    st: Arc<Mutex<State>>,
    udp: Arc<UdpSocket>,
    notify: Notify,
    running: Arc<AtomicBool>,
}

pub struct Options {
    pub nick: String,
    pub group: String,
    pub port: u16,
    pub extra: Vec<String>,
    pub key: Option<RsaPrivateKey>,
    pub encryption: bool,
}

pub fn login_name() -> String {
    let n = whoami::username();
    if n.is_empty() { "user".into() } else { n }
}

pub fn host_name() -> String {
    gethostname::gethostname().to_string_lossy().into_owned()
}

fn local_ips() -> HashSet<Ipv4Addr> {
    let mut s: HashSet<Ipv4Addr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| match i.addr {
            if_addrs::IfAddr::V4(v) => Some(v.ip),
            _ => None,
        })
        .collect();
    s.insert(Ipv4Addr::LOCALHOST);
    s
}

fn broadcast_targets(port: u16, extra: &[String]) -> Vec<Addr> {
    let mut v: Vec<Addr> = vec![Addr::new(Ipv4Addr::BROADCAST, port)];
    for i in if_addrs::get_if_addrs().unwrap_or_default() {
        if i.is_loopback() {
            continue;
        }
        if let if_addrs::IfAddr::V4(a) = i.addr {
            if let Some(b) = a.broadcast {
                let t = Addr::new(b, port);
                if !v.contains(&t) {
                    v.push(t);
                }
            }
        }
    }
    for e in extra {
        let e = e.trim();
        let (h, pt) = match e.rsplit_once(':') {
            Some((h, pt)) => (h, pt.parse().unwrap_or(port)),
            None => (e, port),
        };
        if let Ok(ip) = h.parse::<Ipv4Addr>() {
            v.push(Addr::new(ip, pt));
        } else if let Ok(mut it) = std::net::ToSocketAddrs::to_socket_addrs(&(h, pt)) {
            if let Some(SocketAddr::V4(a)) = it.find(|a| a.is_ipv4()) {
                v.push(a);
            }
        }
    }
    v
}

impl Engine {
    pub fn start(opts: Options, notify: impl Fn(Event) + Send + Sync + 'static) -> Result<Engine, String> {
        let sock = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::DGRAM, Some(socket2::Protocol::UDP))
            .map_err(|e| e.to_string())?;
        sock.set_broadcast(true).ok();
        sock.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, opts.port)).into())
            .map_err(|e| format!("porta UDP {}: {e}", opts.port))?;
        let udp: UdpSocket = sock.into();
        udp.set_read_timeout(Some(Duration::from_millis(250))).ok();
        let tcp = TcpListener::bind((Ipv4Addr::UNSPECIFIED, opts.port))
            .map_err(|e| format!("porta TCP {}: {e}", opts.port))?;

        let login = login_name();
        let st = State {
            nick: if opts.nick.is_empty() { login.clone() } else { opts.nick },
            login,
            host: host_name(),
            group: opts.group,
            port: opts.port,
            extra: opts.extra,
            absent: false,
            absence_text: "Sono assente in questo momento.".into(),
            users: BTreeMap::new(),
            pending: HashMap::new(),
            recent: HashMap::new(),
            shared: HashMap::new(),
            encryption: opts.encryption && opts.key.is_some(),
            key: opts.key.map(Arc::new),
            peer_keys: HashMap::new(),
            key_wait: HashMap::new(),
            local_ips: local_ips(),
            share_list: true,
            listed: HashSet::new(),
        };
        let eng = Engine {
            st: Arc::new(Mutex::new(st)),
            udp: Arc::new(udp),
            notify: Arc::new(notify),
            running: Arc::new(AtomicBool::new(true)),
        };
        let e2 = eng.clone();
        thread::Builder::new().name("udp".into()).spawn(move || e2.udp_loop()).ok();
        let e3 = eng.clone();
        thread::Builder::new().name("tcp".into()).spawn(move || e3.tcp_loop(tcp)).ok();
        eng.broadcast_entry();
        Ok(eng)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn emit(&self, ev: Event) {
        (self.notify)(ev)
    }

    pub fn stop(&self) {
        if !self.running.swap(false, Ordering::SeqCst) {
            return;
        }
        let (cmd, nick, tail) = self.entry();
        self.broadcast(p::BR_EXIT | cmd, &nick, &tail);
        // sblocca accept()
        let port = self.lock().port;
        let _ = TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, port)), Duration::from_millis(200));
    }

    // ------------------------------------------------------------ stato
    pub fn users(&self) -> Vec<User> {
        self.lock().users.values().cloned().collect()
    }
    pub fn user(&self, a: &Addr) -> Option<User> {
        self.lock().users.get(a).cloned()
    }
    pub fn port(&self) -> u16 {
        self.lock().port
    }
    pub fn absent(&self) -> bool {
        self.lock().absent
    }
    pub fn fingerprint(&self) -> Option<String> {
        self.lock().key.as_ref().map(|k| crypto::fingerprint(&k.to_public_key()))
    }
    pub fn will_encrypt(&self, u: &User) -> bool {
        let s = self.lock();
        s.encryption && u.encrypts()
    }

    // --------------------------------------------------------- presenza
    fn entry(&self) -> (u32, String, Vec<String>) {
        let s = self.lock();
        let mut opt = p::CAPUTF8OPT | p::FILEATTACHOPT;
        if s.key.is_some() {
            opt |= p::ENCRYPTOPT | p::ENCEXTMSGOPT;
        }
        if s.absent {
            opt |= p::ABSENCEOPT;
        }
        let nick = if s.absent { format!("{} [Assente]", s.nick) } else { s.nick.clone() };
        let (n, t) = p::entry_extra(&nick, &s.group, &s.login, &s.host);
        (opt, n, t)
    }

    pub fn broadcast_entry(&self) {
        let (cmd, nick, tail) = self.entry();
        self.broadcast(p::BR_ENTRY | cmd, &nick, &tail);
    }

    fn broadcast_absence(&self) {
        let (cmd, nick, tail) = self.entry();
        self.broadcast(p::BR_ABSENCE | cmd, &nick, &tail);
    }

    /// Annuncio diretto a indirizzi precisi (utenti noti fuori dal broadcast):
    /// chi è in linea risponde e compare nell'elenco.
    pub fn probe(&self, addrs: &[Addr]) {
        let (cmd, nick, tail) = self.entry();
        let data = self.packet(p::next_packet_no(), p::BR_ENTRY | cmd, &nick, &tail);
        for a in addrs {
            if !a.ip().is_unspecified() && a.port() != 0 {
                self.send_to(*a, &data);
            }
        }
    }

    pub fn refresh(&self) {
        self.lock().users.clear();
        self.emit(Event::UsersChanged);
        self.broadcast_entry();
    }

    pub fn set_identity(&self, nick: &str, group: &str, extra: Vec<String>) {
        {
            let mut s = self.lock();
            s.nick = if nick.is_empty() { s.login.clone() } else { nick.to_string() };
            s.group = group.to_string();
            s.extra = extra;
        }
        self.broadcast_absence();
    }

    pub fn set_share_list(&self, on: bool) {
        self.lock().share_list = on;
    }

    /// Risponde a una richiesta di elenco utenti (testo già pronto, una riga per utente).
    pub fn send_list(&self, a: Addr, text: &str) {
        if !self.lock().share_list {
            return;
        }
        // resta dentro un pacchetto UDP
        let mut t = text;
        while t.len() > 60_000 {
            t = &t[..t[..60_000].rfind('\n').unwrap_or(0)];
        }
        self.reply(a, p::POPMSG_ANSLIST, t);
    }

    pub fn set_encryption(&self, on: bool) {
        let mut s = self.lock();
        s.encryption = on && s.key.is_some();
    }

    pub fn set_absence(&self, on: bool, text: Option<&str>) {
        {
            let mut s = self.lock();
            s.absent = on;
            if let Some(t) = text {
                s.absence_text = t.to_string();
            }
        }
        self.broadcast_absence();
    }

    // ------------------------------------------------------------ invio
    fn packet(&self, no: u64, cmd: u32, extra: &str, tail: &[String]) -> Vec<u8> {
        let s = self.lock();
        p::build(no, &s.login, &s.host, cmd, extra, tail)
    }

    fn send_to(&self, a: Addr, data: &[u8]) {
        let _ = self.udp.send_to(data, a);
    }

    fn reply(&self, a: Addr, cmd: u32, extra: &str) {
        let d = self.packet(p::next_packet_no(), cmd, extra, &[]);
        self.send_to(a, &d);
    }

    fn broadcast(&self, cmd: u32, extra: &str, tail: &[String]) {
        let data = self.packet(p::next_packet_no(), cmd, extra, tail);
        let (port, ex) = {
            let s = self.lock();
            (s.port, s.extra.clone())
        };
        for t in broadcast_targets(port, &ex) {
            self.send_to(t, &data);
        }
    }

    /// Invia un messaggio a ciascun destinatario; restituisce i numeri di pacchetto.
    /// `sealed`: 0 = normale, 1 = sigillato, 2 = sigillato con lucchetto (password)
    pub fn send_message(&self, users: &[User], text: &str, sealed: u8, attachments: &[PathBuf]) -> Vec<u64> {
        let mut files = Vec::new();
        for (i, path) in attachments.iter().enumerate() {
            let Ok(md) = fs::metadata(path) else { continue };
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
            let mtime = md.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
            let info = FileInfo {
                id: i as u32,
                name,
                size: if md.is_dir() { 0 } else { md.len() },
                mtime,
                attr: if md.is_dir() { p::FILE_DIR } else { p::FILE_REGULAR },
            };
            files.push(Arc::new(Shared { path: path.clone(), info }));
        }
        let file_list = if files.is_empty() {
            String::new()
        } else {
            p::build_file_list(&files.iter().map(|f| f.info.clone()).collect::<Vec<_>>())
        };
        let mut out = Vec::new();
        for u in users {
            let mut cmd = p::SENDMSG | p::SENDCHECKOPT;
            if users.len() > 1 {
                cmd |= p::MULTICASTOPT;
            }
            if sealed > 0 {
                cmd |= p::SECRETEXOPT;
            }
            if sealed == 2 {
                cmd |= p::PASSWORDOPT;
            }
            if !files.is_empty() {
                cmd |= p::FILEATTACHOPT;
            }
            let no = p::next_packet_no();
            let need_key = {
                let mut s = self.lock();
                if !files.is_empty() {
                    s.shared.insert(no, files.iter().map(|f| (f.info.id, f.clone())).collect());
                }
                let need = s.encryption && u.encrypts() && !s.peer_keys.contains_key(&u.addr);
                if need {
                    s.key_wait.entry(u.addr).or_default().push(Queued {
                        no,
                        cmd,
                        text: text.to_string(),
                        file_list: file_list.clone(),
                        since: Instant::now(),
                    });
                }
                need
            };
            if need_key {
                self.reply(u.addr, p::GETPUBKEY, &format!("{:X}", crypto::OUR_CAPS));
            } else {
                self.dispatch(u.addr, no, cmd, text, &file_list, true);
            }
            out.push(no);
        }
        out
    }

    fn dispatch(&self, a: Addr, no: u64, mut cmd: u32, text: &str, file_list: &str, allow_enc: bool) {
        let (peer, ext) = {
            let s = self.lock();
            let peer = if s.encryption && allow_enc { s.peer_keys.get(&a).cloned() } else { None };
            let ext = s.users.get(&a).map(|u| u.command & p::ENCEXTMSGOPT != 0).unwrap_or(false);
            (peer, ext)
        };
        let mut extra = text.to_string();
        let mut tail: Vec<String> = if file_list.is_empty() { vec![] } else { vec![file_list.to_string()] };
        if let Some(peer) = peer.filter(|pk| crypto::choose_caps(pk) != 0) {
            // IP Messenger tratta il testo decifrato come stringa C: serve il '\0'
            // finale, altrimenti mostra il padding e i byte che seguono in memoria.
            let mut c2 = cmd;
            let with_files = !file_list.is_empty() && ext;
            if with_files {
                c2 |= p::ENCEXTMSGOPT;
            }
            let plain = p::encrypted_body(text, if with_files { file_list } else { "" });
            match crypto::encrypt(&peer, plain.as_bytes(), no) {
                Ok(enc) => {
                    extra = enc;
                    cmd = c2 | p::ENCRYPTOPT;
                    if c2 & p::ENCEXTMSGOPT != 0 {
                        tail.clear();
                    }
                }
                Err(e) => self.emit(Event::Error(format!("Cifratura non riuscita: {e}"))),
            }
        }
        let data = self.packet(no, cmd, &extra, &tail);
        self.send_to(a, &data);
        self.lock().pending.insert(no, Pending { data, addr: a, tries: 0, last: Instant::now() });
    }

    fn flush_key_wait(&self, a: Addr, plain: bool) {
        let items = self.lock().key_wait.remove(&a).unwrap_or_default();
        for q in items {
            self.dispatch(a, q.no, q.cmd, &q.text, &q.file_list, !plain);
        }
    }

    pub fn mark_read(&self, m: &Message) {
        if m.command & (p::SECRETOPT | p::READCHECKOPT) != 0 {
            self.reply(m.sender.addr, p::READMSG | (m.command & p::READCHECKOPT), &m.packet_no.to_string());
        }
    }
    pub fn request_absence(&self, u: &User) {
        self.reply(u.addr, p::GETABSENCEINFO, "");
    }
    pub fn request_version(&self, u: &User) {
        self.reply(u.addr, p::GETINFO, "");
    }
    pub fn release_files(&self, m: &Message) {
        self.reply(m.sender.addr, p::RELEASEFILES, &m.packet_no.to_string());
    }

    fn tick(&self) {
        let now = Instant::now();
        let expired: Vec<Addr> = self
            .lock()
            .key_wait
            .iter()
            .filter(|(_, v)| v.first().map(|q| now - q.since > KEY_WAIT).unwrap_or(false))
            .map(|(a, _)| *a)
            .collect();
        for a in expired {
            self.flush_key_wait(a, true);
        }
        let mut resend = Vec::new();
        let mut failed = Vec::new();
        {
            let mut s = self.lock();
            s.pending.retain(|no, pd| {
                if now - pd.last < RETRY_INTERVAL {
                    return true;
                }
                pd.tries += 1;
                pd.last = now;
                if pd.tries > RETRY_COUNT {
                    failed.push((*no, pd.addr));
                    return false;
                }
                // imposta RETRYOPT
                let mut parts: Vec<Vec<u8>> = pd.data.splitn(6, |&b| b == b':').map(|x| x.to_vec()).collect();
                if parts.len() == 6 {
                    if let Some(c) = std::str::from_utf8(&parts[4]).ok().and_then(|c| c.parse::<u32>().ok()) {
                        parts[4] = (c | p::RETRYOPT).to_string().into_bytes();
                    }
                }
                resend.push((pd.addr, parts.join(&b':')));
                true
            });
            s.recent.retain(|_, t| now - *t < Duration::from_secs(600));
        }
        for (a, d) in resend {
            self.send_to(a, &d);
        }
        for (no, a) in failed {
            self.emit(Event::Failed(no, a));
        }
    }

    // --------------------------------------------------------- ricezione
    fn udp_loop(&self) {
        let mut buf = vec![0u8; 65536];
        while self.running.load(Ordering::SeqCst) {
            match self.udp.recv_from(&mut buf) {
                Ok((n, SocketAddr::V4(a))) => {
                    if let Some(pk) = p::parse(&buf[..n]) {
                        self.handle(pk, a);
                    }
                }
                Ok(_) => {}
                Err(_) => {}
            }
            self.tick();
        }
    }

    fn update_user(&self, pk: &Packet, a: Addr) {
        let (nick, group) = p::parse_entry(pk);
        {
            let mut s = self.lock();
            if matches!(pk.mode(), p::BR_ENTRY | p::ANSENTRY) {
                s.peer_keys.remove(&a);
            }
            s.users.insert(
                a,
                User { addr: a, login: pk.user.clone(), host: pk.host.clone(), nick, group, command: pk.command },
            );
        }
        self.emit(Event::UsersChanged);
        // altro POPMSG appena visto: scambiamo gli elenchi degli utenti conosciuti
        let ask = {
            let mut s = self.lock();
            let me = s.local_ips.contains(a.ip()) && a.port() == s.port;
            s.share_list && !me && p::is_popmsg(pk) && s.listed.insert(a)
        };
        if ask {
            self.reply(a, p::POPMSG_GETLIST, "");
        }
    }

    fn user_or_stub(&self, pk: &Packet, a: Addr) -> User {
        self.user(&a).unwrap_or_else(|| User {
            addr: a,
            login: pk.user.clone(),
            host: pk.host.clone(),
            nick: pk.user.clone(),
            group: String::new(),
            command: 0,
        })
    }

    fn handle(&self, pk: Packet, a: Addr) {
        let num = || pk.message.trim().parse::<u64>().unwrap_or(0);
        match pk.mode() {
            p::BR_ENTRY => {
                self.update_user(&pk, a);
                let (cmd, nick, tail) = self.entry();
                let d = self.packet(p::next_packet_no(), p::ANSENTRY | cmd, &nick, &tail);
                self.send_to(a, &d);
            }
            p::ANSENTRY | p::BR_ABSENCE => self.update_user(&pk, a),
            p::POPMSG_GETLIST => {
                if self.lock().share_list {
                    self.emit(Event::ListRequest(a));
                }
            }
            p::POPMSG_ANSLIST => {
                if self.lock().share_list {
                    self.emit(Event::ListReceived(a, pk.message.clone()));
                }
            }
            p::BR_EXIT => {
                if self.lock().users.remove(&a).is_some() {
                    self.emit(Event::UsersChanged);
                }
            }
            p::SENDMSG => self.on_message(pk, a),
            p::RECVMSG => {
                let no = num();
                if self.lock().pending.remove(&no).is_some() {
                    self.emit(Event::Delivered(no));
                }
            }
            p::READMSG => {
                let no = num();
                if pk.has(p::READCHECKOPT) {
                    self.reply(a, p::ANSREADMSG, &no.to_string());
                }
                let name = self.user_or_stub(&pk, a).name().to_string();
                self.emit(Event::Opened(no, name));
            }
            p::GETINFO => self.reply(a, p::SENDINFO, &format!("POPMSG {APP_VERSION}")),
            p::SENDINFO => {
                let name = self.user_or_stub(&pk, a).name().to_string();
                self.emit(Event::VersionInfo(name, pk.message.clone()));
            }
            p::GETABSENCEINFO => {
                let t = {
                    let s = self.lock();
                    if s.absent { s.absence_text.clone() } else { "Non sono in modalità assenza.".into() }
                };
                self.reply(a, p::SENDABSENCEINFO, &t);
            }
            p::SENDABSENCEINFO => {
                let name = self.user_or_stub(&pk, a).name().to_string();
                self.emit(Event::AbsenceInfo(name, pk.message.clone()));
            }
            p::RELEASEFILES => {
                self.lock().shared.remove(&num());
            }
            p::GETPUBKEY => {
                let ans = self.lock().key.as_ref().map(|k| crypto::pubkey_answer(k));
                if let Some(ans) = ans {
                    self.reply(a, p::ANSPUBKEY, &ans);
                }
            }
            p::ANSPUBKEY => {
                match crypto::parse_pubkey_answer(&pk.message) {
                    Some(k) => {
                        self.lock().peer_keys.insert(a, k);
                    }
                    None => self.emit(Event::Error(format!("Chiave pubblica non valida da {a}"))),
                }
                self.flush_key_wait(a, false);
            }
            _ => {}
        }
    }

    fn on_message(&self, pk: Packet, a: Addr) {
        if pk.has(p::SENDCHECKOPT) && !pk.has(p::AUTORETOPT | p::BROADCASTOPT) {
            self.reply(a, p::RECVMSG, &pk.packet_no.to_string());
        }
        let (known, absent, absence_text, key) = {
            let mut s = self.lock();
            let k = (a, pk.packet_no);
            if s.recent.contains_key(&k) {
                return; // ritrasmissione già ricevuta
            }
            s.recent.insert(k, Instant::now());
            (s.users.contains_key(&a), s.absent, s.absence_text.clone(), s.key.clone())
        };
        if !known && !pk.has(p::NOADDLISTOPT) {
            let (cmd, nick, tail) = self.entry();
            let d = self.packet(p::next_packet_no(), p::BR_ENTRY | cmd, &nick, &tail);
            self.send_to(a, &d);
        }
        let sender = self.user_or_stub(&pk, a);
        let mut text = pk.message.clone();
        let mut file_text = pk.tail.first().cloned().unwrap_or_default();
        let mut encrypted = false;
        let mut err = String::new();
        if pk.has(p::ENCRYPTOPT) {
            match key {
                None => err = "cifratura disattivata".into(),
                Some(k) => match crypto::decrypt(&k, &pk.message, pk.packet_no) {
                    Ok(plain) => {
                        let dec = p::decode(&plain, pk.has(p::UTF8OPT));
                        let mut it = dec.split('\0');
                        text = it.next().unwrap_or("").to_string();
                        if pk.has(p::ENCEXTMSGOPT) {
                            if let Some(f) = it.next() {
                                file_text = f.to_string();
                            }
                        }
                        encrypted = true;
                    }
                    Err(e) => err = e,
                },
            }
            if !err.is_empty() {
                text = format!("[Messaggio cifrato che non è stato possibile decifrare: {err}]");
            }
        }
        // caratteri di controllo (tranne a capo e tabulazione) non sono testo
        text.retain(|c| c == '\n' || c == '\t' || !c.is_control());
        let files = if pk.has(p::FILEATTACHOPT) { p::parse_file_list(&file_text) } else { vec![] };
        if absent && !pk.has(p::AUTORETOPT | p::BROADCASTOPT) {
            self.reply(a, p::SENDMSG | p::AUTORETOPT, &absence_text);
        }
        self.emit(Event::Message(Box::new(Message {
            packet_no: pk.packet_no,
            sender,
            text,
            command: pk.command,
            received_at: SystemTime::now(),
            files,
            encrypted,
            decrypt_error: err,
        })));
    }

    // ---------------------------------------------------- server dei file
    fn tcp_loop(&self, l: TcpListener) {
        for c in l.incoming() {
            if !self.running.load(Ordering::SeqCst) {
                break;
            }
            if let Ok(c) = c {
                let e = self.clone();
                thread::spawn(move || {
                    let _ = e.serve(c);
                });
            }
        }
    }

    fn serve(&self, mut c: TcpStream) -> std::io::Result<()> {
        c.set_read_timeout(Some(Duration::from_secs(10)))?;
        let mut buf = vec![0u8; 8192];
        let mut n = c.read(&mut buf)?;
        c.set_read_timeout(Some(Duration::from_millis(800)))?;
        while n > 0 && n < buf.len() && buf[..n].iter().filter(|&&b| b == b':').count() < 6 {
            match c.read(&mut buf[n..]) {
                Ok(0) | Err(_) => break,
                Ok(m) => n += m,
            }
        }
        let data: Vec<u8> = buf[..n].iter().copied().take_while(|&b| b != 0).collect();
        let Some(pk) = p::parse(&data) else { return Ok(()) };
        let f: Vec<&str> = pk.message.split(':').collect();
        if f.len() < 2 {
            return Ok(());
        }
        let no = u64::from_str_radix(f[0], 16).unwrap_or(0);
        let fid = u32::from_str_radix(f[1], 16).unwrap_or(u32::MAX);
        let off = f.get(2).and_then(|x| u64::from_str_radix(x, 16).ok()).unwrap_or(0);
        let Some(sh) = self.lock().shared.get(&no).and_then(|m| m.get(&fid)).cloned() else { return Ok(()) };
        c.set_read_timeout(None)?;
        if pk.mode() == p::GETFILEDATA && !sh.info.is_dir() {
            let mut fh = File::open(&sh.path)?;
            if off > 0 {
                use std::io::Seek;
                fh.seek(std::io::SeekFrom::Start(off))?;
            }
            std::io::copy(&mut fh, &mut c)?;
        } else if pk.mode() == p::GETDIRFILES && sh.info.is_dir() {
            send_dir(&mut c, &sh.path)?;
        }
        c.flush()?;
        let _ = c.shutdown(std::net::Shutdown::Write);
        Ok(())
    }

    // ------------------------------------------------- ricezione dei file
    /// Scarica un file o una cartella (bloccante). `progress(ricevuti, totale, nome)`.
    pub fn download(
        &self,
        m: &Message,
        info: &FileInfo,
        dest: &Path,
        progress: &mut dyn FnMut(u64, u64, &str),
        cancel: &AtomicBool,
    ) -> Result<PathBuf, String> {
        let dir = info.is_dir();
        let cmd = if dir { p::GETDIRFILES } else { p::GETFILEDATA };
        let req = if dir {
            format!("{:x}:{:x}", m.packet_no, info.id)
        } else {
            format!("{:x}:{:x}:0", m.packet_no, info.id)
        };
        let data = self.packet(p::next_packet_no(), cmd, &req, &[]);
        let mut s = TcpStream::connect_timeout(&SocketAddr::V4(m.sender.addr), Duration::from_secs(15))
            .map_err(|e| format!("connessione non riuscita: {e}"))?;
        s.write_all(&data).map_err(|e| e.to_string())?;
        s.set_read_timeout(Some(Duration::from_secs(60))).ok();
        if dir {
            return recv_dir(&mut s, dest, progress, cancel);
        }
        let target = unique_path(&dest.join(safe_name(&info.name)));
        let mut fh = File::create(&target).map_err(|e| e.to_string())?;
        let mut got = 0u64;
        let mut buf = vec![0u8; 256 * 1024];
        while got < info.size {
            if cancel.load(Ordering::Relaxed) {
                drop(fh);
                let _ = fs::remove_file(&target);
                return Err("annullato".into());
            }
            let want = ((info.size - got) as usize).min(buf.len());
            let n = s.read(&mut buf[..want]).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            fh.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            got += n as u64;
            progress(got, info.size, &info.name);
        }
        if got < info.size {
            return Err("il trasferimento si è interrotto".into());
        }
        drop(fh);
        set_mtime(&target, info.mtime);
        Ok(target)
    }
}

fn send_dir(c: &mut TcpStream, path: &Path) -> std::io::Result<()> {
    let md = fs::metadata(path)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "cartella".into());
    c.write_all(&p::dir_header(&name, 0, p::FILE_DIR, mtime_of(&md)))?;
    let mut entries: Vec<_> = fs::read_dir(path)?.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            send_dir(c, &e.path())?;
        } else if ft.is_file() {
            let md = e.metadata()?;
            c.write_all(&p::dir_header(&e.file_name().to_string_lossy(), md.len(), p::FILE_REGULAR, mtime_of(&md)))?;
            let mut f = File::open(e.path())?;
            let sent = std::io::copy(&mut Read::by_ref(&mut f).take(md.len()), c)?;
            // se il file si è accorciato nel frattempo, completa con zeri
            if sent < md.len() {
                c.write_all(&vec![0u8; (md.len() - sent) as usize])?;
            }
        }
    }
    c.write_all(&p::dir_header(".", 0, p::FILE_RETPARENT, mtime_of(&md)))
}

fn recv_dir(
    s: &mut TcpStream,
    dest: &Path,
    progress: &mut dyn FnMut(u64, u64, &str),
    cancel: &AtomicBool,
) -> Result<PathBuf, String> {
    let mut r = std::io::BufReader::with_capacity(256 * 1024, s);
    let mut stack: Vec<PathBuf> = Vec::new();
    let mut root = PathBuf::new();
    let mut total = 0u64;
    let io = |e: std::io::Error| e.to_string();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("annullato".into());
        }
        // header-size fino al primo ':'
        let mut head = Vec::new();
        let mut b = [0u8; 1];
        loop {
            match r.read(&mut b).map_err(io)? {
                0 => return if root.as_os_str().is_empty() { Err("nessun dato ricevuto".into()) } else { Ok(root) },
                _ => {
                    head.push(b[0]);
                    if b[0] == b':' {
                        break;
                    }
                    if head.len() > 16 {
                        return Err("intestazione non valida".into());
                    }
                }
            }
        }
        let hsize = usize::from_str_radix(std::str::from_utf8(&head[..head.len() - 1]).unwrap_or(""), 16)
            .map_err(|_| "intestazione non valida")?;
        let mut rest = vec![0u8; hsize.saturating_sub(head.len())];
        r.read_exact(&mut rest).map_err(io)?;
        head.extend_from_slice(&rest);
        let (name, size, attr, mtime) = p::parse_dir_header(&head).ok_or("intestazione non valida")?;
        let cur = stack.last().cloned().unwrap_or_else(|| dest.to_path_buf());
        match p::mode(attr) {
            p::FILE_DIR => {
                let mut path = cur.join(safe_name(&name));
                if stack.is_empty() {
                    path = unique_path(&path);
                    root = path.clone();
                }
                fs::create_dir_all(&path).map_err(io)?;
                stack.push(path);
            }
            p::FILE_RETPARENT => {
                stack.pop();
                if stack.is_empty() {
                    return Ok(root);
                }
            }
            _ => {
                let path = cur.join(safe_name(&name));
                let mut f = File::create(&path).map_err(io)?;
                let mut left = size;
                let mut buf = vec![0u8; 64 * 1024];
                while left > 0 {
                    let want = (left as usize).min(buf.len());
                    let n = r.read(&mut buf[..want]).map_err(io)?;
                    if n == 0 {
                        return Err("il trasferimento si è interrotto".into());
                    }
                    f.write_all(&buf[..n]).map_err(io)?;
                    left -= n as u64;
                    total += n as u64;
                    progress(total, 0, &name);
                }
                drop(f);
                set_mtime(&path, mtime);
            }
        }
    }
}

fn mtime_of(md: &fs::Metadata) -> u64 {
    md.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

fn set_mtime(path: &Path, mtime: u64) {
    if mtime == 0 {
        return;
    }
    if let Ok(f) = File::options().write(true).open(path) {
        let _ = f.set_modified(UNIX_EPOCH + Duration::from_secs(mtime));
    }
}

pub fn safe_name(n: &str) -> String {
    let base = n.rsplit(['/', '\\']).next().unwrap_or("");
    let s: String = base.chars().map(|c| if "<>:\"|?*".contains(c) || (c as u32) < 32 { '_' } else { c }).collect();
    if s.is_empty() || s == "." || s == ".." { "_".into() } else { s }
}

pub fn unique_path(p: &Path) -> PathBuf {
    if !p.exists() {
        return p.to_path_buf();
    }
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let dir = p.parent().unwrap_or(Path::new("."));
    (1..).map(|i| dir.join(format!("{stem} ({i}){ext}"))).find(|c| !c.exists()).unwrap()
}
