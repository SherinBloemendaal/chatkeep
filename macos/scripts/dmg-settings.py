# dmgbuild settings for Chatkeep.dmg: the app on the left, an Applications link on the right,
# and art/dmg-background.svg (as Resources/dmg-background.tiff) behind them.
# Used by scripts/make-dmg.sh; `defines` comes from dmgbuild's -D options.

app = defines["app"]  # noqa: F821 (dmgbuild provides `defines`)

format = "ULFO"  # LZFSE compressed, readable on macOS 10.11 and later
filesystem = "APFS"
size = None

files = [app]
symlinks = {"Applications": "/Applications"}
icon = "Resources/AppIcon.icns"

background = "Resources/dmg-background.tiff"
window_rect = ((200, 140), (660, 400))
default_view = "icon-view"
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
show_icon_preview = False

icon_size = 128
text_size = 13
arrange_by = None
# Centres of the icons; they line up with the arrow in the background.
icon_locations = {
    "Chatkeep.app": (165, 180),
    "Applications": (495, 180),
}
