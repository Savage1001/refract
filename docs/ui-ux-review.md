# Refract UI and UX review

The review started from `b42f6de` on `main` and followed the project lifecycle from opening or importing a ZIP through editing, building, collaborating, recovering work, and updating the app. The scores record the original independent GPT‑6.2 Sol review. A 10 means the reviewer found no remaining actionable issue within the tested renderer scenarios. The rebased branch uses the model picker and provider setup from `main`.

| Aspect | Baseline | First revision | Second revision | Third revision | Fourth revision | Final | Changes and verification |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| Project library and onboarding | 6 | 8 | 9 | 10 | 10 | 10 | Labeled ZIP import, persistent list or grid view, and focus on entry and return. Repeated New project activation creates one project. Delayed creation respects newer navigation. |
| Workspace navigation and responsive layout | 5 | 8 | 8 | 10 | 10 | 10 | Compact PDF switches preserve writing panes across window resizing. Sidebar and editor width limits leave space for preview. Desktop pane buttons have visible names and selected cues. |
| File tree, project search and outline | 5 | 6 | 9 | 10 | 10 | 10 | File and folder actions are available by visible controls, context menu, and keyboard. Search and outline reveal their destination in compact layouts. Search announces result counts. |
| Editor and find | 7 | 8 | 9 | 9 | 10 | 10 | File-tab activation preserves focus. The empty editor offers a next action and disables unavailable commands. Find can bring the editor into view. |
| PDF, build and export | 5 | 8 | 9 | 10 | 10 | 10 | The last successful preview remains visible through a rebuild or build failure. Sync and download are enabled for the current build. File-line diagnostics link to project source. PDF text is exposed to the accessibility tree. Export ZIP is available in the workspace header. |
| Assistant and chats | 5 | 7 | 8 | 10 | 10 | 10 | Stop returns focus to the composer. If preparation is stopped after a new draft is entered, the composer recovers both texts. Reply arrival is announced. |
| Project tools and logs | 5 | 8 | 10 | 10 | 10 | 10 | Project Info can reveal the root file. Build output preserves filters, wrapping, disclosure, and focus across builds and tool changes. File-line diagnostics can open source from Logs. |
| Comments and versions | 6 | 7 | 9 | 9 | 10 | 10 | Comment links reveal source in compact layouts and report unavailable selections. Version restore availability follows the open file. Closing the comparison returns the previous tool layout. |
| Dialogs and recovery | 5 | 7 | 9 | 9 | 9 | 10 | Rejected file names leave the rename dialog open. Folder moves reveal the destination and honor newer file navigation even when the tree refresh is delayed. Deleted files and project recovery announce successful restores. |
| Settings and updates | 6 | 8 | 8 | 9 | 10 | 10 | Status polling preserves operation errors. Keyboard focus returns to the relevant control after an update operation fails. Progress, update tracks, and ready-to-restart have fixture coverage. |
| Visual consistency and accessibility | 6 | 7 | 9 | 10 | 10 | 10 | Reviewed hierarchy, contrast, focus, labels, and control geometry in light and dark themes. Captures include a narrow library, minimum-width sidebar, and compact authoring and PDF. |

## Reproduce the captures

Run `npm run build`, set `REFRACT_THEME_ARTIFACTS` to an output directory, and run `node build/scripts/capture-ui.cjs`. The script uses headless Chromium, isolated IPC data, and a sample PDF. It checks 19 matching light and dark state pairs. The targeted tests exercise keyboard navigation, loading and errors, compact width, and asynchronous focus.

The original background verification passed the TypeScript build, 31 renderer and geometry checks, 18 model picker checks, 78 project, archive, comment, and version checks, and three compiler process checks. A version recovery check passed on retry. The archive test that requires installed TeX skipped.

Native ZIP and save pickers, native title-bar interaction, installed TeX and SyncTeX, authenticated CLI providers, updater and installer execution, and Windows screen reader output were outside the headless evidence.
