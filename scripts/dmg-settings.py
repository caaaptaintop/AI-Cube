app_path = defines["app"]
background_path = defines["background"]

format = "UDZO"
filesystem = "HFS+"
compression_level = 9

files = [(app_path, "Pleat.app")]
symlinks = {"Applications": "/Applications"}

background = background_path
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
window_rect = ((100, 100), (821, 479))
default_view = "icon-view"
show_icon_preview = False

arrange_by = None
grid_offset = (0, 0)
grid_spacing = 100
scroll_position = (0, 0)
label_pos = "bottom"
text_size = 14
icon_size = 128

icon_locations = {
    "Pleat.app": (250, 233),
    "Applications": (585, 233),
    # Finder can reveal hidden files. Keep support files outside the installer viewport.
    ".background.tiff": (1600, 1200),
    ".background.png": (1760, 1200),
    ".DS_Store": (1920, 1200),
    ".VolumeIcon.icns": (2080, 1200),
}

hide = [".background.tiff"]
