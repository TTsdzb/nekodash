#!/usr/bin/env python3
"""Package the release executable on its native GitHub runner."""

import argparse
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import tarfile
import tempfile
import zipfile


TARGETS = {
    "x86_64-unknown-linux-gnu": "linux-x86_64",
    "x86_64-pc-windows-msvc": "windows-x86_64",
    "aarch64-apple-darwin": "macos-aarch64",
    "x86_64-apple-darwin": "macos-x86_64",
}
ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("target", choices=TARGETS)
    args = parser.parse_args()
    platform = TARGETS[args.target]
    metadata = json.loads(subprocess.check_output([
        "cargo", f"+{os.environ.get('RUST_TOOLCHAIN', '1.98.1')}", "metadata",
        "--no-deps", "--format-version", "1", "--locked",
    ], cwd=ROOT))
    version = next(package["version"] for package in metadata["packages"]
                   if package["name"] == "nekodash")
    binary_name = "nekodash.exe" if platform.startswith("windows") else "nekodash"
    binary = ROOT / "target" / args.target / "release" / binary_name
    dist = ROOT / "dist"
    dist.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="nekodash-package-") as temporary:
        staging = Path(temporary)
        if platform.startswith("macos"):
            bundle = staging / "NekoDash.app"
            contents = bundle / "Contents"
            executable_dir = contents / "MacOS"
            resources = contents / "Resources"
            executable_dir.mkdir(parents=True)
            resources.mkdir()
            shutil.copy2(binary, executable_dir / "nekodash")
            shutil.copy2(ROOT / "LICENSE", resources / "LICENSE")
            shutil.copy2(ROOT / "assets/i18n/LICENSE", resources / "LICENSE-MetaCubeXD")
            with (contents / "Info.plist").open("wb") as output:
                plistlib.dump({
                    "CFBundleName": "NekoDash",
                    "CFBundleDisplayName": "NekoDash",
                    "CFBundleIdentifier": "io.github.nekodash.panel",
                    "CFBundleExecutable": "nekodash",
                    "CFBundlePackageType": "APPL",
                    "CFBundleInfoDictionaryVersion": "6.0",
                    "CFBundleShortVersionString": version.split("-")[0].split("+")[0],
                    "CFBundleVersion": version.split("-")[0].split("+")[0],
                    "LSMinimumSystemVersion": os.environ.get("MACOSX_DEPLOYMENT_TARGET", "13.0"),
                    "NSHighResolutionCapable": True,
                    "NSLocalNetworkUsageDescription": "Connect to Mihomo controllers on your local network.",
                }, output)
            subprocess.run(["plutil", "-lint", str(contents / "Info.plist")], check=True)
            subprocess.run(["codesign", "--force", "--sign", "-", str(bundle)], check=True)
            subprocess.run(["codesign", "--verify", "--deep", "--strict", "--verbose=2", str(bundle)], check=True)
            subprocess.run(["ditto", "-c", "-k", "--sequesterRsrc", "--keepParent",
                            str(bundle), str(dist / f"NekoDash-{platform}.zip")], check=True)
        else:
            package = staging / "NekoDash"
            package.mkdir()
            shutil.copy2(binary, package / binary_name)
            shutil.copy2(ROOT / "LICENSE", package / "LICENSE")
            shutil.copy2(ROOT / "assets/i18n/LICENSE", package / "LICENSE-MetaCubeXD")
            if platform.startswith("linux"):
                with tarfile.open(dist / f"NekoDash-{platform}.tar.gz", "w:gz", compresslevel=9) as archive:
                    archive.add(package, arcname="NekoDash")
            else:
                with zipfile.ZipFile(dist / f"NekoDash-{platform}.zip", "w", zipfile.ZIP_DEFLATED,
                                     compresslevel=9) as archive:
                    for path in sorted(package.iterdir()):
                        archive.write(path, arcname=f"NekoDash/{path.name}")


if __name__ == "__main__":
    main()
