"""Icone di POPMSG disegnate al volo (nessun asset esterno)."""
from __future__ import annotations

from PySide6.QtCore import QPointF, QRectF, Qt
from PySide6.QtGui import (QBrush, QColor, QFont, QIcon, QLinearGradient, QPainter, QPainterPath,
                           QPen, QPixmap)

ACCENT = QColor("#F2663A")      # arancio corallo
ACCENT_DARK = QColor("#C94420")
AWAY = QColor("#8A8F98")
AWAY_DARK = QColor("#5E636B")
ALERT = QColor("#2F80ED")


def _draw(size: int, top: QColor, bottom: QColor, badge: QColor | None = None) -> QPixmap:
    pm = QPixmap(size, size)
    pm.fill(Qt.GlobalColor.transparent)
    p = QPainter(pm)
    p.setRenderHint(QPainter.RenderHint.Antialiasing)
    s = size / 64.0

    # fumetto arrotondato con coda in basso a sinistra
    path = QPainterPath()
    path.addRoundedRect(QRectF(4 * s, 6 * s, 56 * s, 42 * s), 14 * s, 14 * s)
    tail = QPainterPath()
    tail.moveTo(14 * s, 44 * s)
    tail.lineTo(10 * s, 60 * s)
    tail.lineTo(28 * s, 46 * s)
    tail.closeSubpath()
    path = path.united(tail)
    grad = QLinearGradient(0, 6 * s, 0, 60 * s)
    grad.setColorAt(0, top)
    grad.setColorAt(1, bottom)
    p.setPen(QPen(bottom.darker(115), max(1.0, 1.5 * s)))
    p.setBrush(QBrush(grad))
    p.drawPath(path)

    # tre "pop": puntini bianchi di dimensione crescente
    p.setPen(Qt.PenStyle.NoPen)
    p.setBrush(QColor("white"))
    for cx, r in ((18, 4.2), (32, 5.2), (46, 6.2)):
        p.drawEllipse(QPointF(cx * s, 27 * s), r * s, r * s)

    if badge is not None:
        p.setBrush(badge)
        p.setPen(QPen(QColor("white"), max(1.0, 2 * s)))
        p.drawEllipse(QPointF(52 * s, 12 * s), 10 * s, 10 * s)
    p.end()
    return pm


def _icon(top, bottom, badge=None) -> QIcon:
    icon = QIcon()
    for sz in (16, 20, 24, 32, 48, 64, 128, 256):
        icon.addPixmap(_draw(sz, top, bottom, badge))
    return icon


def app_icon() -> QIcon:
    return _icon(ACCENT.lighter(115), ACCENT_DARK)


def away_icon() -> QIcon:
    return _icon(AWAY.lighter(115), AWAY_DARK)


def unread_icon() -> QIcon:
    return _icon(ACCENT.lighter(115), ACCENT_DARK, ALERT)


def render_png(path: str, size: int = 256):
    _draw(size, ACCENT.lighter(115), ACCENT_DARK).save(path, "PNG")
