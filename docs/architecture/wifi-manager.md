# Wi-Fi manager

This is a data-only Wi-Fi contract. It owns typed scan state, saved metadata, semantic UI scenes, and redacted events; it does not own a credential store, radio device, host path, log sink, or hardware integration.

## Redaction and persistence boundary

`NetworkId` and `CredentialReference` are validated opaque identifiers. A saved record contains only the network ID, display SSID, security, and an optional credential reference. The manager never accepts or stores password bytes. Public `WifiState`, `WifiEvent`, and scene payloads contain only display data, bounded state/reason/count values, and opaque network IDs. Hidden/manual SSIDs are validated at input and represented publicly as `Hidden network`; they are never emitted in state or events. BSSID, host paths, credential references in events, and secrets are outside the public contract.

## Backend boundary

`WifiBackend` is the production boundary. `GeneratedWifiBackend` is an offline deterministic test double loaded only from `fixtures/wifi-manager/journeys.json`. It contains no device, daemon, socket, filesystem, or credential-store access. `Tg4040WifiPort` is an unavailable port definition only; no TG4040 implementation or hardware claim exists.

Scanning collapses radio duplicates by display SSID. Supported security wins over unsupported security; within that class the strongest signal wins, then security rank and opaque network ID make ties deterministic. Known and connected flags are merged across duplicates. Public ordering is connected first, known second, signal descending, display name, then opaque ID.

The manager supports enable/disable, scan/rescan, selection, credential/confirmation-gated connect, disconnect, forget, retry, cancel, and modeled auto-reconnect. Auto-reconnect blocks for disabled policy, low battery, suspend, active gameplay, unavailable capability, or no saved candidate; open networks require explicit confirmation.

## Validation

The fixture is JSON-schema validated and exercised by the bounded `wifi-manager-fixtures` CLI. The CLI executes scan/refresh/collapse, hidden/manual input, successful open/WPA2/WPA3 connects, bad credentials, timeout, radio failure, cancellation, reconnect policy, disconnect, forget, retry, restart, scene generation, malformed input, unsupported security, and persistence/event redaction. Host and static AArch64 Cargo gates remain container-dependent; no device, live Wi-Fi, private corpus, `/sys`, `/proc`, socket, or network access is part of this contract.
