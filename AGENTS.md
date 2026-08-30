# Repository agent rules

## Simulator controller coverage

In controller coverage, “two fresh roots” means **two simulator instances total**, not per-route restarts: reuse one instance within each pass. Run the representative smoke subset before exhaustive coverage, estimate multiplicative runtime before launching loops, and use bounded timeouts with fail-fast diagnostics.
