# Project instructions

All application and test code must be written in TypeScript. Do not introduce JavaScript source files. Keep TypeScript checking enabled and add stronger types as code evolves. Keep development and verification processes in the background without opening windows unless the user explicitly requests them.

Every task in TODO.md must have a unique `RF-` ID. Allocate new IDs using and advancing the counter in TODO.md. For parallel work, reserve IDs through a serialized update to the latest main branch before assigning them; do not allocate independently from stale counters. Never reuse an allocated ID, even if its task is removed, and do not change an existing task's ID when editing, moving, or completing it.

When asked for a new release, increment the patch (third) version number by default. Increment the minor (second) or major (first) number only when the user explicitly asks for that increment. Honor an explicitly requested version. Every new release must include an installer.

Do not include private email addresses, other nonpublic personal information, credentials, or secrets in commits or pull requests, including commit author and committer metadata, messages, file contents, titles, and descriptions. Use the owner's GitHub-provided noreply address for commit metadata. Check the staged changes and author identity before committing, and check pull request content before submission.
