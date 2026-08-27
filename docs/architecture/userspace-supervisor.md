# Userspace supervisor contract

`userspace-supervisor` is a deterministic fixture-root simulator, not a daemon and not a device boot entrypoint. Startup is ordered `HAL readiness -> session broker -> launcher`; shutdown is the reverse. Readiness and status are represented by typed JSON and a Unix socket at `.brickpro/data/update/supervisor.sock`, which is created and removed only below the explicit fixture root.

The simulator covers `healthy`, `launcher-restart`, `broker-restart`, `session-failure`, `essential-failure`, `hal-loss`, and `shutdown`. Essential-service restarts are bounded at two and synthetic children are reaped. A game/session failure is reported as `sessionFailure`, not boot-health failure. Launcher or broker restart may preserve readiness. HAL loss withdraws readiness, requests controlled next-boot recovery, and never blesses a pending release.

All paths are logical fixture paths. There is no stock process, device node, mount, `/system` overlay, block write, eMMC operation, or hardware claim. The simulator starts the same binary in an explicit child-helper mode, kills and waits for each synthetic child, and records every restart and reap. It proves lifecycle ordering and failure classification only; it does not prove physical TG4040 behavior.
