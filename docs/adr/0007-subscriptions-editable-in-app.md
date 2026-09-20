# Subscriptions are editable in-app (supersedes part of ADR-0006)

The subscription registry is still one YAML settings file, and the subscription file is still the interface for nodes and rules — but subscription *entries* (name, update URL, auto-update) are now editable from the TUI, not only by hand. This reverses ADR-0006's "adding or removing a subscription is done by editing the settings file, not in the app".

The app writes `settings.yaml` only for these registry mutations, atomically (write a sibling temp file, then rename) and only from the in-memory `Settings` it parsed at launch. Hand-editing the file remains fully supported and still takes precedence on the next launch. The subscription `file` path is not user-editable in the form: it is derived at add time as `<config_dir>/subscriptions/<slug>.json` and kept stable across edits, so renaming a subscription never orphans its local file. The client still never writes the subscription file except on update.

Considered: keeping ADR-0006 as-is and shipping a config-file editor in the app (rejected — the user explicitly asked for in-app management of the three registry fields); making `file` a form field (rejected — the derived-and-stable path removes a whole class of "edit the file to match the path" mistakes).
