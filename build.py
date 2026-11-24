"""Crea l'eseguibile di POPMSG con PyInstaller (Windows .exe, macOS .app, Linux binario).

Uso:  python build.py
"""
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.abspath(__file__))
ASSETS = os.path.join(ROOT, "assets")


def make_icons():
    os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
    from PySide6.QtGui import QGuiApplication
    from PIL import Image
    sys.path.insert(0, ROOT)
    from popmsg.ui.icons import render_png
    app = QGuiApplication.instance() or QGuiApplication(sys.argv)  # noqa: F841
    os.makedirs(ASSETS, exist_ok=True)
    png = os.path.join(ASSETS, "popmsg.png")
    render_png(png, 512)
    img = Image.open(png)
    img.save(os.path.join(ASSETS, "popmsg.ico"),
             sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    if sys.platform == "darwin":
        img.save(os.path.join(ASSETS, "popmsg.icns"))


def main():
    make_icons()
    icon = {"win32": "popmsg.ico", "darwin": "popmsg.icns"}.get(sys.platform, "popmsg.png")
    args = [
        sys.executable, "-m", "PyInstaller", "--noconfirm", "--clean",
        "--name", "POPMSG", "--windowed",
        "--icon", os.path.join(ASSETS, icon),
        # moduli Qt non usati: eseguibile più piccolo
        "--exclude-module", "PySide6.QtWebEngineCore",
        "--exclude-module", "PySide6.QtQml",
        "--exclude-module", "PySide6.QtQuick",
        "--exclude-module", "PySide6.Qt3DCore",
        "--exclude-module", "PySide6.QtMultimedia",
        "--exclude-module", "PySide6.QtPdf",
        "--exclude-module", "tkinter",
    ]
    if sys.platform != "darwin":
        args.append("--onefile")
    else:
        args += ["--osx-bundle-identifier", "app.popmsg.POPMSG"]
    args.append(os.path.join(ROOT, "run.py"))
    subprocess.check_call(args, cwd=ROOT)
    print("\nFatto: guarda la cartella dist/")


if __name__ == "__main__":
    main()
