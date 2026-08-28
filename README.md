# Harbor File

Harbor File is a native desktop MVP inspired by the useful parts of WinSCP: a calm two-pane file manager, saved server profiles, drag-and-drop transfers, and a friendlier search surface.

## Current MVP

- Local Mac folder browsing through the Rust desktop layer.
- SFTP password authentication and remote folder browsing.
- macOS Keychain storage for server passwords.
- Upload/download actions from drag-and-drop or the transfer rail.
- Recursive search with plain-language name, scope, kind, and date filters.
- Optional local text-content search for files up to 2 MB.
- Transfer queue surface and clear connection state.

## Build on a Mac

Install the normal Tauri prerequisites for macOS, then from this directory run:

```sh
npm install
npm run tauri dev
```

To create a distributable app:

```sh
npm run tauri build
```

The generated `.app` and `.dmg` will be under `src-tauri/target/release/bundle/`.

## Product decisions

Search is a dedicated workspace panel instead of a modal full of file-mask syntax. The common choices are visible directly and every result keeps its exact path. Remote search is recursive; content search is local-only in this first slice because reading remote file contents needs a separate transfer-aware preview policy.

Before shipping, add host-key verification, SSH key authentication, resumable transfers, conflict-resolution dialogs, richer transfer progress, and code signing/notarization.
