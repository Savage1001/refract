# Security

If you discover a security vulnerability in Refract, please [report it privately through GitHub](https://github.com/Savage1001/refract/security/advisories/new). If private vulnerability reporting is unavailable, email [nikhil.somanchi@gmail.com](mailto:nikhil.somanchi@gmail.com). This address is the maintainer's designated public security contact. Do not open a public issue with vulnerability details.

Include a description, steps to reproduce, the affected version, and any relevant proof of concept. Please avoid including private project data or credentials.

Please allow time for the issue to be investigated and fixed before disclosing it publicly. For other bugs, use the repository's issue tracker.

## Local execution

Refract does not sandbox TeX. `-no-shell-escape` disables TeX shell escape; it does **not** confine filesystem reads. pdfLaTeX can read absolute paths, and LuaLaTeX can use Lua file APIs. Engines, BibTeX/Biber, package lookup, and assistants run with the Windows account's permissions, including access outside the project and network access where supported. Output or assistant requests can disclose data they read. Do not compile or run assistants on documents from authors you do not trust.


Opening a project enables automatic compilation and assistant use without a separate approval step. This applies to new, existing, imported, duplicated, and restored projects. Review unfamiliar sources, uploaded files, restored versions, and assistant instructions/configuration before opening or using them. Provider runtimes may consume project instructions and execute commands independently of Refract's build tools.
