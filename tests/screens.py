"""Genera screenshot delle finestre principali (QT_QPA_PLATFORM=offscreen)."""
import os, sys, time
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from PySide6.QtWidgets import QApplication
from PySide6.QtCore import QTimer, QEventLoop
from popmsg.engine import Engine, Message, User
from popmsg import protocol as P
from popmsg.ui.send_window import SendWindow
from popmsg.ui.recv_window import RecvWindow
from popmsg.ui.icons import app_icon, render_png

out = sys.argv[1]
app = QApplication(sys.argv)
app.setStyle("Fusion")
e = Engine("Gianni", "Amministrazione", port=25201)
e.start()
demo = [("192.168.1.10","Marta Rossi","Amministrazione","PC-MARTA",0),
        ("192.168.1.11","Luca Bianchi","Magazzino","MAG-01",0),
        ("192.168.1.12","Sara Verdi","Commerciale","NB-SARA",P.ABSENCEOPT),
        ("192.168.1.20","Paolo","Tecnico","SRV-TEC",0),
        ("192.168.1.31","Elena Neri","Commerciale","NB-ELENA",0)]
for ip,n,g,h,o in demo:
    e.users[(ip,2425)] = User(ip,2425,n.split()[0].lower(),h,n,g,o)
w = SendWindow(e); w.show(); w.select_users([("192.168.1.10",2425)])
w.set_text("Ciao Marta, ti mando il listino aggiornato.")
w.add_attachments([os.path.abspath(__file__)])
sender = e.users[("192.168.1.11",2425)]
m = Message(1, sender, "Ciao! Il corriere è arrivato, i pacchi sono in magazzino.\nPassi tu a ritirarli?", P.SENDMSG, time.time(),
            [P.FileInfo(0,"bolla_consegna.pdf",184320,0,1), P.FileInfo(1,"foto",0,0,2)])
r = RecvWindow(e, m); r.move(500,0); r.show()
m2 = Message(2, sender, "segreto", P.SENDMSG|P.SECRETOPT, time.time())
r2 = RecvWindow(e, m2); r2.show()
loop = QEventLoop(); QTimer.singleShot(400, loop.quit); loop.exec()
for name, win in (("send", w), ("recv", r), ("sealed", r2)):
    win.grab().save(os.path.join(out, f"{name}.png"))
render_png(os.path.join(out, "icon.png"))
e.stop()
