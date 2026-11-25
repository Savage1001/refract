# Refract native preview

This opt-in Rust and GPUI app opens the existing Refract project library and displays a project's root LaTeX source. It reads the existing project metadata and UTF-8 source files up to 128 KiB. Tab and Shift+Tab move through the library, and Enter opens the focused project.

From the repository root, use stable Rust and the Windows MSVC toolchain to build the preview. Running the binary opens a desktop window.

```sh
cargo build --locked --manifest-path native/Cargo.toml
cargo run --locked --manifest-path native/Cargo.toml
```

The preview looks in the platform's application configuration directory for the `Refract/projects` folder. For a custom profile or a source checkout using a different profile directory, set `REFRACT_DATA_DIR` to the folder containing `projects`.

Use the Electron app for editing, PDF preview, TeX and SyncTeX, assistants, and updates.

Run the project-library tests without building GPUI or opening a window.

```sh
cargo test --locked --no-default-features --manifest-path native/Cargo.toml
```
