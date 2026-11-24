"""Avvio di POPMSG: icona nella barra di sistema e gestione delle finestre."""
from __future__ import annotations

import sys
import time

from PySide6.QtCore import Qt, QTimer
from PySide6.QtGui import QAction, QActionGroup
from PySide6.QtWidgets import (QApplication, QInputDialog, QMenu, QMessageBox, QSystemTrayIcon)

from .config import ABSENCE_PRESETS, Config, write_log
from .engine import Engine, Message, User
from .ui.dialogs import ABOUT_HTML, LogViewer, SettingsDialog
from .ui.icons import app_icon, away_icon, unread_icon
from .ui.recv_window import RecvWindow
from .ui.send_window import SendWindow
from .version import APP_NAME, APP_VERSION


class Controller:
    def __init__(self, app: QApplication):
        self.app = app
        self.cfg = Config()
        self.engine = Engine(self.cfg.nick, self.cfg.group, self.cfg.port, self.cfg.extra_broadcast)
        self.engine.absence_text = self.cfg.absence_text
        self.windows: set = set()
        self.unread: list[Message] = []
        self._sent: dict[int, tuple] = {}

        e = self.engine
        e.messageReceived.connect(self.on_message)
        e.messageOpened.connect(self.on_opened)
        e.messageFailed.connect(self.on_failed)
        e.absenceInfo.connect(lambda u, t: self._info(f"Messaggio di assenza di {u.display_name}", t))
        e.versionInfo.connect(lambda u, t: self._info(f"Versione usata da {u.display_name}", t))
        e.error.connect(lambda m: print(m, file=sys.stderr))

        self.tray = None
        if QSystemTrayIcon.isSystemTrayAvailable():
            self.tray = QSystemTrayIcon(app_icon())
            self.tray.setToolTip(APP_NAME)
            self.tray.activated.connect(self.on_tray)
            self.tray.messageClicked.connect(self.show_unread)
            self.tray.setContextMenu(self._build_menu())
            self.tray.show()

    # ------------------------------------------------------------------ menu
    def _build_menu(self) -> QMenu:
        m = QMenu()
        m.addAction("Invia messaggio…", self.open_send).setFont(_bold(m))
        m.addSeparator()
        am = m.addMenu("Modalità assenza")
        self.absence_group = QActionGroup(am)
        off = QAction("Presente", am, checkable=True, checked=True)
        off.triggered.connect(lambda: self.set_absence(False))
        self.absence_group.addAction(off)
        am.addAction(off)
        am.addSeparator()
        for label, text in ABSENCE_PRESETS:
            a = QAction(label, am, checkable=True)
            a.triggered.connect(lambda _=False, t=text: self.set_absence(True, t))
            self.absence_group.addAction(a)
            am.addAction(a)
        custom = QAction("Personalizzata…", am, checkable=True)
        custom.triggered.connect(self._custom_absence)
        self.absence_group.addAction(custom)
        am.addAction(custom)
        self._absence_off = off
        m.addAction("Registro messaggi", self.open_log)
        m.addAction("Impostazioni…", self.open_settings)
        m.addAction(f"Informazioni su {APP_NAME}", self.about)
        m.addSeparator()
        m.addAction("Esci", self.quit)
        return m

    def on_tray(self, reason):
        if reason in (QSystemTrayIcon.ActivationReason.DoubleClick,
                      QSystemTrayIcon.ActivationReason.Trigger):
            if self.unread:
                self.show_unread()
            elif reason == QSystemTrayIcon.ActivationReason.DoubleClick or sys.platform != "win32":
                self.open_send()

    # --------------------------------------------------------------- finestre
    def _track(self, w):
        self.windows.add(w)
        w.destroyed.connect(lambda *_: self.windows.discard(w))
        w.show()
        w.raise_()
        w.activateWindow()
        return w

    def open_send(self, users: list[User] | None = None, text: str = ""):
        w = SendWindow(self.engine, self.cfg.seal_default)
        w.sent.connect(self.do_send)
        if users:
            w.select_users([u.key for u in users])
        if text:
            w.set_text(text)
        self._track(w)
        if not self.tray:
            w.destroyed.connect(self._maybe_quit_no_tray)
        return w

    def _maybe_quit_no_tray(self, *_):
        QTimer.singleShot(0, lambda: None if self.windows else self.quit())

    def do_send(self, users, text, sealed, attachments):
        nos = self.engine.send_message(users, text, sealed, attachments)
        for no, u in zip(nos, users):
            self._sent[no] = (u, text, sealed, attachments)
        if self.cfg.log_enabled:
            who = ", ".join(f"{u.display_name} ({u.group}/{u.host})" for u in users)
            flags = ["sigillato"] if sealed else []
            write_log(self.cfg.log_path, "A", who, text, flags,
                      [a.rsplit("/", 1)[-1].rsplit("\\", 1)[-1] for a in attachments])

    def on_message(self, msg: Message):
        if self.cfg.log_enabled:
            s = msg.sender
            flags = []
            if msg.sealed:
                flags.append("sigillato")
            if msg.broadcast:
                flags.append("multiplo")
            write_log(self.cfg.log_path, "Da", f"{s.display_name} ({s.group}/{s.host})",
                      msg.text, flags, [f.name for f in msg.files])
        if self.cfg.sound:
            QApplication.beep()
        if self.cfg.popup or not self.tray:
            self.show_message(msg)
        else:
            self.unread.append(msg)
            self.tray.setIcon(unread_icon())
            self.tray.setToolTip(f"{APP_NAME}: {len(self.unread)} messaggi non letti")
            self.tray.showMessage(APP_NAME, f"Nuovo messaggio da {msg.sender.display_name}",
                                  app_icon(), 5000)

    def show_message(self, msg: Message):
        w = RecvWindow(self.engine, msg, self.cfg.quote_default, self.cfg.download_dir)
        w.replyRequested.connect(lambda m, q: self.open_send([m.sender], q))
        self._track(w)

    def show_unread(self):
        pending, self.unread = self.unread, []
        for m in pending:
            self.show_message(m)
        self._update_tray_icon()

    def on_opened(self, no: int, user: User):
        if no not in self._sent and user is None:
            return
        name = user.display_name if user else "?"
        box = QMessageBox(QMessageBox.Icon.Information, APP_NAME,
                          f"Il messaggio sigillato è stato aperto da {name}.\n"
                          f"({time.strftime('%H:%M:%S')})")
        box.setWindowIcon(app_icon())
        box.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
        box.setWindowModality(Qt.WindowModality.NonModal)
        self._track(box)

    def on_failed(self, no: int, user: User | None):
        data = self._sent.get(no)
        if not data:
            return
        u, text, sealed, att = data
        r = QMessageBox.question(None, APP_NAME,
                                 f"Il messaggio a {u.display_name} non è stato consegnato.\n"
                                 "Vuoi riprovare?",
                                 QMessageBox.StandardButton.Retry | QMessageBox.StandardButton.Cancel)
        if r == QMessageBox.StandardButton.Retry:
            target = self.engine.users.get(u.key, u)
            self.do_send([target], text, sealed, att)

    def _info(self, title, text):
        box = QMessageBox(QMessageBox.Icon.Information, title, text or "(nessun testo)")
        box.setWindowIcon(app_icon())
        box.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
        box.setWindowModality(Qt.WindowModality.NonModal)
        self._track(box)

    # ---------------------------------------------------------------- assenza
    def set_absence(self, on: bool, text: str | None = None):
        if text is not None:
            self.cfg.absence_text = text
        self.engine.set_absence(on, text)
        self._update_tray_icon()

    def _custom_absence(self):
        text, ok = QInputDialog.getMultiLineText(None, "Modalità assenza",
                                                 "Messaggio di risposta automatica:",
                                                 self.cfg.absence_text)
        if ok:
            self.set_absence(True, text)
        elif not self.engine.absent:
            self._absence_off.setChecked(True)

    def _update_tray_icon(self):
        if not self.tray:
            return
        if self.unread:
            self.tray.setIcon(unread_icon())
        elif self.engine.absent:
            self.tray.setIcon(away_icon())
            self.tray.setToolTip(f"{APP_NAME} (assente)")
        else:
            self.tray.setIcon(app_icon())
            self.tray.setToolTip(APP_NAME)

    # --------------------------------------------------------------- altro
    def open_log(self):
        self._track(LogViewer(self.cfg.log_path))

    def open_settings(self):
        groups = [u.group for u in self.engine.users.values() if u.group]
        old_port, old_bc = self.cfg.port, self.cfg.extra_broadcast
        dlg = SettingsDialog(self.cfg, groups)
        if dlg.exec():
            self.engine.extra_broadcast = self.cfg.extra_broadcast
            self.engine.set_identity(self.cfg.nick, self.cfg.group)
            if self.cfg.extra_broadcast != old_bc:
                self.engine.broadcast_entry()
            if self.cfg.port != old_port:
                QMessageBox.information(None, APP_NAME, "La nuova porta sarà usata al prossimo avvio.")

    def about(self):
        box = QMessageBox(QMessageBox.Icon.NoIcon, f"Informazioni su {APP_NAME}", ABOUT_HTML)
        box.setIconPixmap(app_icon().pixmap(64, 64))
        box.setWindowIcon(app_icon())
        box.exec()

    def quit(self):
        self.engine.stop()
        if self.tray:
            self.tray.hide()
        # lascia partire il pacchetto di uscita
        QTimer.singleShot(150, self.app.quit)


def _bold(menu):
    f = menu.font()
    f.setBold(True)
    return f


def main():
    if sys.platform == "win32":
        try:  # icona corretta nella barra delle applicazioni
            import ctypes
            ctypes.windll.shell32.SetCurrentProcessExplicitAppUserModelID("POPMSG.POPMSG")
        except Exception:
            pass
    app = QApplication(sys.argv)
    app.setApplicationName(APP_NAME)
    app.setOrganizationName(APP_NAME)
    app.setApplicationVersion(APP_VERSION)
    app.setWindowIcon(app_icon())
    app.setQuitOnLastWindowClosed(False)
    ctl = Controller(app)
    if not ctl.engine.start():
        QMessageBox.critical(None, APP_NAME,
                             f"Impossibile avviare {APP_NAME} sulla porta {ctl.cfg.port}.\n"
                             "Forse è già in esecuzione un altro programma di messaggistica "
                             "(POPMSG o IP Messenger).")
        return 1
    if not ctl.tray or "--show" in sys.argv:
        ctl.open_send()
    app.aboutToQuit.connect(ctl.engine.stop)
    ret = app.exec()
    # chiusura ordinata: distruggi le finestre prima dell'applicazione
    for w in list(ctl.windows):
        w.deleteLater()
    if ctl.tray:
        ctl.tray.hide()
        ctl.tray.setContextMenu(None)
        ctl.tray.deleteLater()
    ctl.engine.deleteLater()
    QApplication.sendPostedEvents()
    del ctl
    return ret


if __name__ == "__main__":
    sys.exit(main())
