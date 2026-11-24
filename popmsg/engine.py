"""Motore di rete di POPMSG: UDP per presenza e messaggi, TCP per i file."""
from __future__ import annotations

import getpass
import os
import socket
import threading
import time
from dataclasses import dataclass, field

from PySide6.QtCore import QObject, QTimer, Signal
from PySide6.QtNetwork import QAbstractSocket, QHostAddress, QNetworkInterface, QUdpSocket

from . import protocol as P
from .version import APP_NAME, APP_VERSION

USER_TIMEOUT = 0  # gli utenti restano finché non escono (come IPMsg)
RETRY_INTERVAL_MS = 1500
RETRY_COUNT = 4


@dataclass
class User:
    ip: str
    port: int
    login: str
    host: str
    nick: str
    group: str
    command: int
    last_seen: float = field(default_factory=time.time)

    @property
    def key(self) -> tuple[str, int]:
        return (self.ip, self.port)

    @property
    def absent(self) -> bool:
        return bool(self.command & P.ABSENCEOPT)

    @property
    def display_name(self) -> str:
        return self.nick or self.login


@dataclass
class Attachment:
    file_id: int
    path: str
    info: P.FileInfo


@dataclass
class Message:
    packet_no: int
    sender: User
    text: str
    command: int
    received_at: float
    files: list[P.FileInfo] = field(default_factory=list)

    @property
    def sealed(self) -> bool:
        return bool(self.command & P.SECRETOPT)

    @property
    def read_check(self) -> bool:
        return bool(self.command & P.READCHECKOPT)

    @property
    def broadcast(self) -> bool:
        return bool(self.command & (P.BROADCASTOPT | P.MULTICASTOPT))


@dataclass
class _Pending:
    data: bytes
    key: tuple[str, int]
    tries: int = 0


def local_ipv4_addresses() -> list[str]:
    result = []
    for iface in QNetworkInterface.allInterfaces():
        flags = iface.flags()
        if not (flags & QNetworkInterface.InterfaceFlag.IsUp):
            continue
        for entry in iface.addressEntries():
            ip = entry.ip()
            if ip.protocol() == QAbstractSocket.NetworkLayerProtocol.IPv4Protocol:
                result.append(ip.toString())
    return result


def broadcast_addresses() -> list[str]:
    result = {"255.255.255.255"}
    for iface in QNetworkInterface.allInterfaces():
        flags = iface.flags()
        if not (flags & QNetworkInterface.InterfaceFlag.IsUp):
            continue
        if flags & QNetworkInterface.InterfaceFlag.IsLoopBack:
            continue
        for entry in iface.addressEntries():
            bc = entry.broadcast()
            if not bc.isNull() and bc.protocol() == QAbstractSocket.NetworkLayerProtocol.IPv4Protocol:
                result.add(bc.toString())
    return sorted(result)


class Engine(QObject):
    usersChanged = Signal()
    messageReceived = Signal(object)            # Message
    messageDelivered = Signal(int, object)      # packet_no, User
    messageOpened = Signal(int, object)         # packet_no, User (messaggio sigillato aperto)
    messageFailed = Signal(int, object)         # packet_no, User
    absenceInfo = Signal(object, str)           # User, testo
    versionInfo = Signal(object, str)           # User, testo
    error = Signal(str)

    def __init__(self, nick: str = "", group: str = "", port: int = P.DEFAULT_PORT,
                 extra_broadcast: list[str] | None = None, parent=None):
        super().__init__(parent)
        self.login = getpass.getuser()
        self.hostname = socket.gethostname()
        self.nick = nick or self.login
        self.group = group
        self.port = port
        self.extra_broadcast = list(extra_broadcast or [])
        self.absent = False
        self.absence_text = "Sono assente in questo momento."
        self.users: dict[tuple[str, int], User] = {}
        self._pending: dict[int, _Pending] = {}
        self._recent: dict[tuple[str, int, int], float] = {}
        self._shared: dict[int, dict[int, Attachment]] = {}
        self._shared_lock = threading.Lock()
        self._local_ips: set[str] = set()
        self.udp = QUdpSocket(self)
        self.udp.readyRead.connect(self._on_ready_read)
        self._retry_timer = QTimer(self)
        self._retry_timer.timeout.connect(self._retry)
        self._retry_timer.start(RETRY_INTERVAL_MS)
        self._tcp: socket.socket | None = None
        self._running = False

    # ------------------------------------------------------------------ avvio
    def start(self) -> bool:
        ok = self.udp.bind(QHostAddress(QHostAddress.SpecialAddress.AnyIPv4), self.port,
                           QAbstractSocket.BindFlag.DefaultForPlatform)
        if not ok:
            self.error.emit(f"Impossibile usare la porta UDP {self.port}: {self.udp.errorString()}")
            return False
        self._running = True
        try:
            self._tcp = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            if os.name != "nt":
                self._tcp.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            self._tcp.bind(("0.0.0.0", self.port))
            self._tcp.listen(16)
        except OSError as exc:
            self.error.emit(f"Impossibile usare la porta TCP {self.port}: {exc}")
            self._tcp = None
            self._running = False
            self.udp.close()
            return False
        else:
            threading.Thread(target=self._tcp_loop, daemon=True).start()
        self._local_ips = set(local_ipv4_addresses()) | {"127.0.0.1"}
        self.broadcast_entry()
        return True

    def stop(self):
        if not self._running:
            return
        self._broadcast(P.BR_EXIT | self._entry_opts(), *self._entry_payload())
        self._running = False
        if self._tcp:
            try:
                self._tcp.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            try:
                self._tcp.close()
            except OSError:
                pass
        self.udp.close()

    # ---------------------------------------------------------------- presenza
    def _entry_opts(self) -> int:
        opt = P.CAPUTF8OPT | P.FILEATTACHOPT
        if self.absent:
            opt |= P.ABSENCEOPT
        return opt

    def _entry_payload(self) -> tuple[str, list[str]]:
        nick = self.nick + (" [Assente]" if self.absent else "")
        return P.build_entry_extra(nick, self.group, self.login, self.hostname)

    def broadcast_entry(self):
        self._broadcast(P.BR_ENTRY | self._entry_opts(), *self._entry_payload())

    def refresh(self):
        """Svuota la lista e chiede di nuovo chi c'è (pulsante Aggiorna)."""
        self.users.clear()
        self.usersChanged.emit()
        self.broadcast_entry()

    def set_identity(self, nick: str, group: str):
        self.nick, self.group = nick or self.login, group
        if self._running:
            self._broadcast(P.BR_ABSENCE | self._entry_opts(), *self._entry_payload())

    def set_absence(self, absent: bool, text: str | None = None):
        self.absent = absent
        if text is not None:
            self.absence_text = text
        if self._running:
            self._broadcast(P.BR_ABSENCE | self._entry_opts(), *self._entry_payload())

    # ------------------------------------------------------------------- invio
    def _send(self, key: tuple[str, int], data: bytes):
        self.udp.writeDatagram(data, QHostAddress(key[0]), key[1])

    def _broadcast(self, command: int, extra: str = "", tail: list[str] | None = None):
        data = P.build_packet(P.next_packet_no(), self.login, self.hostname, command, extra, tail=tail)
        targets = broadcast_addresses() + self.extra_broadcast
        for addr in targets:
            host, port = addr, self.port
            if ":" in addr:
                host, p = addr.rsplit(":", 1)
                port = int(p)
            self.udp.writeDatagram(data, QHostAddress(host), port)

    def _packet(self, command: int, extra: str = "", tail: list[str] | None = None,
                packet_no: int | None = None) -> tuple[int, bytes]:
        no = packet_no or P.next_packet_no()
        return no, P.build_packet(no, self.login, self.hostname, command, extra, tail=tail)

    def send_message(self, users: list[User], text: str, sealed: bool = False,
                     attachments: list[str] | None = None) -> list[int]:
        """Invia un messaggio a ciascun utente. Restituisce i numeri di pacchetto."""
        packet_nos = []
        multi = len(users) > 1
        files: list[Attachment] = []
        file_list = ""
        for i, path in enumerate(attachments or []):
            st = os.stat(path)
            is_dir = os.path.isdir(path)
            info = P.FileInfo(i, os.path.basename(os.path.normpath(path)),
                              0 if is_dir else st.st_size, int(st.st_mtime),
                              P.FILE_DIR if is_dir else P.FILE_REGULAR)
            files.append(Attachment(i, path, info))
        if files:
            file_list = P.build_file_list([a.info for a in files])
        for user in users:
            cmd = P.SENDMSG | P.SENDCHECKOPT
            if multi:
                cmd |= P.MULTICASTOPT
            if sealed:
                cmd |= P.SECRETEXOPT
            tail = None
            if files:
                cmd |= P.FILEATTACHOPT
                tail = [file_list]
            no, data = self._packet(cmd, text, tail)
            if files:
                with self._shared_lock:
                    self._shared[no] = {a.file_id: a for a in files}
            self._pending[no] = _Pending(data, user.key)
            self._send(user.key, data)
            packet_nos.append(no)
        return packet_nos

    def _retry(self):
        for no, p in list(self._pending.items()):
            p.tries += 1
            if p.tries > RETRY_COUNT:
                del self._pending[no]
                user = self.users.get(p.key)
                self.messageFailed.emit(no, user)
                continue
            # imposta RETRYOPT sul ritrasmesso
            parts = p.data.split(b":", 5)
            parts[4] = str(int(parts[4]) | P.RETRYOPT).encode()
            self._send(p.key, b":".join(parts))

    def resend(self, packet_no: int, user: User, data: bytes):
        self._pending[packet_no] = _Pending(data, user.key)
        self._send(user.key, data)

    def mark_read(self, msg: Message):
        """Avvisa il mittente che il messaggio sigillato è stato aperto."""
        if msg.sealed or msg.read_check:
            _, data = self._packet(P.READMSG | (msg.command & P.READCHECKOPT), str(msg.packet_no))
            self._send(msg.sender.key, data)

    def request_absence_info(self, user: User):
        _, data = self._packet(P.GETABSENCEINFO)
        self._send(user.key, data)

    def request_version(self, user: User):
        _, data = self._packet(P.GETINFO)
        self._send(user.key, data)

    def release_files(self, msg: Message):
        _, data = self._packet(P.RELEASEFILES, str(msg.packet_no))
        self._send(msg.sender.key, data)

    # --------------------------------------------------------------- ricezione
    def _on_ready_read(self):
        while self.udp.hasPendingDatagrams():
            dg = self.udp.receiveDatagram()
            addr = dg.senderAddress()
            ip = addr.toString()
            if ip.startswith("::ffff:"):
                ip = ip[7:]
            pkt = P.parse_packet(bytes(dg.data()))
            if pkt:
                try:
                    self._handle(pkt, (ip, dg.senderPort()))
                except Exception as exc:  # non bloccare il ciclo eventi
                    self.error.emit(f"Errore nel pacchetto da {ip}: {exc}")

    def _is_self(self, pkt: P.Packet, key: tuple[str, int]) -> bool:
        return (key[1] == self.port and key[0] in self._local_ips
                and pkt.user == self.login and pkt.host == self.hostname)

    def _update_user(self, pkt: P.Packet, key: tuple[str, int]) -> User:
        nick, group = P.parse_entry(pkt)
        user = self.users.get(key)
        if user is None:
            user = User(key[0], key[1], pkt.user, pkt.host, nick, group, pkt.command)
            self.users[key] = user
        else:
            user.login, user.host, user.nick, user.group = pkt.user, pkt.host, nick, group
            user.command = pkt.command
            user.last_seen = time.time()
        self.usersChanged.emit()
        return user

    def _user_for(self, pkt: P.Packet, key: tuple[str, int]) -> User:
        user = self.users.get(key)
        if user is None:
            user = User(key[0], key[1], pkt.user, pkt.host, pkt.user, "", 0)
        return user

    def _handle(self, pkt: P.Packet, key: tuple[str, int]):
        mode = pkt.mode
        if mode == P.BR_ENTRY:
            # come IPMsg, anche noi stessi compariamo nella lista
            self._update_user(pkt, key)
            _, data = self._packet(P.ANSENTRY | self._entry_opts(), *self._entry_payload())
            self._send(key, data)
        elif mode in (P.ANSENTRY, P.BR_ABSENCE):
            self._update_user(pkt, key)
        elif mode == P.BR_EXIT:
            if self.users.pop(key, None) is not None:
                self.usersChanged.emit()
        elif mode == P.SENDMSG:
            self._on_message(pkt, key)
        elif mode == P.RECVMSG:
            try:
                no = int(pkt.message.strip() or 0)
            except ValueError:
                return
            if no in self._pending:
                del self._pending[no]
                self.messageDelivered.emit(no, self._user_for(pkt, key))
        elif mode == P.READMSG:
            try:
                no = int(pkt.message.strip() or 0)
            except ValueError:
                return
            if pkt.has(P.READCHECKOPT):
                _, data = self._packet(P.ANSREADMSG, str(no))
                self._send(key, data)
            self.messageOpened.emit(no, self._user_for(pkt, key))
        elif mode == P.GETINFO:
            _, data = self._packet(P.SENDINFO, f"{APP_NAME} {APP_VERSION}")
            self._send(key, data)
        elif mode == P.SENDINFO:
            self.versionInfo.emit(self._user_for(pkt, key), pkt.message)
        elif mode == P.GETABSENCEINFO:
            text = self.absence_text if self.absent else "Non sono in modalità assenza."
            _, data = self._packet(P.SENDABSENCEINFO, text)
            self._send(key, data)
        elif mode == P.SENDABSENCEINFO:
            self.absenceInfo.emit(self._user_for(pkt, key), pkt.message)
        elif mode == P.RELEASEFILES:
            try:
                no = int(pkt.message.strip() or 0)
            except ValueError:
                return
            with self._shared_lock:
                self._shared.pop(no, None)

    def _on_message(self, pkt: P.Packet, key: tuple[str, int]):
        if pkt.has(P.SENDCHECKOPT) and not pkt.has(P.AUTORETOPT | P.BROADCASTOPT):
            _, data = self._packet(P.RECVMSG, str(pkt.packet_no))
            self._send(key, data)
        # scarta i duplicati (ritrasmissioni)
        dup_key = (key[0], key[1], pkt.packet_no)
        now = time.time()
        self._recent = {k: t for k, t in self._recent.items() if now - t < 600}
        if dup_key in self._recent:
            return
        self._recent[dup_key] = now
        user = self.users.get(key)
        if user is None and not pkt.has(P.NOADDLISTOPT):
            # mittente sconosciuto: chiediamo chi è
            _, data = self._packet(P.BR_ENTRY | self._entry_opts(), *self._entry_payload())
            self._send(key, data)
        sender = user or User(key[0], key[1], pkt.user, pkt.host, pkt.user, "", 0)
        files = []
        if pkt.has(P.FILEATTACHOPT) and pkt.tail:
            files = P.parse_file_list(pkt.tail[0])
        msg = Message(pkt.packet_no, sender, pkt.message, pkt.command, now, files)
        if self.absent and not pkt.has(P.AUTORETOPT | P.BROADCASTOPT):
            _, data = self._packet(P.SENDMSG | P.AUTORETOPT, self.absence_text)
            self._send(key, data)
        self.messageReceived.emit(msg)

    # ----------------------------------------------------- server TCP dei file
    def _tcp_loop(self):
        while self._running and self._tcp:
            try:
                conn, addr = self._tcp.accept()
            except OSError:
                break
            threading.Thread(target=self._serve_file, args=(conn, addr), daemon=True).start()

    def _serve_file(self, conn: socket.socket, addr):
        try:
            conn.settimeout(10)
            data = conn.recv(8192)
            # la richiesta arriva di solito in un solo segmento; altrimenti attendi il resto
            while data and data.count(b":") < 6 and len(data) < 8192:
                conn.settimeout(1)
                try:
                    more = conn.recv(8192)
                except socket.timeout:
                    break
                if not more:
                    break
                data += more
            pkt = P.parse_packet(data.rstrip(b"\0"))
            if not pkt:
                return
            fields = pkt.message.split(":")
            if len(fields) < 2:
                return
            no, fid = int(fields[0], 16), int(fields[1], 16)
            offset = int(fields[2], 16) if len(fields) > 2 and fields[2] else 0
            with self._shared_lock:
                att = self._shared.get(no, {}).get(fid)
            if att is None:
                return
            conn.settimeout(None)
            if pkt.mode == P.GETFILEDATA and not att.info.is_dir:
                with open(att.path, "rb") as fh:
                    fh.seek(offset)
                    while True:
                        buf = fh.read(256 * 1024)
                        if not buf:
                            break
                        conn.sendall(buf)
            elif pkt.mode == P.GETDIRFILES and att.info.is_dir:
                self._send_dir(conn, att.path)
        except (OSError, ValueError):
            pass
        finally:
            try:
                conn.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            conn.close()

    def _send_dir(self, conn: socket.socket, path: str):
        st = os.stat(path)
        name = os.path.basename(os.path.normpath(path))
        conn.sendall(P.build_dir_header(name, 0, P.FILE_DIR, int(st.st_mtime)))
        for entry in sorted(os.scandir(path), key=lambda e: e.name):
            if entry.is_symlink():
                continue
            if entry.is_dir():
                self._send_dir(conn, entry.path)
            elif entry.is_file():
                est = entry.stat()
                conn.sendall(P.build_dir_header(entry.name, est.st_size, P.FILE_REGULAR, int(est.st_mtime)))
                with open(entry.path, "rb") as fh:
                    while True:
                        buf = fh.read(256 * 1024)
                        if not buf:
                            break
                        conn.sendall(buf)
        conn.sendall(P.build_dir_header(".", 0, P.FILE_RETPARENT, int(st.st_mtime)))

    # ----------------------------------------------------- download dei file
    def download(self, msg: Message, info: P.FileInfo, dest_dir: str,
                 progress=None, cancel: threading.Event | None = None) -> str:
        """Scarica un file o una cartella (bloccante: da usare in un thread).

        progress(byte_ricevuti, totale_o_0, nome_corrente)
        Restituisce il percorso salvato.
        """
        cmd = P.GETDIRFILES if info.is_dir else P.GETFILEDATA
        req = f"{msg.packet_no:x}:{info.file_id:x}" + ("" if info.is_dir else ":0")
        _, data = self._packet(cmd, req)
        sock = socket.create_connection((msg.sender.ip, msg.sender.port), timeout=15)
        try:
            sock.sendall(data)
            sock.settimeout(60)
            if info.is_dir:
                return self._recv_dir(sock, dest_dir, progress, cancel)
            target = _unique_path(os.path.join(dest_dir, _safe_name(info.name)))
            got = 0
            with open(target, "wb") as fh:
                while got < info.size:
                    if cancel and cancel.is_set():
                        raise InterruptedError("Trasferimento annullato")
                    chunk = sock.recv(min(256 * 1024, info.size - got))
                    if not chunk:
                        break
                    fh.write(chunk)
                    got += len(chunk)
                    if progress:
                        progress(got, info.size, info.name)
            if got < info.size:
                raise ConnectionError("Il trasferimento si è interrotto")
            if info.mtime:
                try:
                    os.utime(target, (info.mtime, info.mtime))
                except OSError:
                    pass
            return target
        finally:
            sock.close()

    def _recv_dir(self, sock, dest_dir, progress, cancel) -> str:
        reader = _SockReader(sock)
        stack: list[str] = []
        root = ""
        total = 0
        while True:
            if cancel and cancel.is_set():
                raise InterruptedError("Trasferimento annullato")
            size_field = reader.read_until(b":")
            if size_field is None:
                break
            hsize = int(size_field[:-1], 16)
            rest = reader.read_exact(hsize - len(size_field))
            name, size, attr, mtime = P.parse_dir_header(size_field + rest)
            kind = P.get_mode(attr)
            cur = stack[-1] if stack else dest_dir
            if kind == P.FILE_DIR:
                path = os.path.join(cur, _safe_name(name))
                if not stack:
                    path = _unique_path(path)
                    root = path
                os.makedirs(path, exist_ok=True)
                stack.append(path)
            elif kind == P.FILE_RETPARENT:
                if stack:
                    stack.pop()
                if not stack:
                    break
            else:
                path = os.path.join(cur, _safe_name(name))
                with open(path, "wb") as fh:
                    remaining = size
                    while remaining:
                        chunk = reader.read_some(min(remaining, 256 * 1024))
                        if not chunk:
                            raise ConnectionError("Il trasferimento si è interrotto")
                        fh.write(chunk)
                        remaining -= len(chunk)
                        total += len(chunk)
                        if progress:
                            progress(total, 0, name)
                if mtime:
                    try:
                        os.utime(path, (mtime, mtime))
                    except OSError:
                        pass
        return root


class _SockReader:
    def __init__(self, sock):
        self.sock = sock
        self.buf = b""

    def _fill(self) -> bool:
        chunk = self.sock.recv(256 * 1024)
        if not chunk:
            return False
        self.buf += chunk
        return True

    def read_until(self, sep: bytes) -> bytes | None:
        while sep not in self.buf:
            if not self._fill():
                return None
        i = self.buf.index(sep) + len(sep)
        out, self.buf = self.buf[:i], self.buf[i:]
        return out

    def read_exact(self, n: int) -> bytes:
        while len(self.buf) < n:
            if not self._fill():
                raise ConnectionError("Il trasferimento si è interrotto")
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def read_some(self, n: int) -> bytes:
        if not self.buf and not self._fill():
            return b""
        out, self.buf = self.buf[:n], self.buf[n:]
        return out


def _safe_name(name: str) -> str:
    name = name.replace("\\", "/").split("/")[-1]
    for c in '<>:"|?*':
        name = name.replace(c, "_")
    if name in ("", ".", ".."):
        name = "_"
    return name


def _unique_path(path: str) -> str:
    if not os.path.exists(path):
        return path
    base, ext = os.path.splitext(path)
    i = 1
    while os.path.exists(f"{base} ({i}){ext}"):
        i += 1
    return f"{base} ({i}){ext}"
