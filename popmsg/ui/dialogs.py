"""Impostazioni, registro messaggi e informazioni."""
from __future__ import annotations

import os

from PySide6.QtCore import Qt, QUrl
from PySide6.QtGui import QDesktopServices, QTextCursor, QTextDocument
from PySide6.QtWidgets import (QCheckBox, QComboBox, QDialog, QDialogButtonBox, QFileDialog,
                               QFormLayout, QHBoxLayout, QLabel, QLineEdit, QPlainTextEdit,
                               QPushButton, QSpinBox, QTabWidget, QVBoxLayout, QWidget)

from ..config import Config
from ..version import APP_NAME, APP_VERSION
from .icons import app_icon


class SettingsDialog(QDialog):
    def __init__(self, cfg: Config, groups: list[str], parent=None):
        super().__init__(parent)
        self.cfg = cfg
        self.setWindowTitle(f"Impostazioni di {APP_NAME}")
        self.setWindowIcon(app_icon())
        self.setMinimumWidth(460)
        tabs = QTabWidget()

        # --- Generale
        gen = QWidget()
        f = QFormLayout(gen)
        self.nick = QLineEdit(cfg.nick)
        self.group = QComboBox()
        self.group.setEditable(True)
        self.group.addItems(sorted(set(groups) | ({cfg.group} if cfg.group else set())))
        self.group.setCurrentText(cfg.group)
        f.addRow("Nome visualizzato:", self.nick)
        f.addRow("Gruppo:", self.group)
        self.popup = QCheckBox("Apri subito la finestra quando arriva un messaggio")
        self.popup.setChecked(cfg.popup)
        self.sound = QCheckBox("Suono alla ricezione")
        self.sound.setChecked(cfg.sound)
        self.quote = QCheckBox("Cita il messaggio quando rispondi")
        self.quote.setChecked(cfg.quote_default)
        self.seal = QCheckBox("Sigilla i messaggi per impostazione predefinita")
        self.seal.setChecked(cfg.seal_default)
        for w in (self.popup, self.sound, self.quote, self.seal):
            f.addRow(w)
        self.autostart = QCheckBox("Avvia POPMSG all'accesso a Windows")
        self.autostart.setChecked(cfg.autostart)
        self.autostart.setVisible(Config.autostart_supported())
        f.addRow(self.autostart)
        dl = QHBoxLayout()
        self.download = QLineEdit(cfg.download_dir)
        b = QPushButton("…")
        b.setFixedWidth(30)
        b.clicked.connect(lambda: self._pick_dir(self.download))
        dl.addWidget(self.download)
        dl.addWidget(b)
        f.addRow("Cartella per i file:", dl)
        tabs.addTab(gen, "Generale")

        # --- Rete
        net = QWidget()
        n = QFormLayout(net)
        self.port = QSpinBox()
        self.port.setRange(1, 65535)
        self.port.setValue(cfg.port)
        n.addRow("Porta (UDP e TCP):", self.port)
        n.addRow(QLabel("<small>La porta standard di IP Messenger è 2425. "
                        "Le modifiche alla porta richiedono il riavvio.</small>"))
        self.bcast = QPlainTextEdit("\n".join(cfg.extra_broadcast))
        self.bcast.setPlaceholderText("Un indirizzo per riga, ad esempio\n192.168.2.255\n10.0.5.20\n10.0.6.30:2425")
        n.addRow("Indirizzi aggiuntivi\n(altre sottoreti, host):", self.bcast)
        tabs.addTab(net, "Rete")

        # --- Registro
        lg = QWidget()
        l = QFormLayout(lg)
        self.log_on = QCheckBox("Registra i messaggi inviati e ricevuti")
        self.log_on.setChecked(cfg.log_enabled)
        l.addRow(self.log_on)
        ll = QHBoxLayout()
        self.log_path = QLineEdit(cfg.log_path)
        b2 = QPushButton("…")
        b2.setFixedWidth(30)
        b2.clicked.connect(self._pick_log)
        ll.addWidget(self.log_path)
        ll.addWidget(b2)
        l.addRow("File di registro:", ll)
        tabs.addTab(lg, "Registro")

        bb = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel)
        bb.button(QDialogButtonBox.StandardButton.Cancel).setText("Annulla")
        bb.accepted.connect(self.accept)
        bb.rejected.connect(self.reject)
        lay = QVBoxLayout(self)
        lay.addWidget(tabs)
        lay.addWidget(bb)

    def _pick_dir(self, edit: QLineEdit):
        d = QFileDialog.getExistingDirectory(self, "Scegli cartella", edit.text())
        if d:
            edit.setText(d)

    def _pick_log(self):
        p, _ = QFileDialog.getSaveFileName(self, "File di registro", self.log_path.text(),
                                           "Registro (*.log *.txt)")
        if p:
            self.log_path.setText(p)

    def accept(self):
        c = self.cfg
        c.nick = self.nick.text().strip()
        c.group = self.group.currentText().strip()
        c.popup = self.popup.isChecked()
        c.sound = self.sound.isChecked()
        c.quote_default = self.quote.isChecked()
        c.seal_default = self.seal.isChecked()
        c.download_dir = self.download.text().strip()
        c.port = self.port.value()
        c.extra_broadcast = [x.strip() for x in self.bcast.toPlainText().splitlines() if x.strip()]
        c.log_enabled = self.log_on.isChecked()
        c.log_path = self.log_path.text().strip()
        if Config.autostart_supported():
            c.autostart = self.autostart.isChecked()
        super().accept()


class LogViewer(QWidget):
    def __init__(self, path: str, parent=None):
        super().__init__(parent, Qt.WindowType.Window)
        self.path = path
        self.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
        self.setWindowTitle("Registro messaggi")
        self.setWindowIcon(app_icon())
        self.resize(620, 520)
        self.view = QPlainTextEdit()
        self.view.setReadOnly(True)
        self.view.setLineWrapMode(QPlainTextEdit.LineWrapMode.WidgetWidth)
        self.search = QLineEdit()
        self.search.setPlaceholderText("Cerca nel registro…")
        self.search.returnPressed.connect(self._find)
        find = QPushButton("Trova")
        find.clicked.connect(self._find)
        reload_btn = QPushButton("Aggiorna")
        reload_btn.clicked.connect(self.load)
        folder = QPushButton("Apri cartella")
        folder.clicked.connect(lambda: QDesktopServices.openUrl(
            QUrl.fromLocalFile(os.path.dirname(self.path))))
        top = QHBoxLayout()
        top.addWidget(self.search, 1)
        top.addWidget(find)
        top.addWidget(reload_btn)
        top.addWidget(folder)
        lay = QVBoxLayout(self)
        lay.addLayout(top)
        lay.addWidget(self.view)
        self.load()

    def load(self):
        try:
            with open(self.path, encoding="utf-8", errors="replace") as fh:
                self.view.setPlainText(fh.read())
        except OSError:
            self.view.setPlainText("(Il registro è vuoto.)")
        self.view.moveCursor(QTextCursor.MoveOperation.End)

    def _find(self):
        t = self.search.text()
        if t and not self.view.find(t, QTextDocument.FindFlag.FindBackward):
            self.view.moveCursor(QTextCursor.MoveOperation.End)
            self.view.find(t, QTextDocument.FindFlag.FindBackward)


ABOUT_HTML = f"""
<h2>{APP_NAME} {APP_VERSION}</h2>
<p>Messaggistica istantanea per reti locali, senza server.</p>
<p>Compatibile con il protocollo di IP Messenger: scoperta automatica degli utenti,
messaggi sigillati, modalità assenza, invio di file e cartelle.</p>
<p><small>Porta UDP/TCP 2425 · Python + Qt</small></p>
"""
