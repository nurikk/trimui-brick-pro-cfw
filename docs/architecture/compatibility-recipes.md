# Compatibility recipes

A compatibility recipe is ordinary versioned JSON from the configured project or upstream catalog. It identifies a TG4040 ROM hash, system, exact core version, safe configuration delta, profile references, and known issues. The parser is closed, bounded, and rejects payload-shaped fields, unsafe values, duplicate keys, and malformed paths.

The launcher matches the recipe to the supplied ROM hash and current catalog, then supports preview, explicit apply, and undo. Apply writes only the private recipe layer and a pre-change rollback record through atomic publication. ROMs, saves, states, settings, and the Save Vault remain protected.

The fixture journey covers local matching, precedence/collision preview, explicit apply, rollback, mismatch rejection, bounded input, and malformed JSON.
