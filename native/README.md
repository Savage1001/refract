# Refract native preview

This opt-in Rust and GPUI app opens the existing Refract project library in list or grid view. Create a project with **New project**, or select its name in the workspace to rename it. The workspace opens the root LaTeX document first and lets you browse nested folders and edit UTF-8 text files up to 32 MiB in separate tabs. Edits save automatically and update existing comment anchors. Use Save or Ctrl+S (Cmd+S on macOS) to retry a failed save. Closing a tab or leaving the workspace waits for pending saves and asks before discarding edits that could not be saved.

Use **Duplicate** in a project’s library menu to open a copy with its files, comments, version history, and LaTeX settings. **Delete** moves the project to **Deleted projects**, where it can be restored for up to 30 days.

Use **+** in Files to create a file at the project root or in a selected folder. Open **More file options** to create a folder. A folder’s **Add items** menu preselects that destination.

Right-click a file or folder or focus its row and press `Shift+F10` to open its actions. Open tabs, the root document, and comment references follow a renamed or moved path. Use **Deleted files** in the sidebar to restore files and folders to their original paths.

Use **Search files** above the project tree to search saved files. Select a result to open the file and select the matching text. Press Escape in the search field to clear it.

Use **Find** or Ctrl+F (Cmd+F on macOS) to find and replace text in the active file. Enter and Shift+Enter move between matches. Match case, Whole word, and Wrap control which matches are selected. Escape closes the bar and returns focus to the source.

Use **Commands** or Ctrl+Space (Cmd+Space on macOS) for LaTeX command suggestions. Use the arrow keys to choose a command and Enter or Tab to insert it.

**Outline** follows the active file as you edit LaTeX section headings. Select a heading to jump to the source. Drag the divider above Outline or focus it and use the arrow keys to resize the list.

Inline math in outline titles uses KaTeX fonts under the [SIL Open Font License](licenses/KaTeX-OFL.txt).

Use **Download file** to save a copy of a project file or **Export project ZIP** to save the visible project files and folders, including empty folders.

Use **More file options** in the New file dialog to upload files or a folder into the selected project folder. Existing files at the destination are preserved.

Use **Import ZIP** in the project library to open a LaTeX project archive. Select a root document when the archive has multiple candidates. **Import all** creates a separate project for each root in its own top-level folder.

Open Comments to search discussions, add a file comment or comment on selected source, write replies, and resolve or reopen threads. Selecting a reference opens the file and moves the caret to the quoted source when the anchor can be located.

Versions lists existing and newly captured snapshots. Refract captures a snapshot when you open a project and after saved edits settle, using the same manifest and ZIP format as the Electron app. Compare with current shows file, folder, and root document changes. Open a changed file to see its previous and current lines side by side. Restore file replaces the active file. Restore project restores files, folders, and the root document. Both save the current project as a snapshot first. An interrupted project restore recovers the previous version on startup.

When opening a project, the preview retries cleanup of temporary restore data recorded by Electron. It shows a notice if a stage still contains data that could not be verified or removed.

From the repository root, use stable Rust and the Windows MSVC toolchain to build the preview. Running the binary opens a desktop window.

```sh
cargo build --locked --manifest-path native/Cargo.toml
cargo run --locked --manifest-path native/Cargo.toml
```

The preview looks in the platform's application configuration directory for the `Refract/projects` folder. For a custom profile or a source checkout using a different profile directory, set `REFRACT_DATA_DIR` to the folder containing `projects`.

The workspace builds LaTeX from saved project files using the installed TeX toolchain. Use **Compile** to rebuild and **Download PDF** to save the result. On Windows, the preview includes page navigation, zoom, Fit width, and Fit page. Use **Show in PDF** to jump from the current source line. Double-click a PDF page or focus it and press Enter to jump back to the source.

Open **Tools** for **Project Info**, **Versions**, **Comments**, and **Logs**. Use **Expand panel** for more room or **Tools** to return to the PDF. In Project Info, select the root file to open it, choose the LaTeX engine, or turn on draft mode to compile with image placeholders.

Logs shows build errors and warnings and links file diagnostics to the reported source line. Open **Full build output** to filter the compiler messages or change line wrapping.

Use the Electron app for inline PDF rendering and SyncTeX navigation on macOS and Linux, assistants, and updates.

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
