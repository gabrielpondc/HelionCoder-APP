# HelionCoder Desktop

Local-first desktop client for HelionCoder, built with [Tauri](https://tauri.app/), Rust, and [Svelte](https://svelte.dev/).

HelionCoder Desktop wraps the HelionCoder CLI with a visual workspace organized around three workflows:

- **Chat** — plain AI conversation.
- **Cowork** — office-style file work inside a selected folder, including Word, Excel, PPT, PDF, and other document skills.
- **Code** — HelionCoder CLI sessions with project folders, diffs, tools, model/effort controls, and local usage statistics.

## Features

- HelionCoder CLI integration (`helion-coder`, with `helioncoder` fallback).
- Native folder picker for local projects.
- Chinese and English UI (i18n).
- Local settings and history stored under `~/.helioncoder`.
- SQLite usage stats: sessions, messages, tokens, models, active days, streaks, and code line changes.
- Sidebar entry points for Skills, Plugins, MCP, Hooks, Agents, and review-changes.
- Code editor powered by CodeMirror with syntax highlighting for 15+ languages.
- Integrated terminal (xterm.js).

## Platforms

| Platform | Status |
| -------- | ------ |
| macOS    | Supported (13.0+) |
| Linux    | Supported (deb) |
| Windows  | Supported |

## Installation Notes

### macOS: "App is damaged" warning

Since the app is not code-signed with an Apple Developer certificate, macOS Gatekeeper may show:

> "HelionCoder" is damaged and can't be opened. You should move it to the Trash.

This is a false positive. To fix it, run the following in Terminal:

```bash
xattr -cr /Applications/HelionCoder.app
```

Or right-click the app and select **Open** the first time — macOS will remember the exception.

## Development

### Prerequisites

- [Node.js](https://nodejs.org/) 20+
- [Rust](https://www.rust-lang.org/) 1.75+ with Cargo
- [Tauri CLI](https://tauri.app/) (installed via npm)

### Setup

```bash
git clone https://github.com/gabrielpondc/HelionCoder-APP.git
cd HelionCoder-APP
npm install
```

### Run in development

```bash
npm run tauri dev
```

### Build

```bash
npm run tauri build
```

Build output is produced under `src-tauri/target/release/bundle`.

### Build on GitHub Actions

This project includes a GitHub Actions workflow (`.github/workflows/desktop-build.yml`) that builds the desktop app for all platforms. It can be triggered manually:

1. Go to your repository on GitHub.
2. Navigate to **Actions** > **Build desktop app**.
3. Click **Run workflow** and select the branch.

The workflow builds for:

| Target            | Platform        |
| ----------------- | --------------- |
| `macos-arm64`     | macOS Apple Silicon |
| `macos-x64`       | macOS Intel     |
| `linux`           | Ubuntu 22.04    |
| `windows`         | Windows         |

Build artifacts can be downloaded from the workflow run page after completion.

### Other commands

```bash
npm run lint          # ESLint
npm run format        # Prettier
npm run check         # svelte-check
npm run test          # Vitest
npm run verify        # Full check (lint + format + test + build + rust)
npm run fix           # Auto-fix lint + format
```

## Project Structure

```
├── src/                 # Svelte frontend
├── src-tauri/           # Rust / Tauri backend
├── messages/            # i18n locale files (en.json, zh-CN.json)
├── scripts/             # Build and release scripts
├── static/              # Static assets
└── icon/                # App icons source
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines on bug reports, feature requests, code contributions, and translations.

## Security

Please report security vulnerabilities privately — see [SECURITY.md](SECURITY.md).

## License

[Apache License 2.0](LICENSE)
