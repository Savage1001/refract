# Codex picker integration

The renderer consumes `store.discoverCodexModels()` and `store.codexPreferencesLoad()` / `store.codexPreferencesSave()`. Preferences and favorites use the catalog's `model` value as the CLI override. When no model is saved, the picker selects the model marked as default in the catalog, or the first listed model if the catalog has no default. It shows supported reasoning efforts and defaults to Low when available. It uses the advertised default or lowest supported effort for models without Low.

ChatGPT account mode and CLI mode keep separate Codex preferences. Preferences are loaded before the picker allows changes. A failed load disables editing to avoid overwriting saved data. Writes are serialized, and a failed save is surfaced with a retry action. Discovery failures and unavailable models do not erase preferences. Saved preferences are passed to new and resumed Codex conversations. Discovery and preferences are whole-snapshot APIs; any other writer must coordinate updates to avoid lost changes.


The Codex picker footer offers Standard and Fast. Standard is the default and passes `service_tier="default"` on send. Fast is saved with the Codex preferences and uses `service_tier="fast"` and `features.fast_mode=true` for new and resumed CLI runs. Fast availability and usage depend on the selected model and account. [OpenAI speed documentation](https://learn.chatgpt.com/docs/agent-configuration/speed)
