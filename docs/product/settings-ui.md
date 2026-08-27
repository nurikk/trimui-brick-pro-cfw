# Controller-first settings UI

`settings-ui` is a renderer-neutral projection of the validated `settings-schema` registry. It produces semantic 1024×768 scenes for a section list and the selected section's form; another component owns drawing and platform integration.

## Public semantic contract

`SettingsUi::new` accepts a validated `settings_schema::Registry` and `ProjectionContext`. The registry remains the authoritative source for section order, groups, labels, descriptor kinds, constraints, predicates, capability requirements, redaction, values, scopes, and apply mode. `SettingsUi::scene` returns ordered `SectionScene` values, setting controls, selected-control help, a `ui-model` controller help strip, pending changes, validation errors, capability-disabled reasons, keyboard request metadata, and external-operation requests.

Control kind is projected from `FieldKind` without provider or setting-specific mappings. Boolean, enum-single, enum-multi, integer, decimal, text, secret, action, read-only, and status descriptors use the same generic path. A new valid descriptor automatically follows its registry section, group, and order. No section name, descriptor ID, or concrete provider is referenced by the library implementation.

Scenes expose `Surface::SectionList` or `Surface::Form`, stable IDs, semantic values, and no rendering primitives. `SemanticValue::Masked` is the only value representation for redacted controls. `ApplyBadge` marks restart-launcher and reboot-candidate controls.

## Controller and apply behavior

`press` and `ControllerAction` support section/form navigation, activation and editing, value selection/toggling, back, cancel, confirm, and apply. Immediate values update the in-memory committed projection. Other editable values remain in the deterministic pending summary until confirm/apply; cancel discards them. Validation rejects values without coercion and records a safe `ValidationError`; descriptor text patterns are evaluated with the locked `regex-automata` backend. All state is in-memory; this crate does not persist settings.

Action descriptors create only `ExternalOperationRequest` metadata. No external operation is started. Restart and reboot are semantic badges, not lifecycle calls.

## Keyboard and redaction boundary

Text, integer, and secret editing use `virtual-keyboard::FieldPolicy` and `Keyboard` through their public API. Decimal controls use generic controller stepping from their validated numeric range; they do not open the integer-only virtual keyboard. `KeyboardRequest` identifies the setting and reports only field type, masking, and length metadata. A `KeyboardSession` exposes the keyboard's masked scene and semantic events. A confirmed secret is accepted only at `accept_keyboard`'s boundary, its scalar length may be retained as metadata, and its value is immediately discarded; it is never committed, placed in a scene, event, debug output, or fixture.

The Wi-Fi descriptors remain ordinary capability-gated registry controls. This crate performs no Wi-Fi, network, radio, device, filesystem, shell, hardware, credential-store, or remote operation.

## Deterministic fixtures

`settings-ui-fixtures` loads the existing validated synthetic registry, adds one valid descriptor without changing the library, and checks the generic projection path. Its journey covers Display, Audio, Input, Scraper, Theme, System, Network, and placeholder Wi-Fi; all control kinds; ordered scenes; help; disabled capability reasons; predicates; restart/reboot badges; keyboard invocation; validation failure; pending cancel/confirm/apply; external request metadata; and redaction. `fixtures/settings-ui/journey.json` is generated synthetic evidence only and contains no credential reference, secret payload, private corpus data, device path, or remote-access data.

Repeated scene and fixture serialization is byte-identical. The fixture executable rejects malformed fixture JSON and forbidden secret-reference text.

## Non-goals

This component does not render pixels, own navigation routes, persist settings, execute actions, communicate with Wi-Fi or other providers, access hardware, or resolve credentials. It does not alter the authoritative schema or virtual keyboard.
