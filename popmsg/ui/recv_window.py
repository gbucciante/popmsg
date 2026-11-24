"""Finestra "Messaggio ricevuto" in stile IP Messenger."""
from __future__ import annotations

import os
import threading
import time

from PySide6.QtCore import QDateTime, QLocale, QObject, Qt, QUrl, Signal
from PySide6.QtGui import QDesktopServices, QKeySequence, QShortcut
from PySide6.QtWidgets import (QCheckBox, QFileDialog, QFrame, QHBoxLayout, QLabel, QMessageBox,
                               QPlainTextEdit, QProgressDialog, QPushButton, QStackedWidget,
                               QVBoxLayout, QWidget)

from .. import protocol as P
from ..engine import Engine, Message
from .icons import app_icon


def human_size(n: int) -> str:
    for unit in ("B", "KB", "MB", "GB"):
        if n < 1024 or unit == "GB":
            return f"{n:.0f} {unit}" if unit == "B" else f"{n:.1f} {unit}"
        n /= 1024
    return str(n)


class _Bridge(QObject):
    progress = Signal(int, int, str)
    done = Signal(str)
    failed = Signal(str)


class RecvWindow(QWidget):
    replyRequested = Signal(object, str)     # Message, testo citato (o "")
    opened = Signal(object)                  # Message (sigillo aperto)

    def __init__(self, engine: Engine, msg: Message, quote_default=True,
                 download_dir: str = "", parent=None):
        super().__init__(parent, Qt.WindowType.Window)
        self.engine = engine
        self.msg = msg
        self.download_dir = download_dir
        self._downloaded: set[int] = set()
        self.setWindowTitle("Messaggio ricevuto")
        self.setWindowIcon(app_icon())
        self.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
        self.resize(400, 300)

        # --- intestazione "Messaggio da"
        head = QFrame()
        head.setFrameShape(QFrame.Shape.StyledPanel)
        hl = QVBoxLayout(head)
        hl.setContentsMargins(10, 6, 10, 6)
        hl.setSpacing(2)
        kind = "Messaggio da"
        if msg.command & P.BROADCASTOPT:
            kind = "Messaggio a tutti da"
        elif msg.command & P.MULTICASTOPT:
            kind = "Messaggio di gruppo da"
        if msg.command & P.AUTORETOPT:
            kind = "Risposta automatica da"
        cap = QLabel(kind)
        cap.setStyleSheet("color: palette(dark);")
        s = msg.sender
        who = s.display_name
        detail = "/".join(x for x in (s.group, s.host) if x)
        name = QLabel(f"<b>{_esc(who)}</b>" + (f"  ({_esc(detail)})" if detail else ""))
        name.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
        when = QLabel(QLocale.system().toString(QDateTime.fromSecsSinceEpoch(int(msg.received_at)),
                                                 "dddd d MMMM, HH:mm:ss"))
        when.setStyleSheet("color: palette(dark);")
        hl.addWidget(cap)
        hl.addWidget(name)
        hl.addWidget(when)

        # --- testo (o pulsante "Apri" se sigillato)
        self.text = QPlainTextEdit(msg.text)
        self.text.setReadOnly(True)
        self.stack = QStackedWidget()
        self.open_btn = QPushButton("Apri\n(messaggio sigillato)")
        f = self.open_btn.font()
        f.setPointSizeF(f.pointSizeF() * 1.3)
        f.setBold(True)
        self.open_btn.setFont(f)
        self.open_btn.setMinimumHeight(110)
        self.open_btn.clicked.connect(self._open_seal)
        self.stack.addWidget(self.text)
        self.stack.addWidget(self.open_btn)
        self.stack.setCurrentIndex(1 if msg.sealed else 0)

        # --- allegati
        self.files_box = QWidget()
        fl = QVBoxLayout(self.files_box)
        fl.setContentsMargins(0, 0, 0, 0)
        fl.setSpacing(3)
        self.file_buttons: list[QPushButton] = []
        for info in msg.files:
            icon = "📁" if info.is_dir else "📄"
            label = f"{icon}  {info.name}" + ("" if info.is_dir else f"   ({human_size(info.size)})")
            b = QPushButton(label)
            b.setStyleSheet("text-align: left; padding: 4px 8px;")
            b.setToolTip("Salva…")
            b.clicked.connect(lambda _=False, i=info, btn=b: self._save(i, btn))
            fl.addWidget(b)
            self.file_buttons.append(b)
        self.files_box.setVisible(bool(msg.files) and not msg.sealed)

        # --- pulsanti
        self.quote = QCheckBox("Cita")
        self.quote.setChecked(quote_default)
        close_btn = QPushButton("Chiudi")
        close_btn.clicked.connect(self.close)
        self.reply_btn = QPushButton("Rispondi")
        self.reply_btn.setDefault(True)
        self.reply_btn.setMinimumWidth(100)
        self.reply_btn.clicked.connect(self._reply)
        bl = QHBoxLayout()
        bl.addWidget(close_btn)
        bl.addStretch()
        bl.addWidget(self.reply_btn)
        bl.addWidget(self.quote)

        lay = QVBoxLayout(self)
        lay.setContentsMargins(8, 8, 8, 8)
        lay.addWidget(head)
        lay.addWidget(self.stack, 1)
        lay.addWidget(self.files_box)
        lay.addLayout(bl)

        QShortcut(QKeySequence("Escape"), self, activated=self.close)
        if msg.sealed:
            self.open_btn.setFocus()
        else:
            self.reply_btn.setFocus()

    # ----------------------------------------------------------------------
    def _open_seal(self):
        self.stack.setCurrentIndex(0)
        self.files_box.setVisible(bool(self.msg.files))
        self.engine.mark_read(self.msg)
        self.opened.emit(self.msg)
        self.reply_btn.setFocus()

    def _reply(self):
        if self.stack.currentIndex() == 1:
            self._open_seal()
        quoted = ""
        if self.quote.isChecked() and self.msg.text.strip():
            quoted = "\n".join("> " + line for line in self.msg.text.rstrip("\n").split("\n")) + "\n"
        self.replyRequested.emit(self.msg, quoted)
        self.close()

    def _save(self, info: P.FileInfo, btn: QPushButton):
        if info.is_dir:
            dest = QFileDialog.getExistingDirectory(self, f"Salva la cartella «{info.name}» in…",
                                                    self.download_dir)
        else:
            path, _ = QFileDialog.getSaveFileName(self, "Salva file",
                                                  os.path.join(self.download_dir, info.name))
            dest = os.path.dirname(path) if path else ""
            if path and os.path.basename(path) != info.name:
                info = P.FileInfo(info.file_id, os.path.basename(path), info.size, info.mtime, info.attr)
                if os.path.exists(path):
                    os.remove(path)
            elif path and os.path.exists(path):
                os.remove(path)
        if not dest:
            return
        self.download_dir = dest
        dlg = QProgressDialog(f"Ricezione di «{info.name}»…", "Annulla", 0,
                              info.size if not info.is_dir and info.size else 0, self)
        dlg.setWindowTitle("Ricezione file")
        dlg.setWindowModality(Qt.WindowModality.WindowModal)
        dlg.setMinimumDuration(300)
        cancel = threading.Event()
        dlg.canceled.connect(cancel.set)
        bridge = _Bridge(self)
        self._bridge = bridge

        def on_progress(got, total, name):
            if total:
                dlg.setMaximum(total)
                dlg.setValue(min(got, total))
            else:
                dlg.setLabelText(f"Ricezione di «{name}»…  {human_size(got)}")

        def on_done(path):
            dlg.reset()
            self._downloaded.add(info.file_id)
            btn.setText(btn.text() + "   ✓ salvato")
            btn.clicked.disconnect()
            btn.clicked.connect(lambda: QDesktopServices.openUrl(QUrl.fromLocalFile(path)))
            btn.setToolTip(f"Apri {path}")
            if len(self._downloaded) == len(self.msg.files):
                self.engine.release_files(self.msg)

        def on_failed(err):
            dlg.reset()
            if not cancel.is_set():
                QMessageBox.warning(self, "Ricezione file", f"Impossibile ricevere «{info.name}»:\n{err}")

        bridge.progress.connect(on_progress)
        bridge.done.connect(on_done)
        bridge.failed.connect(on_failed)

        def work():
            try:
                path = self.engine.download(
                    self.msg, info, dest,
                    progress=lambda g, t, n: bridge.progress.emit(g, t, n), cancel=cancel)
                bridge.done.emit(path)
            except Exception as exc:  # noqa: BLE001
                bridge.failed.emit(str(exc))

        threading.Thread(target=work, daemon=True).start()


def _esc(s: str) -> str:
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
