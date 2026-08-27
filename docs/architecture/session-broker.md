# Session broker

`session-broker` is the clean-room consumer of the typed `launch-contract::LaunchRequest`. Its host journey is software evidence only; it does not claim device, stock ABI, loader, process-group, emulator, or hardware compatibility.

## Boundary

The broker accepts only a catalog member and rooted fixture content. It revalidates the catalog, request, canonical paths, and SHA-256 immediately before materializing a plan. Executable, shell, argv, environment, cwd, redirection, and process options are not request fields. The three adapter modules contain the only plan mappings:

- RetroArch: fixed synthetic runtime, `--config`, `-L`, and content arguments.
- Standalone: fixed synthetic runtime with typed content/save/state substitutions.
- PortMaster: fixed package-manager-resolved private runtime and verified package entrypoint; no script text.

The child receives a cleared environment, one 256-bit CSPRNG ownership marker, a launch-barrier descriptor, null standard streams, and a fixed fixture working directory. `pre_exec` creates a new session/process group and sets a parent-death signal; the workspace-built helper waits on the parent-owned barrier before doing session work. The helper used by journeys is never a host utility.

## Lifecycle and recovery

A broker has exactly one lifecycle: `idle -> preparing -> running -> finalizing -> idle`. A non-idle request is a typed `busy` rejection. A complete typed platform snapshot is captured before profile application. The checksummed ownership journal is written below the supplied fixture's `data/activity/sessions` boundary before spawn, and the Running record is durably published before the barrier is released. Every child outcome, including spawn failure, uses the same finalization path: verify PID start time, PGID, and exact marker, terminate only the owned process group with bounded TERM/wait/KILL escalation, restore the snapshot, and use the typed safe default if restoration or journal handling is unrecoverable.

Terminal output and append-only activity records contain only request identity, catalog-owned runner/core identity, reason, measured running duration, restoration status, safe-default status, and persistence status. They contain no content paths, hashes, command lines, URLs, or credentials. Playtime, recent, terminal result, and resume records are synced before reporting; playtime uses only measured running time, and resume is published only after an adapter confirms a usable save/state and terminal persistence succeeds.

Startup recovery accepts only valid checksummed broker-owned journals. Invalid records are not interpreted as process instructions. A valid unfinished record may stop a surviving process only after its ownership marker is verified; otherwise recovery leaves that identity alone and returns the logical platform to a safe default.

## Fixture journey

Run the complete deterministic journey set with:

```text
cargo run --locked -p session-broker --release -- simulate --journeys success,standalone,portmaster,portmaster-success,portmaster-rejection,portmaster-mismatch,portmaster-symlink,portmaster-injection,portmaster-nonzero,nonzero,signal,timeout,cancel,grandchild,restart,marker-mismatch,start-time-mismatch,publication-failure,crash-before-publish,crash-after-publish,crash-after-release,result-fsync-failure
```

Each JSON line is one typed result and exposes `restored: true`. The generated fixture is synthetic and is copied to a temporary root before execution. No hardware, private corpus, live service, or real `/data` path is used.
