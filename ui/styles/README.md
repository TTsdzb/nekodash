# Slint widget styles

Source: Slint 1.18.1, `i-slint-compiler/widgets` (crate release pinned in Cargo.toml).
Copyright notices and upstream licenses are retained in each source file and `LICENSES`.

`common`, `nekodash-fluent` and `nekodash-material` preserve upstream widgets.
The two `styling.slint` palettes import `ui/monet.slint` and map color roles to the
provided Material Tonal Spot scheme. Disabling `Monet.enabled` restores upstream colors.
Slint 1.18.1 exposes these palette brushes as read-only; copying the widget style
lets controls use the supplied exact colors, including neutral surfaces.

Regenerate with `python3 scripts/vendor_widget_styles.py /path/to/i-slint-compiler-1.18.1`.

The copied widgets use Slint's internal interfaces and menu components. The
project's `.cargo/config.toml` enables `SLINT_ENABLE_EXPERIMENTAL_FEATURES` so the
1.18.1 compiler accepts these upstream definitions from the local include path.
Review this integration together with the widgets when upgrading Slint.
