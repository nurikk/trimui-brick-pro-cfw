# Crash-safe resume validation

The broker records checkpoints on normal exit, pre-suspend, low-battery, and at most once every 30 seconds while a session is active. A periodic write failure leaves the last durable generation visible.

The generated TG4040 simulator journey is `tools/sim/journeys/resume-crash-safe.sh`. It drives the typed controller path, injects artifact/metadata/promotion/pointer faults, verifies crash recovery, and validates the four legal demos without exposing storage paths. It also proves exact runner/core-version rejection, an explicitly retained alternate core, unchanged SRAM/save bytes, and lifecycle checkpoint ordering.

The session-broker fixture journeys `standalone-sram-only` and `standalone-undeclared` prove that declared SRAM-only behavior is recorded while an undeclared standalone remains non-resumable.
