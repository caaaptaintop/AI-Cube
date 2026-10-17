set shell := ["zsh", "-cu"]

root := justfile_directory()
release_dir := root + "/target/release"
app_dir := release_dir + "/Pleat.app"
dmg_background := root + "/assets/dmg/background.png"
version := `sed -nE 's/^version = "([^"]+)"/\1/p' Cargo.toml | head -n 1`
architecture := `uname -m`
dmg := release_dir + "/Pleat-" + version + "-macos-" + architecture + ".dmg"

default:
    @just --list

check:
    cargo test --locked

build:
    cargo build --release --locked --features desktop --bin kitter-desktop --bin kitter

# Regenerate all production icons from the editable Pleat vector master.
brand-icons:
    python3 "{{root}}/scripts/export-brand-assets.py"

macos-icon:
    "{{root}}/scripts/export-macos-icon.sh"

app: build
    rm -rf "{{app_dir}}"
    mkdir -p "{{app_dir}}/Contents/MacOS" "{{app_dir}}/Contents/Resources"
    cp "{{root}}/assets/macos/app-icon.icns" "{{app_dir}}/Contents/Resources/logo.icns"
    cp "{{release_dir}}/kitter-desktop" "{{app_dir}}/Contents/MacOS/Kitter"
    cp "{{root}}/resources/Info.plist" "{{app_dir}}/Contents/Info.plist"
    cp "{{root}}/LICENSE" "{{root}}/THIRD_PARTY_LICENSES.md" "{{app_dir}}/Contents/Resources/"
    cp "{{root}}/assets/brand/README.md" "{{app_dir}}/Contents/Resources/BRANDING.md"
    plutil -replace CFBundleShortVersionString -string "{{version}}" "{{app_dir}}/Contents/Info.plist"
    plutil -replace CFBundleVersion -string "{{version}}" "{{app_dir}}/Contents/Info.plist"
    chmod +x "{{app_dir}}/Contents/MacOS/Kitter"
    xattr -cr "{{app_dir}}"
    codesign --deep --force --sign - "{{app_dir}}"
    codesign --verify --deep --strict "{{app_dir}}"
    @echo "App ready: {{app_dir}}"

package: app
    "{{root}}/scripts/package-macos-dmg.sh" "{{app_dir}}" "{{dmg_background}}" "{{dmg}}"
    @file "{{release_dir}}/kitter" "{{release_dir}}/kitter-desktop"
    @shasum -a 256 "{{dmg}}"
    @echo "Package ready: {{dmg}}"

run: app
    open "{{app_dir}}"

release: check package
    @echo "Release ready: {{dmg}}"
