"""Finestra "Invia messaggio" in stile IP Messenger."""
from __future__ import annotations

import os

from PySide6.QtCore import Qt, Signal
from PySide6.QtGui import QAction, QColor, QFont, QKeySequence, QShortcut
from PySide6.QtWidgets import (QAbstractItemView, QCheckBox, QFileDialog, QHBoxLayout, QHeaderView,
                               QLabel, QMenu, QPlainTextEdit, QPushButton, QSplitter, QToolButton,
                               QTreeWidget, QTreeWidgetItem, QVBoxLayout, QWidget)

from ..engine import Engine, User
from .icons import app_icon


class _UserItem(QTreeWidgetItem):
    def __lt__(self, other):
        col = self.treeWidget().sortColumn() if self.treeWidget() else 0
        if col == 3:  # ordina gli IP numericamente
            def key(it):
                try:
                    return tuple(int(x) for x in it.text(3).split("."))
                except ValueError:
                    return (999,)
            return key(self) < key(other)
        return self.text(col).lower() < other.text(col).lower()


class _MessageEdit(QPlainTextEdit):
    filesDropped = Signal(list)
    sendRequested = Signal()

    def __init__(self, parent=None):
        super().__init__(parent)
        self.setAcceptDrops(True)

    def dragEnterEvent(self, e):
        if e.mimeData().hasUrls():
            e.acceptProposedAction()
        else:
            super().dragEnterEvent(e)

    def dragMoveEvent(self, e):
        if e.mimeData().hasUrls():
            e.acceptProposedAction()
        else:
            super().dragMoveEvent(e)

    def dropEvent(self, e):
        if e.mimeData().hasUrls():
            paths = [u.toLocalFile() for u in e.mimeData().urls() if u.isLocalFile()]
            if paths:
                self.filesDropped.emit(paths)
            e.acceptProposedAction()
        else:
            super().dropEvent(e)

    def keyPressEvent(self, e):
        # Ctrl+Invio invia, come in IP Messenger
        if e.key() in (Qt.Key.Key_Return, Qt.Key.Key_Enter) and \
                e.modifiers() & Qt.KeyboardModifier.ControlModifier:
            self.sendRequested.emit()
            return
        super().keyPressEvent(e)


class SendWindow(QWidget):
    sent = Signal(list, str, bool, list)   # utenti, testo, sigillato, allegati

    def __init__(self, engine: Engine, seal_default=False, parent=None):
        super().__init__(parent, Qt.WindowType.Window)
        self.engine = engine
        self.attachments: list[str] = []
        self.setWindowTitle("Invia messaggio")
        self.setWindowIcon(app_icon())
        self.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
        self.resize(540, 420)
        self.setAcceptDrops(True)

        # --- lista utenti + contatore a destra
        self.tree = QTreeWidget()
        self.tree.setColumnCount(4)
        self.tree.setHeaderLabels(["Utente", "Gruppo", "Host", "Indirizzo IP"])
        self.tree.setRootIsDecorated(False)
        self.tree.setAlternatingRowColors(True)
        self.tree.setSelectionMode(QAbstractItemView.SelectionMode.ExtendedSelection)
        self.tree.setSortingEnabled(True)
        self.tree.sortByColumn(1, Qt.SortOrder.AscendingOrder)
        self.tree.setUniformRowHeights(True)
        hdr = self.tree.header()
        hdr.setStretchLastSection(False)
        hdr.setSectionResizeMode(0, QHeaderView.ResizeMode.Stretch)
        hdr.setMinimumSectionSize(60)
        for c in (1, 2, 3):
            hdr.setSectionResizeMode(c, QHeaderView.ResizeMode.ResizeToContents)
        self.tree.setContextMenuPolicy(Qt.ContextMenuPolicy.CustomContextMenu)
        self.tree.customContextMenuRequested.connect(self._user_menu)
        self.tree.itemSelectionChanged.connect(self._update_send_enabled)
        self.tree.itemDoubleClicked.connect(lambda *_: self.edit.setFocus())

        side = QVBoxLayout()
        side.setContentsMargins(4, 0, 0, 0)
        lbl = QLabel("Utenti")
        lbl.setAlignment(Qt.AlignmentFlag.AlignHCenter)
        self.count = QLabel("0")
        self.count.setAlignment(Qt.AlignmentFlag.AlignHCenter)
        f = self.count.font()
        f.setPointSizeF(f.pointSizeF() * 1.6)
        f.setBold(True)
        self.count.setFont(f)
        self.refresh_btn = QPushButton("Aggiorna")
        self.refresh_btn.clicked.connect(self.engine.refresh)
        side.addWidget(lbl)
        side.addWidget(self.count)
        side.addWidget(self.refresh_btn)
        side.addStretch()

        top = QWidget()
        tl = QHBoxLayout(top)
        tl.setContentsMargins(0, 0, 0, 0)
        tl.addWidget(self.tree, 1)
        tl.addLayout(side)

        # --- area testo
        self.edit = _MessageEdit()
        self.edit.filesDropped.connect(self.add_attachments)
        self.edit.sendRequested.connect(self._send)
        self.edit.setContextMenuPolicy(Qt.ContextMenuPolicy.CustomContextMenu)
        self.edit.customContextMenuRequested.connect(self._edit_menu)

        split = QSplitter(Qt.Orientation.Vertical)
        split.addWidget(top)
        split.addWidget(self.edit)
        split.setStretchFactor(0, 1)
        split.setStretchFactor(1, 1)
        split.setChildrenCollapsible(False)

        # --- allegati
        self.att_label = QLabel()
        self.att_label.setWordWrap(True)
        self.att_label.setStyleSheet("color: palette(dark);")
        self.att_label.hide()

        # --- barra inferiore
        self.attach_btn = QToolButton()
        self.attach_btn.setText("Allega")
        self.attach_btn.setPopupMode(QToolButton.ToolButtonPopupMode.InstantPopup)
        m = QMenu(self)
        m.addAction("File…", self._pick_files)
        m.addAction("Cartella…", self._pick_folder)
        m.addSeparator()
        self.clear_att_action = m.addAction("Rimuovi allegati", self.clear_attachments)
        self.attach_btn.setMenu(m)

        self.seal = QCheckBox("Sigilla")
        self.seal.setChecked(seal_default)
        self.seal.setToolTip("Il destinatario deve aprire il messaggio; riceverai un avviso quando lo apre")
        self.send_btn = QPushButton("Invia")
        self.send_btn.setDefault(True)
        self.send_btn.setMinimumWidth(110)
        self.send_btn.setMinimumHeight(30)
        bf = self.send_btn.font()
        bf.setBold(True)
        self.send_btn.setFont(bf)
        self.send_btn.clicked.connect(self._send)

        bottom = QHBoxLayout()
        bottom.addWidget(self.attach_btn)
        bottom.addSpacing(6)
        bottom.addWidget(self.seal)
        bottom.addStretch()
        bottom.addWidget(self.send_btn)

        lay = QVBoxLayout(self)
        lay.setContentsMargins(8, 8, 8, 8)
        lay.addWidget(split, 1)
        lay.addWidget(self.att_label)
        lay.addLayout(bottom)

        QShortcut(QKeySequence("Escape"), self, activated=self.close)
        QShortcut(QKeySequence("F5"), self, activated=self.engine.refresh)

        self.engine.usersChanged.connect(self.reload_users)
        self.reload_users()
        self.edit.setFocus()

    # ---------------------------------------------------------------- utenti
    def reload_users(self):
        selected = {it.data(0, Qt.ItemDataRole.UserRole) for it in self.tree.selectedItems()}
        self.tree.setSortingEnabled(False)
        self.tree.clear()
        grey = QColor(self.palette().color(self.palette().ColorRole.PlaceholderText))
        for user in self.engine.users.values():
            name = user.display_name
            it = _UserItem([name, user.group, user.host, user.ip if user.port == self.engine.port
                            else f"{user.ip}:{user.port}"])
            it.setData(0, Qt.ItemDataRole.UserRole, user.key)
            it.setToolTip(0, f"{name}\nAccesso: {user.login}\nHost: {user.host}\nIP: {user.ip}")
            if user.absent:
                for c in range(4):
                    it.setForeground(c, grey)
            self.tree.addTopLevelItem(it)
            if user.key in selected:
                it.setSelected(True)
        self.tree.setSortingEnabled(True)
        self.count.setText(str(len(self.engine.users)))
        self._update_send_enabled()

    def select_users(self, keys):
        keys = set(keys)
        for i in range(self.tree.topLevelItemCount()):
            it = self.tree.topLevelItem(i)
            if it.data(0, Qt.ItemDataRole.UserRole) in keys:
                it.setSelected(True)
                self.tree.scrollToItem(it)

    def selected_users(self) -> list[User]:
        out = []
        for it in self.tree.selectedItems():
            u = self.engine.users.get(it.data(0, Qt.ItemDataRole.UserRole))
            if u:
                out.append(u)
        return out

    def _update_send_enabled(self):
        self.send_btn.setEnabled(bool(self.tree.selectedItems()))

    def _user_menu(self, pos):
        users = self.selected_users()
        m = QMenu(self)
        m.addAction("Aggiorna elenco", self.engine.refresh)
        m.addAction("Seleziona tutti", self.tree.selectAll)
        if users:
            m.addSeparator()
            m.addAction("Messaggio di assenza…", lambda: [self.engine.request_absence_info(u) for u in users])
            m.addAction("Versione del programma…", lambda: [self.engine.request_version(u) for u in users])
        groups = sorted({u.group for u in self.engine.users.values() if u.group})
        if groups:
            gm = m.addMenu("Seleziona gruppo")
            for g in groups:
                gm.addAction(g, lambda g=g: self._select_group(g))
        m.exec(self.tree.viewport().mapToGlobal(pos))

    def _select_group(self, group):
        self.tree.clearSelection()
        self.select_users(u.key for u in self.engine.users.values() if u.group == group)

    # ------------------------------------------------------------- allegati
    def _edit_menu(self, pos):
        m = self.edit.createStandardContextMenu()
        m.addSeparator()
        m.addAction("Allega file…", self._pick_files)
        m.addAction("Allega cartella…", self._pick_folder)
        if self.attachments:
            m.addAction("Rimuovi allegati", self.clear_attachments)
        m.exec(self.edit.viewport().mapToGlobal(pos))

    def _pick_files(self):
        paths, _ = QFileDialog.getOpenFileNames(self, "Allega file")
        self.add_attachments(paths)

    def _pick_folder(self):
        path = QFileDialog.getExistingDirectory(self, "Allega cartella")
        if path:
            self.add_attachments([path])

    def add_attachments(self, paths):
        for p in paths:
            if p and os.path.exists(p) and p not in self.attachments:
                self.attachments.append(p)
        self._show_attachments()

    def clear_attachments(self):
        self.attachments.clear()
        self._show_attachments()

    def _show_attachments(self):
        if self.attachments:
            names = [os.path.basename(os.path.normpath(p)) + ("/" if os.path.isdir(p) else "")
                     for p in self.attachments]
            self.att_label.setText("📎 " + ",  ".join(names))
            self.att_label.show()
        else:
            self.att_label.hide()

    def dragEnterEvent(self, e):
        if e.mimeData().hasUrls():
            e.acceptProposedAction()

    def dropEvent(self, e):
        self.add_attachments([u.toLocalFile() for u in e.mimeData().urls() if u.isLocalFile()])

    # ---------------------------------------------------------------- invio
    def set_text(self, text: str):
        self.edit.setPlainText(text)
        c = self.edit.textCursor()
        c.movePosition(c.MoveOperation.End)
        self.edit.setTextCursor(c)

    def _send(self):
        users = self.selected_users()
        text = self.edit.toPlainText()
        if not users or (not text.strip() and not self.attachments):
            return
        self.sent.emit(users, text, self.seal.isChecked(), list(self.attachments))
        self.close()
