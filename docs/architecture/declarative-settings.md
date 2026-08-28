# Declarative settings registry

The settings registry is a versioned, closed data contract. A descriptor identifies a stable setting, its section/group/order and localization keys, typed values, constraints, scope, apply mode, capabilities, predicates, redaction policy, and migration metadata. The `settings-schema` crate validates the registry and projects it into deterministic sections, groups, and form controls; adding a descriptor does not require launcher UI Rust changes.

## Closed data boundary

Registry JSON uses strict structs and rejects unknown fields and duplicate JSON keys. It has no script, shell, command, executable, dynamic-library, URL-fetch, or arbitrary-expression fields. The only predicate language is the bounded typed AST of `all`, `any`, `not`, `equals`, `present`, and `capability`. References must name declared settings; cycles, excessive depth, and excessive predicate nodes fail closed.

Provider namespaces are explicitly listed in `allowedNamespaces`. A setting ID must be inside its namespace, and a provider cannot claim a `core.*` ID. Duplicate section, setting, option, capability, and migration IDs are rejected. Numeric ranges, text limits/patterns, enum options, typed values, and apply-mode cardinality are validated before projection. Actions use `external-operation`; read-only and status controls use `immediate`.

The generic schema supports secret fields through an opaque `credentialRef`; secret bytes are never a registry value, and redacted controls expose neither value nor reference in projected menu/form JSON. The generated Scraper provider declarations use status controls for credential requirement/configuration and contain no credential reference. The Wi-Fi fixture remains a placeholder and performs no network operation.

## Versioning and canonicalization

Schema version `1` is the current format. Unsupported versions are rejected rather than deserialized permissively. Future formats must add an explicit migration descriptor with typed changes before they can be supported. Canonical registry JSON sorts namespaces, sections, settings, and migrations by stable keys. Projection sorts by section order, group, setting order, and ID and evaluates predicates and capabilities fail-closed.

The registry is no-device, no-network-operation data. It does not probe hardware, invoke providers, fetch URLs, execute migrations, or perform settings operations; those concerns remain outside this contract.
