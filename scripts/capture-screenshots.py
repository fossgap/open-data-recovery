#!/usr/bin/env python3
"""Capture production GTK windows using a freshly created, disposable image.

Run through capture-preview.sh for an isolated X display. No user sources,
snapshots or settings are opened, and no application state is fabricated.
"""
import argparse
import hashlib
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time

import cairo
import gi
gi.require_version("Rsvg", "2.0")
from gi.repository import Rsvg


def run(*args, **kwargs):
    return subprocess.run([str(a) for a in args], check=True, **kwargs)


def xdo(*args):
    return subprocess.check_output(["xdotool", *map(str, args)], text=True).strip()


def key(window, keys):
    xdo("windowfocus", "--sync", window)
    xdo("key", "--clearmodifiers", keys)


def click(window, x, y):
    xdo("mousemove", "--window", window, x, y, "click", 1)


def find_window(process, title):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        result = subprocess.run(["xdotool", "search", "--onlyvisible", "--all", "--pid", str(process.pid), "--name", title], capture_output=True, text=True)
        if result.returncode == 0 and result.stdout.strip():
            return result.stdout.strip().splitlines()[-1]
        if process.poll() is not None:
            raise RuntimeError("Application exited during screenshot capture")
        time.sleep(0.1)
    raise TimeoutError(title)


def picture(destination, color):
    svg = f'''<svg xmlns="http://www.w3.org/2000/svg" width="960" height="600" viewBox="0 0 960 600">
      <defs><linearGradient id="sky" x2="0" y2="1"><stop stop-color="#83b8db"/><stop offset="1" stop-color="#e0eff1"/></linearGradient></defs>
      <rect width="960" height="600" fill="url(#sky)"/>
      <circle cx="755" cy="115" r="46" fill="#fff2c8"/>
      <path d="M0 400L220 125 420 370 610 70 960 410V600H0" fill="#819fae"/>
      <path d="M220 125l-64 79 65-25 47 21zm390-55l-63 99 62-32 68 36z" fill="#f6fafb"/>
      <path d="M0 445L220 320 430 430 680 285 960 440V600H0" fill="{color}"/>
      <path d="M0 505Q230 430 480 505T960 490V600H0" fill="#3f8e9d"/>
      <path d="M0 555Q210 520 380 570L435 600H0M960 520L815 560 725 600H960" fill="#234c50"/>
    </svg>'''
    handle = Rsvg.Handle.new_from_data(svg.encode())
    surface = cairo.ImageSurface(cairo.FORMAT_ARGB32, 960, 600)
    rect = Rsvg.Rectangle()
    rect.x, rect.y, rect.width, rect.height = 0, 0, 960, 600
    handle.render_document(cairo.Context(surface), rect)
    surface.write_to_png(str(destination))


def fixture(root):
    files = root / "Files"
    files.mkdir()
    payloads = {
        "Documents/Field-notes.txt": "FIELD NOTES\n\nMountain lakes · October 2026\n\nThe north trail follows the river through\nthe forest before opening onto the lake.\n\nReturn route: the western ridge.\nDistance: 12.4 km\nElevation gain: 680 m\n\nPack water, a paper map and a spare battery.\n",
        "Documents/Project-plan.txt": "PROJECT PLAN\n\nOrganize the archive by date and subject.\nKeep original photographs with their notes.\nVerify copies before removing old backups.\n",
        "Documents/Travel-itinerary.txt": "TRAVEL ITINERARY\n\nMonday: arrive in the valley.\nTuesday: follow the north trail to the lake.\nWednesday: visit the ridge before sunrise.\n",
        "Documents/Expenses.csv": "date,item,amount\n2026-10-18,Train,42.50\n2026-10-19,Accommodation,85.00\n2026-10-20,Supplies,18.40\n",
        "Reports/Inventory.csv": "folder,files,status\nDocuments,4,archived\nPictures,3,archived\nReports,3,archived\n",
        "Reports/Backup-checklist.txt": "BACKUP CHECKLIST\n\nCheck the archive inventory.\nVerify file hashes.\nKeep a second copy on another drive.\n",
        "Reports/Archive-index.txt": "ARCHIVE INDEX\n\nDocuments: field notes, plans and travel records.\nPictures: mountain landscapes.\nReports: inventory and backup records.\n",
    }
    for name, text in payloads.items():
        (files / name).parent.mkdir(parents=True, exist_ok=True)
        (files / name).write_text(text)
    (files / "Pictures").mkdir()
    for name, color in [("Mountain-lake.png", "#3f756b"), ("Forest-ridge.png", "#5e7956"), ("Evening-valley.png", "#64718e")]:
        picture(files / "Pictures" / name, color)
    source = root / "Archive.img"
    with source.open("xb") as stream:
        stream.truncate(512 * 1024 * 1024)
    run("mkfs.ext2", "-q", "-F", "-L", "Archive", source)
    commands = ["mkdir Documents", "mkdir Pictures", "mkdir Reports"]
    originals = sorted(p for p in files.rglob("*") if p.is_file())
    commands += [f"write {p} {p.relative_to(files)}" for p in originals]
    commands += [f"rm {p.relative_to(files)}" for p in originals]
    command_file = root / "filesystem.commands"
    command_file.write_text("\n".join(commands) + "\n")
    run("debugfs", "-w", "-f", command_file, source, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    source.chmod(0o400)
    return source


def capture(binary, output, root, env, dark=False):
    logfile = (root / ("dark.log" if dark else "capture.log")).open("w")
    process = subprocess.Popen([str(binary)], env=env, stdout=logfile, stderr=subprocess.STDOUT)
    window = None
    try:
        window = find_window(process, "^Open Data Recovery$")
        xdo("windowsize", window, 1380, 920)
        time.sleep(1.2)
        # The saved scan was created by the real CLI in this isolated XDG workspace.
        click(window, 115, 743)
        time.sleep(1.2)

        def shot(name, target=window):
            xdo("mousemove", 1570, 1070)
            time.sleep(0.35)
            run("import", "-window", target, output / name)
            print("Captured", name, flush=True)

        def dialog(shortcut, title):
            time.sleep(0.3)
            key(window, shortcut)
            result = find_window(process, "^" + re.escape(title) + "$")
            time.sleep(0.5)
            return result

        if dark:
            custom = dialog("ctrl+shift+r", "Custom scan")
            shot("custom-scan-dark.png", custom)
            key(custom, "Escape")
            return

        click(window, 490, 434)
        time.sleep(0.5)
        shot("workspace.png")
        xdo("windowsize", window, 1060, 700)
        key(window, "ctrl+l")
        xdo("mousemove", "--window", window, 150, 350, "click", "--repeat", 3, "--delay", 100, 5)
        shot("compact.png")
        xdo("windowsize", window, 1380, 920)
        key(window, "ctrl+l")
        key(window, "ctrl+f")
        xdo("type", "--clearmodifiers", "Mountain-lake")
        time.sleep(0.6)
        click(window, 480, 405)
        time.sleep(0.7)
        shot("image-preview.png")
        key(window, "ctrl+f")
        key(window, "ctrl+a")
        key(window, "BackSpace")
        time.sleep(0.4)
        click(window, 325, 334)
        time.sleep(0.4)
        click(window, 1275, 850)
        recovery = find_window(process, "^Review recovery$")
        click(recovery, 195, 165)
        key(recovery, "ctrl+a")
        xdo("type", "--clearmodifiers", str(root / "Recovered"))
        time.sleep(0.8)
        shot("recovery.png", recovery)
        key(recovery, "Escape")
        custom = dialog("ctrl+shift+r", "Custom scan")
        shot("custom-scan.png", custom)
        key(custom, "Escape")
        saved = dialog("ctrl+s", "Saved scans")
        click(saved, 150, 210)
        time.sleep(0.4)
        shot("saved-scans.png", saved)
        key(saved, "Escape")
        # Start a genuine Full scan and pause it while its engine runs.
        click(window, 373, 154)
        key(window, "End")
        key(window, "Return")
        key(window, "ctrl+r")
        # Wait for actual carving artifacts, after the metadata findings exist.
        workspace = root / "Library/open-data-recovery/snapshots"
        deadline = time.monotonic() + 15
        while not list(workspace.glob("*/artifacts/photorec-*")):
            if time.monotonic() > deadline:
                raise TimeoutError("PhotoRec did not start")
            time.sleep(0.03)
        click(window, 700, 154)
        time.sleep(0.3)
        shot("scan-progress.png")
    finally:
        if process.poll() is None and window:
            key(window, "ctrl+q")
        try:
            process.wait(timeout=15)
        except subprocess.TimeoutExpired:
            process.terminate()
            process.wait(timeout=5)
        logfile.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, default=Path("docs/screenshots"))
    args = parser.parse_args()
    binary = args.binary.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="odr-capture-", dir="/tmp") as temp:
        root = Path(temp)
        source = fixture(root)
        (root / "Recovered").mkdir()
        env = dict(os.environ, XDG_DATA_HOME=str(root / "Library"), XDG_CONFIG_HOME=str(root / "Settings"), XDG_CACHE_HOME=str(root / "Cache"), GTK_THEME="Adwaita", GTK_A11Y="none")
        workspace = root / "Library/open-data-recovery/snapshots"
        before = hashlib.file_digest(source.open("rb"), "sha256").hexdigest()
        run(binary, "--scan", source, "--workspace", workspace, "--mode", "quick", "--metadata", "tsk", "--extensions", "txt,png,csv", env=env, stdout=subprocess.DEVNULL)
        capture(binary, output, root, env)
        capture(binary, output, root, dict(env, GTK_THEME="Adwaita:dark"), dark=True)
        assert hashlib.file_digest(source.open("rb"), "sha256").hexdigest() == before
        print("Production screenshots captured; source image hash unchanged.")


if __name__ == "__main__":
    main()
