"""Impostazioni persistenti e registro dei messaggi."""
from __future__ import annotations

import getpass
import os
import sys
import time

from PySide6.QtCore import QSettings, QStandardPaths

from .protocol import DEFAULT_PORT

ABSENCE_PRESETS = [
    ("Assente", "Sono assente in questo momento."),
    ("In riunione", "Sono in riunione, ti rispondo appena posso."),
    ("In pausa", "Sono in pausa, torno a breve."),
    ("Fuori ufficio", "Sono fuori ufficio."),
]


class Config:
    def __init__(self):
        self.s = QSettings("POPMSG", "POPMSG")

    def _get(self, key, default, typ=str):
        v = self.s.value(key, default)
        if typ is bool:
            return v in (True, "true", "1", 1)
        if typ is int:
            try:
                return int(v)
            except (TypeError, ValueError):
                return default
        if typ is list:
            if v is None or v == "":
                return []
            return v if isinstance(v, list) else [v]
        return v if v is not None else default

    # proprietà
    nick = property(lambda self: self._get("nick", getpass.getuser()),
                    lambda self, v: self.s.setValue("nick", v))
    group = property(lambda self: self._get("group", ""),
                     lambda self, v: self.s.setValue("group", v))
    port = property(lambda self: self._get("port", DEFAULT_PORT, int),
                    lambda self, v: self.s.setValue("port", int(v)))
    extra_broadcast = property(lambda self: self._get("extra_broadcast", [], list),
                               lambda self, v: self.s.setValue("extra_broadcast", list(v)))
    log_enabled = property(lambda self: self._get("log_enabled", True, bool),
                           lambda self, v: self.s.setValue("log_enabled", bool(v)))
    sound = property(lambda self: self._get("sound", True, bool),
                     lambda self, v: self.s.setValue("sound", bool(v)))
    quote_default = property(lambda self: self._get("quote_default", True, bool),
                             lambda self, v: self.s.setValue("quote_default", bool(v)))
    seal_default = property(lambda self: self._get("seal_default", False, bool),
                            lambda self, v: self.s.setValue("seal_default", bool(v)))
    popup = property(lambda self: self._get("popup", True, bool),
                     lambda self, v: self.s.setValue("popup", bool(v)))
    download_dir = property(
        lambda self: self._get("download_dir", QStandardPaths.writableLocation(
            QStandardPaths.StandardLocation.DownloadLocation) or os.path.expanduser("~")),
        lambda self, v: self.s.setValue("download_dir", v))
    absence_text = property(lambda self: self._get("absence_text", ABSENCE_PRESETS[0][1]),
                            lambda self, v: self.s.setValue("absence_text", v))

    @property
    def log_path(self) -> str:
        p = self._get("log_path", "")
        if p:
            return p
        base = QStandardPaths.writableLocation(QStandardPaths.StandardLocation.AppDataLocation)
        os.makedirs(base, exist_ok=True)
        return os.path.join(base, "popmsg.log")

    @log_path.setter
    def log_path(self, v):
        self.s.setValue("log_path", v)

    # avvio automatico (solo Windows)
    @staticmethod
    def autostart_supported() -> bool:
        return sys.platform == "win32" and getattr(sys, "frozen", False)

    @property
    def autostart(self) -> bool:
        if sys.platform != "win32":
            return False
        run = QSettings(r"HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run",
                        QSettings.Format.NativeFormat)
        return run.contains("POPMSG")

    @autostart.setter
    def autostart(self, on: bool):
        if sys.platform != "win32":
            return
        run = QSettings(r"HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run",
                        QSettings.Format.NativeFormat)
        if on:
            run.setValue("POPMSG", f'"{sys.executable}"')
        else:
            run.remove("POPMSG")


def write_log(path: str, direction: str, who: str, text: str, flags: list[str], files: list[str]):
    """Scrive una voce nel registro, in un formato simile a quello di IPMsg."""
    sep = "=" * 38
    line = "-" * 38
    when = time.strftime("%d/%m/%Y %H:%M:%S")
    head = f" {direction}: {who}"
    if flags:
        head += "  (" + ", ".join(flags) + ")"
    out = [sep, head, f"  alle {when}"]
    if files:
        out.append("  Allegati: " + ", ".join(files))
    out += [line, text.rstrip("\n"), "", ""]
    try:
        with open(path, "a", encoding="utf-8") as fh:
            fh.write("\n".join(out))
    except OSError:
        pass
