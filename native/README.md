# Refract native preview

This opt-in Rust and GPUI app opens the existing Refract project library in list or grid view. It opens the root LaTeX document first and lets you browse nested folders and edit UTF-8 text files up to 128 KiB in separate tabs. Edits save automatically and update existing comment anchors. Use Save or Ctrl+S (Cmd+S on macOS) to retry a failed save. Closing a tab or leaving the workspace waits for pending saves and asks before discarding edits that could not be saved.

Open Comments to read existing project comments and replies. Selecting a reference opens the file and moves the caret to the quoted source when the anchor can be located.

Versions lists existing and newly captured snapshots. Refract captures a snapshot when you open a project and after saved edits settle, using the same manifest and ZIP format as the Electron app. Compare with current shows file, folder, and root document changes. Open a changed file to see its previous and current lines side by side. Restore file replaces the active file. Restore project restores files, folders, and the root document. Both save the current project as a snapshot first. An interrupted project restore recovers the previous version on startup.

When opening a project, the preview retries cleanup of temporary restore data recorded by Electron. It shows a notice if a stage still contains data that could not be verified or removed.

From the repository root, use stable Rust and the Windows MSVC toolchain to build the preview. Running the binary opens a desktop window.

```sh
cargo build --locked --manifest-path native/Cargo.toml
cargo run --locked --manifest-path native/Cargo.toml
```

The preview looks in the platform's application configuration directory for the `Refract/projects` folder. For a custom profile or a source checkout using a different profile directory, set `REFRACT_DATA_DIR` to the folder containing `projects`.

Use the Electron app for editing, PDF preview, TeX and SyncTeX, assistants, and updates.

## Parity before replacing Electron

The current Electron app is the reference for appearance and behavior. The Native Windows preview workflow runs the existing renderer's interaction tests and uploads fixture screenshots of the library, settings, and workspace in light and dark themes, including the compact workspace layout. Run `node build/scripts/capture-electron-parity.cjs <output-folder>` after `npm run build` to capture the same reference on a machine with Microsoft Edge.

Before changing the default app or shipping this crate, verify the GPUI build against these references and run equivalent workflows on Windows, macOS, and Linux. Compare views at the same viewport, scale, and theme. Include app chrome, keyboard and pointer interaction, focus, accessibility, and responsive layouts. Verify project and file management, editing, PDF preview and SyncTeX in both directions, versions and comments, chats and settings, and installation and updates. Confirm that existing projects and settings open in place.

Run the project-library tests without building GPUI or opening a window.

```sh
cargo test --locked --no-default-features --manifest-path native/Cargo.toml
```

The Windows CI job also exercises keyboard navigation through GPUI's simulated platform, using a temporary profile.

```sh
cargo test --locked --bin refract-native --features ui-tests --manifest-path native/Cargo.toml
```
