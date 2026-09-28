#!/usr/bin/env python3
"""Copy pinned Slint widgets and adapt their read-only color palettes."""
import argparse
from pathlib import Path
import re
import shutil
import tomllib

parser = argparse.ArgumentParser()
parser.add_argument("compiler_source", type=Path, help="i-slint-compiler 1.18.1 source directory")
args = parser.parse_args()
with (args.compiler_source / "Cargo.toml").open("rb") as manifest:
    if tomllib.load(manifest)["package"]["version"] != "1.18.1":
        parser.error("Expected i-slint-compiler 1.18.1")
root = Path(__file__).resolve().parent.parent
dest = root / "ui/styles"
for style in ("common", "interfaces", "fluent", "material"):
    name = style if style in ("common", "interfaces") else f"nekodash-{style}"
    shutil.copytree(args.compiler_source / "widgets" / style, dest / name, dirs_exist_ok=True)
shutil.copytree(args.compiler_source / "LICENSES", dest / "LICENSES", dirs_exist_ok=True)
for path in dest.rglob("*.slint"):
    path.write_text(path.read_text().rstrip() + "\n")

roles = {
    "background": "surface", "foreground": "on-surface",
    "alternate-background": "surface-container-low", "alternate-foreground": "on-surface",
    "control-background": "surface-container-high", "control-foreground": "on-surface",
    "accent-background": "primary", "accent-foreground": "on-primary",
    "selection-background": "primary-container", "selection-foreground": "on-primary-container",
    "border": "outline-variant", "control-background-variant": "surface-container-highest",
    "control-foreground-variant": "on-surface-variant", "accent-container": "primary-container",
    "accent-ripple": "primary", "border-variant": "outline-variant", "foreground-alt": "on-surface",
    "surface-container": "surface-container", "surface-container-high": "surface-container-high",
    "surface-container-highest": "surface-container-highest", "tertiary-container": "tertiary-container",
    "on-tertiary-container": "on-tertiary-container", "state-default": "on-surface",
    "state-secondary": "primary", "state-tertiary": "on-primary",
    "control-border": "outline-variant", "text-secondary": "on-surface-variant",
    "text-tertiary": "on-surface-variant", "control-secondary": "surface-container-high",
    "control-tertiary": "surface-container-highest", "control-solid": "surface-container-high",
    "control-input-active": "surface-container-highest", "focus-stroke-outer": "primary",
    "text-control-border": "outline", "text-accent-foreground-secondary": "on-primary",
}
for style in ("fluent", "material"):
    path = dest / f"nekodash-{style}" / "styling.slint"
    source = path.read_text()
    source = 'import { Monet } from "../../monet.slint";\n' + source
    for name, role in roles.items():
        source = re.sub(rf"(out property <brush> {name}: )([^;]+);",
                        lambda m: f"{m[1]}Monet.enabled ? Monet.{role} : ({m[2]});", source)
    path.write_text(source)
