"""Costanti e codifica/decodifica dei pacchetti del protocollo IP Messenger.

Formato di un pacchetto:
    Ver:PacketNo:SenderName:SenderHost:CommandNo:AdditionalSection
"""
from __future__ import annotations

import itertools
import time
from dataclasses import dataclass, field

PROTOCOL_VERSION = "1"
DEFAULT_PORT = 2425

# ---- Comandi (byte basso) ----
NOOPERATION = 0x00
BR_ENTRY = 0x01
BR_EXIT = 0x02
ANSENTRY = 0x03
BR_ABSENCE = 0x04
BR_ISGETLIST = 0x10
OKGETLIST = 0x11
GETLIST = 0x12
ANSLIST = 0x13
BR_ISGETLIST2 = 0x18
SENDMSG = 0x20
RECVMSG = 0x21
READMSG = 0x30
DELMSG = 0x31
ANSREADMSG = 0x32
GETINFO = 0x40
SENDINFO = 0x41
GETABSENCEINFO = 0x50
SENDABSENCEINFO = 0x51
GETFILEDATA = 0x60
RELEASEFILES = 0x61
GETDIRFILES = 0x62
GETPUBKEY = 0x72
ANSPUBKEY = 0x73

# ---- Opzioni valide per tutti i comandi ----
ABSENCEOPT = 0x00000100
SERVEROPT = 0x00000200
DIALUPOPT = 0x00010000
FILEATTACHOPT = 0x00200000
ENCRYPTOPT = 0x00400000
UTF8OPT = 0x00800000
CAPUTF8OPT = 0x01000000
ENCEXTMSGOPT = 0x04000000
CLIPBOARDOPT = 0x08000000

# ---- Opzioni per SENDMSG ----
SENDCHECKOPT = 0x00000100
SECRETOPT = 0x00000200
BROADCASTOPT = 0x00000400
MULTICASTOPT = 0x00000800
AUTORETOPT = 0x00002000
RETRYOPT = 0x00004000
PASSWORDOPT = 0x00008000
NOLOGOPT = 0x00020000
NOADDLISTOPT = 0x00080000
READCHECKOPT = 0x00100000
SECRETEXOPT = READCHECKOPT | SECRETOPT

# ---- Tipi di file (byte basso dell'attributo) ----
FILE_REGULAR = 0x00000001
FILE_DIR = 0x00000002
FILE_RETPARENT = 0x00000003
FILE_SYMLINK = 0x00000004

# ---- Attributi estesi ----
FILE_MTIME = 0x00000014
FILE_CREATETIME = 0x00000016

FILELIST_SEPARATOR = "\a"

COMMAND_MASK = 0x000000FF
OPTION_MASK = 0xFFFFFF00


def get_mode(command: int) -> int:
    return command & COMMAND_MASK


def get_opt(command: int) -> int:
    return command & OPTION_MASK


_counter = itertools.count(int(time.time()))


def next_packet_no() -> int:
    return next(_counter)


def decode_bytes(data: bytes, utf8: bool) -> str:
    """Decodifica testo: UTF-8 se dichiarato, altrimenti prova UTF-8 e poi le code page tipiche."""
    if utf8:
        return data.decode("utf-8", errors="replace")
    for enc in ("utf-8", "cp932", "cp1252"):
        try:
            return data.decode(enc)
        except UnicodeDecodeError:
            continue
    return data.decode("latin-1", errors="replace")


@dataclass
class Packet:
    version: str
    packet_no: int
    user: str
    host: str
    command: int
    extra: bytes = b""
    # campi calcolati
    message: str = ""
    tail: list[str] = field(default_factory=list)  # sezioni dopo il primo \0

    @property
    def mode(self) -> int:
        return get_mode(self.command)

    @property
    def opt(self) -> int:
        return get_opt(self.command)

    def has(self, flag: int) -> bool:
        return bool(self.command & flag)


def build_packet(packet_no: int, user: str, host: str, command: int,
                 extra: str | bytes = "", *, tail: list[str] | None = None) -> bytes:
    """Crea un pacchetto. Il testo viene sempre codificato in UTF-8 (si imposta UTF8OPT)."""
    command |= UTF8OPT
    head = f"{PROTOCOL_VERSION}:{packet_no}:{user}:{host}:{command}:".encode("utf-8")
    body = extra if isinstance(extra, bytes) else extra.encode("utf-8")
    if tail:
        for section in tail:
            body += b"\0" + section.encode("utf-8")
    return head + body


def parse_packet(data: bytes) -> Packet | None:
    # Le prime cinque sezioni sono separate da ':' e non contengono ':'
    parts = data.split(b":", 5)
    if len(parts) < 5:
        return None
    try:
        packet_no = int(parts[1])
        command = int(parts[4])
    except ValueError:
        return None
    utf8 = bool(command & UTF8OPT)
    extra = parts[5] if len(parts) > 5 else b""
    pkt = Packet(
        version=decode_bytes(parts[0], utf8),
        packet_no=packet_no,
        user=decode_bytes(parts[2], utf8),
        host=decode_bytes(parts[3], utf8),
        command=command,
        extra=extra,
    )
    sections = extra.split(b"\0")
    pkt.message = decode_bytes(sections[0], utf8)
    pkt.tail = [decode_bytes(s, utf8) for s in sections[1:]]
    return pkt


# ---- Sezione di ingresso (nickname, gruppo, estensioni UTF-8) ----

def build_entry_extra(nick: str, group: str, user: str, host: str) -> tuple[str, list[str]]:
    ext = f"\nUN:{user}\nHN:{host}\nNN:{nick}\nGN:{group}\n"
    return nick, [group, ext]


def parse_entry(pkt: Packet) -> tuple[str, str]:
    """Restituisce (nickname, gruppo) da un pacchetto di ingresso/risposta."""
    nick = pkt.message
    group = pkt.tail[0] if pkt.tail else ""
    for section in pkt.tail[1:]:
        for line in section.split("\n"):
            if line.startswith("NN:") and line[3:]:
                nick = line[3:]
            elif line.startswith("GN:"):
                group = line[3:]
    return nick, group


# ---- Elenco file allegati ----

@dataclass
class FileInfo:
    file_id: int
    name: str
    size: int
    mtime: int
    attr: int

    @property
    def is_dir(self) -> bool:
        return get_mode(self.attr) == FILE_DIR


def escape_name(name: str) -> str:
    return name.replace(":", "::")


def build_file_list(files: list[FileInfo]) -> str:
    out = []
    for f in files:
        out.append(f"{f.file_id}:{escape_name(f.name)}:{f.size:x}:{f.mtime:x}:{f.attr:x}:")
    return FILELIST_SEPARATOR.join(out) + FILELIST_SEPARATOR


def _split_fields(entry: str) -> list[str]:
    """Divide su ':' singoli, trattando '::' come ':' letterale."""
    fields, cur, i = [], [], 0
    while i < len(entry):
        c = entry[i]
        if c == ":":
            if i + 1 < len(entry) and entry[i + 1] == ":":
                cur.append(":")
                i += 2
                continue
            fields.append("".join(cur))
            cur = []
        else:
            cur.append(c)
        i += 1
    fields.append("".join(cur))
    return fields


def parse_file_list(text: str) -> list[FileInfo]:
    result = []
    for entry in text.split(FILELIST_SEPARATOR):
        entry = entry.lstrip(":").strip("\0")
        if not entry:
            continue
        f = _split_fields(entry)
        if len(f) < 5:
            continue
        try:
            result.append(FileInfo(int(f[0]), f[1], int(f[2], 16), int(f[3], 16), int(f[4], 16)))
        except ValueError:
            continue
    return result


# ---- Intestazioni per GETDIRFILES ----

def build_dir_header(name: str, size: int, attr: int, mtime: int) -> bytes:
    body = f":{escape_name(name)}:{size:x}:{attr:x}:{FILE_MTIME:x}={mtime:x}:".encode("utf-8")
    # header-size (4 cifre esadecimali) comprende se stesso
    total = len(body) + 4
    return f"{total:04x}".encode() + body


def parse_dir_header(header: bytes) -> tuple[str, int, int, int]:
    """Restituisce (nome, dimensione, attributo, mtime) da un'intestazione completa."""
    text = decode_bytes(header, True)
    f = _split_fields(text)
    # f[0]=header-size, f[1]=nome, f[2]=size, f[3]=attr, poi attributi estesi
    name, size, attr = f[1], int(f[2], 16), int(f[3], 16)
    mtime = 0
    for ext in f[4:]:
        if "=" in ext:
            k, v = ext.split("=", 1)
            try:
                if int(k, 16) == FILE_MTIME:
                    mtime = int(v.split(",")[0], 16)
            except ValueError:
                pass
    return name, size, attr, mtime
