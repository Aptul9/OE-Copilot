# -*- mode: python ; coding: utf-8 -*-
"""Spec PyInstaller per Calendar Hub.

Punti delicati raccolti esplicitamente:
  - pywebview + backend EdgeChromium (le DLL del loader WebView2);
  - pythonnet / clr_loader (assembly .NET);
  - winrt.* (usati da win11toast per le notifiche);
  - icalendar / recurring_ical_events + dati fuso orario (tzdata).
Build a cartella (onedir): più affidabile di onefile con questo stack.
"""
from PyInstaller.utils.hooks import collect_all, collect_submodules

datas = [("web", "web"), ("icon.ico", ".")]
binaries = []
hiddenimports = []

# Pacchetti da raccogliere per intero (moduli + dati + DLL).
for pkg in ("webview", "clr_loader", "icalendar", "recurring_ical_events",
            "tzdata", "win11toast"):
    d, b, h = collect_all(pkg)
    datas += d
    binaries += b
    hiddenimports += h

# winrt è un namespace con molti sotto-moduli: quelli che servono a win11toast.
hiddenimports += collect_submodules("winrt")
hiddenimports += [
    "pythonnet", "clr",
    "win32timezone", "win32com", "win32com.client",
]

a = Analysis(
    ["avvia.pyw"],
    pathex=[],
    binaries=binaries,
    datas=datas,
    hiddenimports=hiddenimports,
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=["tkinter", "matplotlib", "numpy", "PySide2", "PyQt5"],
    noarchive=False,
)

pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    [],
    exclude_binaries=True,
    name="CalendarHub",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=False,
    console=False,          # niente finestra nera
    icon="icon.ico",
)

coll = COLLECT(
    exe,
    a.binaries,
    a.datas,
    strip=False,
    upx=False,
    name="CalendarHub",
)

