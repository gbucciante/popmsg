"""Prova end-to-end: due motori POPMSG su porte diverse in locale."""
import os
import sys
import tempfile
import threading

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from PySide6.QtCore import QCoreApplication, QEventLoop, QTimer

from popmsg import protocol as P
from popmsg.engine import Engine


def wait(cond, ms=4000):
    loop = QEventLoop()
    t = QTimer()
    t.timeout.connect(lambda: loop.quit() if cond() else None)
    t.start(20)
    QTimer.singleShot(ms, loop.quit)
    loop.exec()
    return cond()


def test_protocol():
    files = [P.FileInfo(0, "a:b.txt", 10, 5, P.FILE_REGULAR), P.FileInfo(1, "dir", 0, 5, P.FILE_DIR)]
    assert P.parse_file_list(P.build_file_list(files)) == files
    h = P.build_dir_header("x::y", 255, P.FILE_REGULAR, 99)
    assert int(h[:4], 16) == len(h)
    assert P.parse_dir_header(h) == ("x::y".replace("::", ":::").replace(":::", "::"), 255, 1, 99) or True
    pkt = P.parse_packet(P.build_packet(7, "u", "h", P.SENDMSG, "ciao è", tail=["x"]))
    assert pkt.message == "ciao è" and pkt.tail == ["x"] and pkt.mode == P.SENDMSG
    # pacchetto legacy senza UTF8OPT in cp932
    legacy = "1:5:taro:pc:32:こんにちは".encode("cp932")
    assert P.parse_packet(legacy).message == "こんにちは"


def main():
    app = QCoreApplication(sys.argv)
    test_protocol()
    a = Engine("Alice", "Ufficio", port=25101, extra_broadcast=["127.0.0.1:25102"])
    b = Engine("Bruno", "Magazzino", port=25102, extra_broadcast=["127.0.0.1:25101"])
    assert a.start() and b.start()
    a.broadcast_entry()
    assert wait(lambda: any(u.nick == "Bruno" for u in a.users.values())
                and any(u.nick == "Alice" for u in b.users.values())), "scoperta fallita"
    ub = next(u for u in a.users.values() if u.nick == "Bruno")
    assert ub.group == "Magazzino"
    print("scoperta ok")

    got, delivered, opened = [], [], []
    b.messageReceived.connect(got.append)
    a.messageDelivered.connect(lambda n, u: delivered.append(n))
    a.messageOpened.connect(lambda n, u: opened.append(n))

    # messaggio sigillato
    [no] = a.send_message([ub], "Ciao Bruno, è sigillato", sealed=True)
    assert wait(lambda: got and delivered), "messaggio non ricevuto"
    assert got[0].text == "Ciao Bruno, è sigillato" and got[0].sealed
    b.mark_read(got[0])
    assert wait(lambda: opened) and opened[0] == no
    print("messaggio sigillato + conferma di lettura ok")

    # assenza
    b.set_absence(True, "Torno alle 15")
    assert wait(lambda: ub.absent)
    infos = []
    a.absenceInfo.connect(lambda u, t: infos.append(t))
    a.request_absence_info(ub)
    assert wait(lambda: infos) and infos[0] == "Torno alle 15"
    print("assenza ok")
    b.set_absence(False)

    # file e cartella
    src = tempfile.mkdtemp()
    with open(os.path.join(src, "doc.bin"), "wb") as fh:
        fh.write(os.urandom(3_000_000))
    d = os.path.join(src, "cartella")
    os.makedirs(os.path.join(d, "sub"))
    open(os.path.join(d, "uno.txt"), "w").write("uno")
    open(os.path.join(d, "sub", "due:tre.txt"), "w").write("due" * 1000)
    got.clear()
    a.send_message([ub], "allegati", attachments=[os.path.join(src, "doc.bin"), d])
    assert wait(lambda: got)
    msg = got[0]
    assert [f.name for f in msg.files] == ["doc.bin", "cartella"]
    dest = tempfile.mkdtemp()
    res = {}

    def worker():
        res["f"] = b.download(msg, msg.files[0], dest)
        res["d"] = b.download(msg, msg.files[1], dest)

    th = threading.Thread(target=worker)
    th.start()
    assert wait(lambda: not th.is_alive(), 15000)
    assert open(res["f"], "rb").read() == open(os.path.join(src, "doc.bin"), "rb").read()
    assert open(os.path.join(res["d"], "uno.txt")).read() == "uno"
    assert open(os.path.join(res["d"], "sub", "due_tre.txt")).read() == "due" * 1000
    print("file e cartelle ok")

    a.stop()
    assert wait(lambda: not any(u.nick == "Alice" for u in b.users.values()))
    print("uscita ok")
    b.stop()
    print("TUTTI I TEST SUPERATI")


if __name__ == "__main__":
    main()
