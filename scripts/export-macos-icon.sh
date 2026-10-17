#!/bin/bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
# The master already includes the 824px macOS footprint. Do not mask it again.
exec python3 "$root/scripts/export-brand-assets.py"
