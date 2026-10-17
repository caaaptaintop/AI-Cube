# Pleat

<img src="./assets/app-icon.png" width="128" height="128" alt="Pleat">

Pleat is a skill management app based on [Kitter](https://github.com/what1f/kitter), maintained in `caaaptaintop/AI-Cube`. The desktop display name and icon are Pleat; the `kitter` CLI, application identity and existing data paths remain compatible.

See [brand assets and reproducible exports](assets/brand/README.md). Build the local macOS candidate with `just package`; this brand candidate has not been publicly released. The upstream Kitter documentation and download links below are retained for reference.

## Upstream Kitter documentation

<p align="center">
  <img src="./assets/readme/hero.en.png" width="100%" alt="Kitter — one skill library where every project gets only what it needs">
</p>

<p align="center">
  <a href="./README.zh-CN.md">简体中文</a>
</p>

<p align="center">
  <a href="./LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-3f8997" alt="Apache-2.0 license"></a>
  <img src="https://img.shields.io/badge/desktop-macOS%20%7C%20Windows%20%7C%20Linux-15191a" alt="macOS, Windows, and Linux desktop app">
  <img src="https://img.shields.io/badge/built_with-Rust-b8aaa0" alt="Built with Rust">
</p>

<p align="center"><strong>One skill library. Every project gets only what it needs.</strong></p>

Kitter is a desktop app and CLI for managing Agent Skills across projects. Keep your skills in one library, install the right combination for each project, and update them in one place.

Built entirely in Rust with GPUI, Kitter pairs a straightforward interface with a small footprint and smooth native performance.

<p align="center">
  <img src="./assets/readme/skill-workflow.png" width="100%" alt="Kitter maintains one skill library and links selected skills to projects and user-level installations">
</p>

## Why Kitter

Working across projects often means maintaining several copies of the same skill and keeping track of what each agent can use. Kitter keeps those connections visible:

- **Maintain once** — projects link to the same skill source, so one update reaches every linked installation.
- **Choose per project** — give each project its own skill set, with user-level installation for skills you use everywhere.
- **See what is active** — inspect the skills each agent discovers, including installations outside Kitter, along with their sources and estimated context cost.

## Install Kitter

Download the app for your platform from [GitHub Releases](https://github.com/what1f/kitter/releases/latest).

- **macOS (Apple Silicon / Intel)** — choose the `macos-arm64.dmg` (Apple Silicon) or `macos-x86_64.dmg` (Intel) download, open the `.dmg` and drag `Kitter.app` into `Applications`.
- **Windows (x64)** — download `Kitter-<version>-desktop-windows-x86_64.exe` and run it directly.
- **Linux (x64)** — extract `Kitter-<version>-desktop-linux-x86_64.tar.gz` and run `./Kitter` from the extracted `Kitter` directory.

Kitter is not yet signed with an Apple Developer ID. If macOS blocks the first launch, confirm that you downloaded it from the official release, then go to **System Settings → Privacy & Security → Open Anyway** and follow the prompts. See [Apple’s instructions](https://support.apple.com/102445).

You can also run the following command, then open Kitter again:

```bash
xattr -dr com.apple.quarantine /Applications/Kitter.app
```

The desktop app and CLI are separate release artifacts built on the same core. Standalone CLI packages for macOS, Windows, and Linux are available from [GitHub Releases](https://github.com/what1f/kitter/releases/latest). The built-in Kitter skill resolves that standalone CLI and guides you through downloading it when needed.

## Manage your skills with Kitter

### 1. Build one library

Use **+** to add skills from a local folder, GitHub or a skills.sh-compatible source, or a Claude plugin source. If skills are already scattered across projects, choose **Existing installations** to inspect and adopt them without moving their source directories.

Kitter keeps one maintained source for each skill. Open its **Installs** tab to immediately see every project using it, every installation location, and the agents that can discover it.

<p align="center">
  <img src="./assets/readme/skill-library.en.png" width="100%" alt="Kitter skill library showing one managed skill installed across several projects">
</p>

### 2. Install only where needed

Select a skill, choose a project, then install it into the shared `.agents/skills` directory or an agent-specific directory. Kitter creates managed links instead of independent copies, so projects can use different combinations without creating update drift.

DimAgent uses the shared install option. For Oh My Pi (omp), choose the shared option or its own `.omp/skills` target (`--target omp` in the CLI). User-level omp installs use `~/.omp/agent/skills` by default.

<p align="center">
  <img src="./assets/readme/install-skill.en.png" width="100%" alt="Kitter installation dialog for selecting a project and agent targets">
</p>

Skills you use across all projects can also be installed at the user level.

### 3. Verify what is actually active

Open **Projects** to see the complete effective skill set for every agent—not just installations managed by Kitter. The view discovers project, parent, user-level, built-in, and plugin-provided capabilities, then shows where each one came from.

The per-agent token estimate helps you spot skills that add unnecessary context overhead.

<p align="center">
  <img src="./assets/readme/project-effective-skills.en.png" width="100%" alt="Kitter project view showing managed and unmanaged effective skills, plugins, agents, and estimated context cost">
</p>

### 4. Update once

Run **Check for updates** from the desktop app or use `kitter check` and `kitter update`. Every managed project continues to use the same maintained source.

The equivalent CLI workflow is intentionally small:

```bash
kitter add npx https://github.com/owner/repository --skill skill-a
kitter install skill-a --project /path/to/project --target universal
kitter project /path/to/project
kitter update skill-a
```

## Standalone CLI and agent skill

You do not need the desktop app to use Kitter. Download the standalone CLI from [GitHub Releases](https://github.com/what1f/kitter/releases/latest), put `kitter` on your `PATH`, and install the [`$kitter` skill](./resources/skills/kitter) directly:

```bash
npx skills add what1f/kitter --skill kitter
```

The skill lets an agent inspect the current machine, add or adopt skill sources, install the right project combination, and verify the result through the standalone `kitter` CLI. If the CLI is missing, the skill can guide you through downloading it from an official Release.

<details>
<summary><strong>Build from source</strong></summary>

```bash
git clone https://github.com/what1f/kitter.git
cd kitter
cargo run --release --locked --features desktop --bin kitter-desktop
```

</details>

## Platform status

- **macOS (Apple Silicon / Intel)** — desktop application and standalone CLI.
- **Windows (x64)** — desktop application and standalone CLI, tested on Windows with platform-specific startup and performance fixes.
- **Linux (x64)** — standalone CLI and desktop build available; the desktop app still needs validation on real systems.

## Local data

Pleat stores configuration and source records in the operating system's application-data directory. New installations use these default skill libraries:

| Platform | Default skill library |
| --- | --- |
| macOS | `~/Library/Application Support/Pleat/skills` |
| Windows | `%LOCALAPPDATA%\Pleat\skills` |
| Linux | `$XDG_DATA_HOME/Pleat/skills` or `~/.local/share/Pleat/skills` |

Existing Kitter data stays in place until an explicit migration. In Settings → Advanced storage settings, preview moving the library, using an existing library, or moving the Kitter data root to its sibling Pleat folder. Moves retain an old-path compatibility link for installed skills and local runtimes; cross-volume moves and occupied targets are refused. An interrupted metadata transaction blocks reopening until its saved recovery material is resolved.

`kitter library` shows the current library; `kitter library --set /absolute/path` switches to an existing nonempty library using the same transaction checks. To move into an empty folder, use the desktop's “Move current library” action. `KITTER_HOME` remains an explicit application-data override.

## Contributing

Issues and pull requests are welcome. Please open an [issue](https://github.com/what1f/kitter/issues) before starting a large behavioral or UI change so the scope can be aligned first.

If Kitter makes your skill setup calmer, consider [starring the repository](https://github.com/what1f/kitter). It helps more multi-project developers find it.

## License

Kitter is available under the [Apache License 2.0](./LICENSE). Licenses for bundled fonts, icons, and other third-party material are listed in [THIRD_PARTY_LICENSES.md](./THIRD_PARTY_LICENSES.md).

## Star History

<a href="https://www.star-history.com/?repos=what1f%2Fkitter&type=date">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=what1f/kitter&type=date&theme=dark" />
    <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=what1f/kitter&type=date" />
    <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=what1f/kitter&type=date" />
  </picture>
</a>
