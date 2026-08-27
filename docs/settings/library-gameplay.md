# Library & Gameplay semantic contract

This clean-room MVP describes semantic fields and actions only. It is metadata-ready input for a later generic descriptor; it is **not** a global settings schema, storage format, UI contract, or emulator/core contract.

## Shared semantics

- Stable IDs are suggestions and use the `library.` or `gameplay.` namespace.
- Value scopes are `domain_default`, `per_system`, and, where listed, `per_game`. Effective precedence is `domain_default` → `per_system` → `per_game`. A per-game value wins only when its value is valid and its capability gate is eligible; otherwise evaluation fails closed and does not silently fall through.
- An override is a value at `per_system` or `per_game`; it is distinct from the domain default. `gameplay.reset_overrides` removes selected overrides and never changes a domain default.
- `immediate` changes update effective state when accepted. `confirmed` changes create a pending value and leave the current effective value unchanged until confirmation. Cancellation discards the pending value.
- Capability gates model eligibility only. An unsupported or invalid request is rejected with a user-visible explanation and no persisted/effective-state mutation.
- All values are non-sensitive (`none`); no credential, path, ROM, BIOS, or private-corpus data is part of this contract.

## MVP requirements

| Stable semantic ID suggestion | Purpose | Field kind | Default | Constraints/options | Scope | Apply semantics | Sensitivity | Capability gate | Visibility rule | Failure behavior |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `library.show_hidden_entries` | Include entries marked hidden in library results. | boolean field | `false` | `true` or `false` | `domain_default`, `per_system` | `immediate` | `none` | `library.hidden_entries` | Visible to library users; show only when the library can classify hidden entries. | Invalid type or ineligible capability: reject, explain, and preserve state. |
| `library.show_favorites` | Show favorited entries in library views. | boolean field | `true` | `true` or `false` | `domain_default`, `per_system` | `immediate` | `none` | `library.favorites` | Visible when favorites are supported. | Invalid type or unsupported capability: reject, explain, and preserve state. |
| `library.show_collections` | Show collection-grouped entries in library views. | boolean field | `true` | `true` or `false` | `domain_default`, `per_system` | `immediate` | `none` | `library.collections` | Visible when collections are supported. | Invalid type or unsupported capability: reject, explain, and preserve state. |
| `library.refresh` | Request a fresh library view/index evaluation. | action | none | No value; accepts an optional library context only. | `library` action scope; not an override | `immediate` action; never persisted as a toggle | `none` | `library.refresh` | Visible when refresh can be requested. | Invalid parameters or unavailable capability: fail closed, explain, and leave settings unchanged. |
| `gameplay.auto_resume_policy` | Choose what launch does when a resumable session exists. | enum field | `prompt` | `off`, `prompt`, `resume_last` | `domain_default`, `per_system`, `per_game` | `confirmed` | `none` | `gameplay.auto_resume` | Visible for eligible launch contexts. | Invalid option/scope or unsupported capability: reject, explain, and preserve effective and persisted state. |
| `gameplay.default_save_state_behavior` | Set the default save-state behavior around a launch. | enum field | `manual_only` | `manual_only`, `load_last_on_launch`, `save_on_exit_and_load_last` | `domain_default`, `per_system`, `per_game` | `confirmed` | `none` | `gameplay.save_states` | Visible when save-state behavior is eligible. | Invalid option/scope or unsupported capability: reject, explain, and preserve state. |
| `gameplay.launch_confirmation` | Require an explicit confirmation before launching. | boolean field | `true` | `true` or `false` | `domain_default`, `per_system`, `per_game` | `confirmed` | `none` | `gameplay.launch_confirmation` | Visible in eligible launch contexts. | Invalid type/scope or unsupported capability: reject, explain, and preserve state. |
| `gameplay.aspect_ratio_default` | Select the default presentation aspect ratio. | enum field | `system` | `system`, `4:3`, `16:9`, `pixel` | `domain_default`, `per_system`, `per_game` | `confirmed` | `none` | `display.aspect_ratio` | Visible when the display context exposes aspect-ratio choices. | Invalid option/scope or unsupported capability: reject, explain, and preserve state. |
| `gameplay.integer_scaling_preference` | Express the preferred integer-scaling policy. | enum field | `prefer` | `off`, `prefer`, `require` | `domain_default`, `per_system`, `per_game` | `confirmed` | `none` | `display.integer_scaling` | Visible when integer-scaling eligibility can be evaluated. | Invalid option/scope or unsupported capability: reject, explain, and preserve state; an ineligible `require` request never falls back silently. |
| `gameplay.reset_overrides` | Remove selected per-system and/or per-game gameplay overrides. | action | none | Target must name an existing synthetic system or game context and one or more override scopes. | `per_system` and `per_game` action targets; not a value scope | `immediate` action; removes overrides only | `none` | `gameplay.override_reset` | Visible when matching overrides exist and reset is eligible. | Invalid target/scope or unsupported capability: reject, explain, and leave overrides and defaults unchanged. |

## Boundary decisions

- `domain_default` is the only default value layer defined here. This contract does not define profiles, accounts, storage keys, migrations, descriptors, or a global schema.
- System and game identifiers in fixtures are synthetic labels used only to exercise scope and precedence. They carry no device, ROM, BIOS, vendor, or private-corpus meaning.
- Unsupported means a known semantic request is not eligible under the named capability. Invalid means the request violates the field's type, options, scope, or action parameters. Both are user-visible, fail closed, and non-mutating.
- Confirmation is an operation boundary, not a persisted setting. A pending request may be represented for the caller, but it must not change effective or persisted state before confirmation.
