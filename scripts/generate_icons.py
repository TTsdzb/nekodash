#!/usr/bin/env python3
"""Derive platform icon containers from logo.png using ImageMagick."""
from pathlib import Path
import struct
import subprocess

ROOT = Path(__file__).resolve().parent.parent
icons = ROOT / "assets/icons"
icons.mkdir(parents=True, exist_ok=True)
for size in (16, 32, 48, 64, 128, 256, 512, 1024):
    subprocess.run(["magick", str(ROOT / "logo.png"), "-resize", f"{size}x{size}",
                    "-strip", str(icons / f"app-{size}.png")], check=True)
subprocess.run(["magick", str(ROOT / "logo.png"), "-define", "icon:auto-resize=256,128,64,48,32,16",
                str(icons / "app.ico")], check=True)
chunks = []
for tag, size in ((b"icp4",16),(b"icp5",32),(b"icp6",64),(b"ic07",128),
                  (b"ic08",256),(b"ic09",512),(b"ic10",1024)):
    data = (icons / f"app-{size}.png").read_bytes()
    chunks.append(tag + struct.pack(">I", len(data) + 8) + data)
data = b"".join(chunks)
(icons / "app.icns").write_bytes(b"icns" + struct.pack(">I", len(data) + 8) + data)
resources = ROOT / "assets/android/res"
for density, size in (("mdpi",48),("hdpi",72),("xhdpi",96),("xxhdpi",144),("xxxhdpi",192)):
    folder = resources / f"mipmap-{density}"
    folder.mkdir(parents=True, exist_ok=True)
    subprocess.run(["magick", str(ROOT / "logo.png"), "-resize", f"{size}x{size}",
                    "-strip", str(folder / "ic_launcher.png")], check=True)
