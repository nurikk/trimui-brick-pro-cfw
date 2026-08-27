# MVP Audio settings semantic contract

This document is a clean-room domain contract for later translation by a generic settings registry. The IDs below are stable semantic ID suggestions for this domain only; they are not a global descriptor or schema.

## Domain rules

- Editable values are global audio preferences. The output/status field is read-only.
- Volume is an integer percentage from `0` through `100`, in steps of `1`. `0` is silent and `100` is the maximum contract value. Mute is independent of the numeric volume; changing mute does not change the stored volume.
- Menu sounds, launch sound, and exit sound are independent boolean toggles.
- Background-music enablement is independent of the other sound toggles. Background-music volume is visible only when the capability is supported and the pending background-music enablement is `true`. Disabling it hides the volume, preserves its last valid stored or pending value, and performs no background-music-volume apply operation. Re-enabling it reveals that preserved value.
- A supported capability is required before any audio operation or probing. If unavailable, all audio settings and output/status are hidden; no read, write, reset, confirm, cancel, or probe operation is performed. This is fail-closed. A supported read-only output/status field is visible but cannot be written.
- Edits, including reset, remain pending until explicit confirm. Confirm applies valid pending edits atomically; cancel discards them. Reset replaces editable pending values with the documented defaults and still requires confirm. Invalid edits are rejected without changing pending or committed values.

## Requirements

| Stable semantic ID suggestion | Purpose | Field kind | Default | Constraints/options | Scope | Apply semantics | Sensitivity | Capability gate | Visibility rule | Failure behavior |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `audio.master_volume` | Set overall output level. | integer | `50` | Percent; integer `0..100`; step `1`. | Global audio preferences. | Set pending; apply only on confirm. | Normal | Supported audio-settings capability. | Visible when supported. | Reject non-integers or values outside `0..100`; preserve prior pending value. |
| `audio.mute` | Silence output without changing volume. | boolean | `false` | `true` or `false`; does not rewrite master volume. | Global audio preferences. | Set pending; apply only on confirm. | Normal | Supported audio-settings capability. | Visible when supported. | Reject non-boolean values; preserve prior pending value. |
| `audio.menu_sounds` | Enable sounds for menu actions. | boolean | `true` | `true` or `false`; independent toggle. | Global audio preferences. | Set pending; apply only on confirm. | Normal | Supported audio-settings capability. | Visible when supported. | Reject non-boolean values; preserve prior pending value. |
| `audio.launch_sound` | Enable the launch sound. | boolean | `true` | `true` or `false`; independent toggle. | Global audio preferences. | Set pending; apply only on confirm. | Normal | Supported audio-settings capability. | Visible when supported. | Reject non-boolean values; preserve prior pending value. |
| `audio.exit_sound` | Enable the exit sound. | boolean | `true` | `true` or `false`; independent toggle. | Global audio preferences. | Set pending; apply only on confirm. | Normal | Supported audio-settings capability. | Visible when supported. | Reject non-boolean values; preserve prior pending value. |
| `audio.background_music_enabled` | Enable background music. | boolean | `true` | `true` or `false`; controls background-music volume visibility. | Global audio preferences. | Set pending; apply only on confirm. | Normal | Supported audio-settings capability. | Visible when supported. | Reject non-boolean values; preserve prior pending value; no operation when capability is unavailable. |
| `audio.background_music_volume` | Set background-music level. | integer | `30` | Percent; integer `0..100`; step `1`; retained while background music is disabled. | Global audio preferences. | Set pending only while visible; apply only on confirm when enablement is pending `true`. Hiding it does not clear or apply its retained value. | Normal | Supported audio-settings capability and pending `audio.background_music_enabled=true`. | Visible only when supported and pending background-music enablement is `true`; otherwise hidden, not disabled. | Reject non-integers or values outside `0..100`; reject writes while hidden; preserve prior value. |
| `audio.output_status` | Report current output/status state. | read-only string | Not applicable; no editable default. | Read only; writes are rejected. | Global audio status. | No apply action. | Normal | Supported audio-settings capability. | Visible when supported; unavailable capability hides it. | Reject every write; unsupported capability performs no read or probe operation. |

The generated scenario contract is at `fixtures/settings-domains/audio/scenarios.json`.
