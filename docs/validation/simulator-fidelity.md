# Simulator-fidelity contract

## Purpose and authority

This document is the authoritative clean-room contract for simulator evidence. It defines what each lane can demonstrate; it does not claim that a simulator is hardware emulation or that this repository is compatible with a device. The only target SKU named by the synthetic profile is `TG4040`.

The contract's lane labels are exact and must be copied into every evidence record, report, and claim:

- **host-native userspace simulator** — a deterministic test double running natively on the development host.
- **aarch64/QEMU** — a target-architecture userspace environment used for ABI, packaging, and shell behavior.
- **physical TG4040 hardware-in-loop** — real hardware exercised through an approved HIL setup.

## Fidelity matrix

| Subsystem | host-native userspace simulator | aarch64/QEMU | physical TG4040 hardware-in-loop |
| --- | --- | --- | --- |
| UI and session flows | Supported for deterministic semantic routes, selections, transitions, and fake display output; no hardware fidelity claim. | Supported for target userspace launch and session behavior where the environment provides it; no hardware fidelity claim. | Only lane that may close hardware-backed UI/session claims. |
| ABI, packaging, and shell behavior | Host ABI and package/shell smoke evidence only; not target ABI evidence. | Supported for aarch64 userspace ABI, packaging, and shell behavior. | May close claims involving the physical target's deployed userspace. |
| Storage media | Synthetic logical/virtual storage evidence only; cannot prove physical media, filesystem durability, corruption behavior, or power-loss safety. | Package, filesystem, and userspace behavior only; cannot prove physical microSD/eMMC behavior or durability. | The only lane that may close physical media, mount, corruption, durability, and power-loss claims when actual conditions are recorded. |
| A133P/CPU and board behavior | Not claimed; deterministic fake only. | Not claimed; QEMU is not evidence of the A133P or board. | HIL only. |
| PowerVR/GPU and framebuffer behavior | Not claimed; logical display output only. | Not claimed; no PowerVR or physical framebuffer proof. | HIL only. |
| Physical controls and Hall/input | Not claimed; semantic control events are generated input. | Not claimed; injected userspace input is not physical control evidence. | HIL only. |
| PMIC, battery, charging, and power | Not claimed; battery and power values are fixture state. | Not claimed; QEMU does not prove PMIC, charging, or power behavior. | HIL only. |
| Suspend/resume | Not claimed; suspend state is a deterministic fixture field. | Not claimed; a QEMU lifecycle is not physical suspend/resume evidence. | HIL only. |
| xradio/radio | Not claimed; radio state is a deterministic fake. | Not claimed; userspace radio configuration is not radio evidence. | HIL only. |
| Audio, LED, rumble, USB | Semantic state and route evidence only; no peripheral fidelity claim. | Userspace integration evidence only; no physical peripheral fidelity claim. | HIL only for physical peripheral behavior. |
| Thermal and device performance | Not claimed; host timing and throughput are irrelevant to device qualification. | Not claimed; QEMU timing and throughput are not device performance evidence. | HIL only, with a separately defined qualification method. |

## Lane evidence and hard non-claims

### host-native userspace simulator

Positive evidence is limited to repeatable execution of the synthetic contract in `sim/contracts/virtual-device.schema.json`, deterministic fake controls and state transitions, semantic route/selection records, JSONL events, logical screenshots, readiness, and process exit status. It can expose application logic and host-native userspace flow defects.

This lane does **not** emulate or prove A133P, PowerVR, PMIC, xradio, physical controls, Hall/input, suspend, radio, power, thermal, or device-performance behavior. Battery, LED, audio, radio, suspend, and display values are test-double state, not observations.

### aarch64/QEMU

Positive evidence is limited to aarch64 userspace ABI, packaging, shell behavior, process lifecycle, and the semantic artifacts produced by that userspace environment. QEMU is an ABI and userspace lane, not a board simulator.

This lane does **not** emulate or prove A133P, PowerVR, PMIC, xradio, physical controls, Hall/input, suspend, radio, power, thermal, or device-performance behavior. It does not turn a virtual display, injected input, or userspace peripheral API into physical evidence.

### physical TG4040 hardware-in-loop

This is the only lane that may close claims about the listed hardware subsystems: A133P/CPU and board behavior, PowerVR/GPU and framebuffer behavior, physical controls and Hall/input, PMIC/battery/charging/power, suspend/resume, xradio/radio, physical audio/LED/rumble/USB, thermal behavior, and device performance. Its records must identify the exact lane label and the test conditions needed for the claim.

No physical HIL result is available from this clean-room scaffold. A HIL claim requires observed evidence from real hardware; a fake or QEMU result cannot be relabeled as HIL.

## Evidence promotion rules

1. A **host-native userspace simulator** result may justify preparing or running an **aarch64/QEMU** check for target userspace ABI, packaging, and shell behavior.
2. An **aarch64/QEMU** result may justify preparing a **physical TG4040 hardware-in-loop** run.
3. Neither the **host-native userspace simulator** nor **aarch64/QEMU** is a substitute for **physical TG4040 hardware-in-loop**.
4. Promotion preserves the original lane label. A promoted test plan is not promoted evidence.
5. Reports must state the lane label in the claim itself or in an unambiguous per-record `lane` field. Hardware claims are closed only by evidence labeled **physical TG4040 hardware-in-loop**.

## Stable artifact interfaces

All interfaces below are UTF-8, deterministic, and owned by the process that writes them unless stated otherwise. The caller selects `$EVIDENCE_DIR`; it must be outside this repository. `$VIRTUAL_SD_JSON` is a caller-supplied logical input reference, not a host path that may be copied into evidence.

### Caller-supplied virtual-SD input

The caller supplies one JSON value matching the `virtualStorage` property of `sim/contracts/virtual-device.schema.json`. It contains only logical keys and synthetic text content. The simulator owns reading and validating the value for the run; the caller owns its source. Host source paths, private filesystem paths, content identifiers, ROM/BIOS/firmware content, and vendor blobs are prohibited in the input and in public evidence.

### Deterministic readiness record

The lane writes `readiness.json` in `$EVIDENCE_DIR` with exactly these fields:

```json
{"schema":"sim-readiness/v1","lane":"host-native userspace simulator","targetSku":"TG4040","ready":true,"elapsedMs":0,"reason":"ready"}
```

`schema`, `lane`, `targetSku`, and `reason` are strings; `ready` is boolean; `elapsedMs` is a non-negative integer. `reason` is `ready` when `ready` is true and a stable failure reason when false. The caller waits for this file for at most 10,000 ms, polling no more often than every 100 ms. Missing, malformed, late, or false readiness is a failed start, not permission to infer readiness. The lane label in this record is owned by the process that emitted it; the caller verifies it before driving the session.

### Semantic route/selection, launch, and session records

The process writes UTF-8 JSON records with stable keys and no widget coordinates or private identifiers:

- `route-selection.json`: `{"kind":"route-selection","lane":"...","route":"...","selection":"..."}`. The caller owns the semantic route and selection values; the process owns the lane.
- `launch.json`: `{"kind":"launch","lane":"...","targetSku":"TG4040","sessionId":"run-local"}`. The process owns the launch record and the opaque run-local session ID.
- `session.json`: `{"kind":"session","lane":"...","sessionId":"run-local","state":"completed"}`. The process owns lifecycle state; allowed state values are `started`, `completed`, `failed`, and `aborted`.

The concrete route and selection strings are semantic names, not source paths or content identifiers. Public evidence must replace any caller-private run token with `redacted` rather than exposing it.

### JSONL event logs

`events.jsonl` contains one JSON object per line, in sequence order. Every object has exactly these base fields: `{"sequence":0,"atMs":0,"lane":"...","event":"..."}`. `sequence` and `atMs` are non-negative integers; `lane` and `event` are strings. Optional event detail fields must remain semantic, deterministic, and free of host paths, private content identifiers, ROM/BIOS/firmware content, and vendor blobs. The process owns the log; the caller owns the requested control sequence.

### Screenshots

The process may write PNG screenshots named `screen-<sequence>.png` under `$EVIDENCE_DIR/screenshots/`. Each is a logical 1024x768 capture associated with an event sequence, and its accompanying claim carries the lane label. Screenshots are owned by the process. They are visual evidence of the lane's output only, never proof of a physical display or GPU.

### Process exit status

The process exits with an integer status and writes `exit-status.json` with exactly these fields:

```json
{"lane":"host-native userspace simulator","sessionId":"run-local","exitCode":0,"cleanShutdown":true}
```

`lane` and `sessionId` are strings, `exitCode` is an integer, and `cleanShutdown` is boolean. The process owns the file; the caller records both the observed exit status and the file. A nonzero status, missing file, or disagreement is a failed run.

### Evidence directory and sharing

The caller owns `$EVIDENCE_DIR`, creates it before launch, and may archive or delete it after validation. The directory is an external per-run boundary, not a repository artifact. Publicly shareable evidence must contain lane labels and semantic values only; it must not contain private host paths, source paths, machine-private paths, private content identifiers, or copied ROM/BIOS/firmware/vendor content.

## Agent workflow

1. Prepare a generated deterministic fixture matching the virtual-device schema and create caller-owned `$EVIDENCE_DIR` outside the repository.
2. Start the selected lane and write the launch record.
3. Perform the bounded readiness wait; stop on timeout, malformed readiness, a wrong lane, or `ready: false`.
4. Drive semantic controls and selections, not device-coordinate assumptions.
5. Capture semantic state, JSONL events, screenshots, readiness, and process exit status.
6. Request shutdown and verify `cleanShutdown: true` and a zero exit status for a successful run.
7. Label every claim with its lane, then promote only according to the rules above.

A clean shutdown writes a completed session record and exit status. On partial failure, retain completed records and the failure indication, write a failed or aborted session state when possible, do not manufacture missing artifacts, and report the exact lane and failed step. Partial evidence cannot close a hardware claim.

## Out of scope

This contract does not implement a simulator, provide QEMU images, build HIL automation, flash hardware, supply performance qualification, or add device images, ROMs, BIOS, firmware, vendor data, dependencies, or test infrastructure. Those activities require separate approved work and cannot be inferred from this documentation or the synthetic profile.
