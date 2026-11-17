# Prism inventory

Observed 2026-10-30 at https://prism.openai.com/ in a hidden browser tab. This inventory covers the unauthenticated project screen only. It does not establish the behavior of a signed-in account or editor. The accessibility snapshot timed out twice; observations come from rendered text and control attributes. No project was created or modified.

## Observed project workflow and Refract mapping

| Prism observation | Refract implementation |
| --- | --- |
| Your Projects navigation | The home sidebar and brand button return to the project list. |
| Search and list/grid controls | The project list offers search and list/grid views. |
| Import, New, and Start from Scratch | The project list offers ZIP import and project creation, including an empty-state action. |
| Name and Created column controls | The list displays name and creation date and offers name and date sorting, plus recently edited sorting. |
| Sign-in controls | Refract stores projects locally and does not require a Prism account for project management. Assistant CLIs are configured separately and may require authentication and network access. |

This mapping informed the local project-list workflow; it is a code-level comparison, not a visual, interaction, or feature-parity assessment. The observed controls alone do not establish Prism's import format, sorting behavior, persistence, or account behavior.

## Local design choices

Refract is a Windows application with local project storage, editing, and compilation. Its project list includes recoverable deletion: deleted projects remain in local application data for 30 days and can be restored. ZIP export contains project files and folders, while duplication also copies comments and version history. These are Refract behaviors, not claims about Prism. See the [README](../README.md) for storage, backup, import/export, and assistant details.

Signed-in project workflows, the editor, menus, shortcuts, settings, and Prism export compatibility were not observed. They cannot be compared from this inventory; their separate verification remains tracked in [TODO.md](../TODO.md).
