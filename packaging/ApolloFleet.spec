# -*- mode: python ; coding: utf-8 -*-
# Run from project root:  pyinstaller packaging/ApolloFleet.spec


a = Analysis(
    ['entry.py'],
    pathex=['../src'],
    binaries=[],
    datas=[('../config/seats.toml.example', '.')],
    hiddenimports=['tkinter', 'tkinter.simpledialog', 'tkinter.messagebox', 'apollo_fleet', 'apollo_fleet.supervisor', 'apollo_fleet.tray'],
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[],
    noarchive=False,
    optimize=0,
)
pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.datas,
    [],
    name='ApolloFleet',
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=True,
    upx_exclude=[],
    runtime_tmpdir=None,
    console=False,
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
)
