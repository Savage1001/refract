# Security

If you discover a security vulnerability in Refract, please [report it privately through GitHub](https://github.com/Savage1001/refract/security/advisories/new). If private vulnerability reporting is unavailable, email [nikhil.somanchi@gmail.com](mailto:nikhil.somanchi@gmail.com). This address is the maintainer's designated public security contact. Do not open a public issue with vulnerability details.

Include a description, steps to reproduce, the affected version, and any relevant proof of concept. Please avoid including private project data or credentials.

Please allow time for the issue to be investigated and fixed before disclosing it publicly. For other bugs, use the repository's issue tracker.

## Project trust and local execution

Refract does not sandbox TeX. `-no-shell-escape` disables TeX shell escape; it does **not** confine filesystem reads. pdfLaTeX can read absolute paths, and LuaLaTeX can use Lua file APIs. Engines, BibTeX/Biber, package lookup, and assistants run with the Windows account's permissions, including access outside the project and network access where supported. Output or assistant requests can disclose data they read. Do not compile or run assistants on documents from authors you do not trust.

Imported ZIPs, duplicates, and existing/unknown projects start untrusted. You may open and edit their sources without compiling or executing project instructions. Use **Review project trust**, read the warning, and explicitly select **Trust project** to enable automatic builds and assistants. Canceling leaves execution blocked. The built-in **New Project** template is created locally and starts trusted. Every project shows its trust state and a revoke action.

Trust is local application state outside the project directory, tied to that directory's filesystem identity. Project metadata, ZIP contents, assistant tools, and restored document content cannot grant it. Exports do not carry trust. Deleting/corrupting local trust records fails closed; replacing a project directory requires renewed consent. Restoring a deleted project with its original directory identity retains its local decision; a duplicate has a new identity and requires consent.

Trust covers future edits, uploads, restored versions, and project instructions within that project. Review these before adding them to a trusted project, or revoke trust first. Assistant execution is blocked for untrusted projects because provider runtimes may automatically consume imported instructions/configuration and can execute commands independently of Refract's build tools. An assistant cannot approve the human consent dialog through Refract tools. Revocation waits for active builds to finish and is unavailable during assistant operations; cancel the assistant first. It does not undo prior execution or protect against malicious software already running as your account. This gate is informed consent, not OS isolation.
