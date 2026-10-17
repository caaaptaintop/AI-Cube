#!/usr/bin/env bash

set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "Usage: $0 <Pleat.app> <background.png> <output.dmg>" >&2
  exit 2
fi

app_path="$1"
background_path="$2"
output_path="$3"
script_dir="$(cd "$(dirname "$0")" && pwd)"
settings_path="${script_dir}/dmg-settings.py"

test -d "$app_path"
test -f "$background_path"
test -f "$settings_path"

rm -f "$output_path"
uvx --from "dmgbuild==1.6.7" dmgbuild \
  -s "$settings_path" \
  -D "app=${app_path}" \
  -D "background=${background_path}" \
  "Pleat Installer" \
  "$output_path"

hdiutil verify "$output_path"

mount_root="$(mktemp -d "${TMPDIR:-/tmp}/kitter-dmg.XXXXXX")"
cleanup() {
  hdiutil detach "$mount_root" >/dev/null 2>&1 || true
  rmdir "$mount_root" >/dev/null 2>&1 || true
}
trap cleanup EXIT

hdiutil attach -readonly -nobrowse -mountpoint "$mount_root" "$output_path" >/dev/null
codesign --verify --deep --strict --verbose=2 "$mount_root/Pleat.app"
uv run --no-project --with "dmgbuild==1.6.7" python - "$mount_root" "$settings_path" <<'PYTHON'
import pathlib
import runpy
import sys
from ds_store import DSStore

root = pathlib.Path(sys.argv[1])
settings = runpy.run_path(sys.argv[2], init_globals={"defines": {"app": "", "background": ""}})
with DSStore.open(str(root / ".DS_Store"), "r") as store:
    for name, expected in settings["icon_locations"].items():
        if (root / name).exists():
            actual = store[name]["Iloc"]
            if tuple(actual) != expected:
                raise SystemExit(f"Unexpected installer icon position for {name}: {actual}")
print("Installer icon positions verified")
PYTHON
hdiutil detach "$mount_root" >/dev/null
rmdir "$mount_root"
trap - EXIT
