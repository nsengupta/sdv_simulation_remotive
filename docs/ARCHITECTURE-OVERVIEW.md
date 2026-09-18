# SDV simulation — architecture overview

This document is the **topology / gap register** for Simulation 5. Detailed Simulation 5
phase history lives in https://github.com/nsengupta/sdv_simulation_5.
Roadmap livedoc: [`PLAN.md`](PLAN.md). Remotive observation path:
[`DESIGN-remotive-observation.md`](DESIGN-remotive-observation.md).
**Tests are mandatory** for every capability slice.

Related:

- [`DESIGN.md`](DESIGN.md) — Stage 5 process / observation / Dashboard decisions
- [`DESIGN-remotive-observation.md`](DESIGN-remotive-observation.md) — Remotive Broker / FLCM / runbook
- [`PLAN.md`](PLAN.md) — capability summary + important TBDs
- [`TODO-twin-lifecycle.md`](TODO-twin-lifecycle.md) — TL-0–TL-5 done; TL-6+ → shutdown/disband
- [`TODO-simulation-5.md`](TODO-simulation-5.md) — engineering backlog
- Library pyramid (L0–L6): see [`sdv_simulation_5`](https://github.com/nsengupta/sdv_simulation_5)
- [`design-documents.md`](design-documents.md) — catalogue

---

## 1. Target topology (final round)

Five **independently runnable** command-line applications share a message carrier.
**CAN (`vcan0`) is the first carrier** and must work defect-free before any Zenoh work.

```text
                    ┌─────────────────────────────────────┐
                    │  Dashboard (TUI — driver + engineer) │
                    │  • displays diagnostic + ledger      │
                    │  • embeds Emulator *core* as library │
                    │  • deferred CSV echo / driver UI     │
                    └──────────┬───────────────┬───────────┘
                               │               │
              observation      │               │ lifecycle + sensors
              (live / replay)  │               ▼
                               │      ┌─────────────────┐
                               │      │ Emulator process │──┐
                               │      │ (or in-dash lib) │  │
                               │      └─────────────────┘  │
                               │                           │ CAN 0x100, 0x102…
                               ▼                           ▼
                    ┌──────────────────────────────────────────────┐
                    │              vcan0 (today)                    │
                    │         Zenoh + uProtocol (much later)        │
                    └──────▲───────────────────────▲───────────────┘
                           │                       │
              ┌────────────┴────────┐    ┌───────────┴────────────┐
              │ Gateway            │    │ Actuators (×N)         │
              │ (Digital Twin)     │    │ headlamp, wiper, …     │
              │ CAN ingress        │    │ CMD in / ACK out       │
              │ actuation egress   │    │ (carrier-adaptive)     │
              └────────────────────┘    └────────────────────────┘
```

| Application | Role |
|-------------|------|
| **Gateway** | Houses the **entire Digital Twin** (actor tree, FSM, zones, `SessionClock`). Reads driver/sensor ingress and publishes actuation. Emits **diagnostic** and **transition ledger** observation streams. |
| **Emulator** | Sends lifecycle (`PowerOn`/`PowerOff`) and bounded-random sensor frames onto the bus. Today it requires `--readings N`, writes a finite `3N + 3` frame run, and exits. It does not know whether the twin accepts its inputs. CSV/echo is a deferred future option. |
| **Actuators** | Separate processes per assembly (headlamp, wiper, …). Listen for CMD frames; respond only after receiving CMD. No spontaneous bus traffic. Future actuators follow the same pattern. |
| **Dashboard** | Observer-only TUI (driver / engineer / ledger-tail panes + UDS live link) displaying twin-authored diagnostic and ledger output; no lifecycle keys. Embedded emulator / driver UI was **cancelled** for this simulation. Observation only — never direct FSM injection. |

### 1.1 Message carrier

| Carrier | When | Scope |
|---------|------|--------|
| **CAN (`vcan0`)** | From CAN lifecycle onward (stays) | Emulator ↔ Gateway ↔ Actuators |
| **File + UDS** | Gateway/Dashboard split Done | Gateway observation archive + live Dashboard link |
| **Zenoh (observation)** | Zenoh live link Done | Alternate live Dashboard link; versioned observation payloads; explicit `--uds` / `--zenoh` |
| **uProtocol** | After Zenoh live link if needed | Optional SDV service layer — not required for observation Zenoh |
| **Zenoh vehicle bus** | Later / stretch | Emulator/actuators off CAN — not part of observation Zenoh |

**Live-link CLI rule:** mutually exclusive flags — Gateway `--uds` / `--zenoh` / `--no-live`;
Dashboard `--uds` / `--zenoh` (exactly one each; no default) so live ends cannot silently
mismatch. Vehicle bus remains CAN.

### 1.2 Dashboard operating modes

| Mode | Status | Behaviour |
|------|--------|-----------|
| **Live + standalone emulator** | **Current** | Emulator → CAN → Gateway; Dashboard observes via UDS or Zenoh |
| **Live + CSV / embedded emulator** | **Cancelled** | Not scheduled this simulation |
| **Live + TUI driver keys** | **Cancelled** | Not scheduled this simulation |
| **Replay** | **TBD next simulation** | Standalone Dashboard from archived run dirs |

### 1.3 Twin behaviour (unchanged commitments)

| Topic | Rule |
|-------|------|
| **While FSM `Off`** | **Silent ignore** — no ledger, no context mutation, no zone side-effects. Only **`PowerOn`** is handled. |
| **Lifecycle on bus** | `PowerOn` / `PowerOff` via CAN ID **`0x100`** (see [`TODO-simulation-5.md`](TODO-simulation-5.md) §1). |
| **PowerOff guard** | Rejected unless parked (`Idle` / standstill); twin records rejections on diagnostic + ledger. |
| **Trust boundary** | Dashboard panes show **only** twin emissions (+ static install metadata). No duplicated FSM logic in the UI. |
| **Session clock** | One `SessionClock` per twin install; `PowerOn`/`PowerOff` are **not** clock anchors. |

### 1.4 Observation storage (target)

Human-readable, versioned artifacts:

- **Diagnostic stream** and **transition ledger** stored as interpretable files (format fixed with observation capture).
- Metadata: **run-id**, **timestamp**, **schema/version**, car identity, optional scenario CSV hash.
- Optional **observation library** crate used by Gateway (writer) and Dashboard (reader / replay).
- Standalone CLI utilities may pretty-print or diff runs (hand-made tools OK).

---

## 2. Transitional state (today)

Gateway/Dashboard process split is done. Live observation UDS|Zenoh is done. Embedded
emulator UI is **cancelled** for this simulation. Replay is **TBD next simulation**.
Next lifecycle work is shutdown/disband.

| Aspect | Today | Target |
|--------|--------|--------|
| Twin location | **Gateway** process via `TwinRuntimeBuilder` | **Gateway** process only |
| Dashboard ↔ Twin | Explicit `--uds` or `--zenoh --keyexpr` via `LiveSink`/`LiveSource` (observation schema versioned; Remotive path currently **v8**) | Same; vehicle-bus Zenoh / uProtocol later |
| Lifecycle | Mode 1 emulator → CAN **`0x100`**; Dashboard has no lifecycle controls | Emulator or future driver UI → CAN **`0x100`** |
| Emulator | Separate binary; `TelemetrySource` + session runner; optional `--readings N` or Ctrl+C; optional `--tick-ms` (default 100) to pace ticks for demos; live bounded-random telemetry | Mode 2 file source / generator and embedded-driver options are deferred TODOs |
| Observation capture | Gateway `ObservationTee` → `RunWriter` (+ optional UDS); Dashboard observation-only | Unchanged file contract; replay deferred from run dirs |
| Dashboard presentation | Driver / engineer / ledger-tail; footer shows UDS connected/disconnected | Honest gaps (`—`) until Twin fields are added; inline widgets later |
| Replay | None | TBD next simulation |

**Naming:** keep crate **`tui_dashboard`** for now. **`simulator`** is reserved for a possible future umbrella binary name.

---

## 3. Gap register (objectives vs code)

| ID | Gap | Capability / status |
|----|-----|---------------------|
| G1 | **Closed:** CAN `0x100` → PowerOn/PowerOff wired at gateway ingress | CAN lifecycle |
| G2 | **Closed:** silent ignore while `Off` enforced at the twin FSM boundary | CAN lifecycle |
| G3 | **Closed:** finite emulator sends full lifecycle and telemetry on CAN | Emulator session |
| G4 | CSV/echo / Mode 2 file `TelemetrySource` deferred (seam exists; reader TODO) | Future / post–emulator session |
| G5 | **Closed:** Gateway sole twin owner; Dashboard UDS observation consumer | Gateway/Dashboard split |
| G6 | **Closed:** versioned, human-readable observation artifacts written by the L6 `observation` adapter | Observation capture |
| G7 | No E2E observation golden / `observation-compare` yet (emulator session delivered; golden remains TODO) | Later |
| G8 | Embedded emulator / TUI driver — **dropped** for this simulation | Cancelled |
| G9 | No standalone replay mode | Replay deferred (TBD next simulation) |
| G10 | Observation live link UDS + Zenoh `LiveSink`/`LiveSource` | **Closed** (Zenoh live link) |
| G11 | No graceful twin disband on Gateway stop (Dashboard `q` no longer tears down twin) | Shutdown/disband (TL-6/7) |

---

## 4. Run order (CAN, multi-process)

```bash
cargo run -p front_headlamp_actuator
cargo run -p wiper_actuator
cargo run -p gateway -- --uds observation.sock
# after Gateway prints “waiting for Dashboard”:
cargo run -p tui_dashboard -- --uds observation.sock
EMULATOR_TUNNEL_PROB=0.01 \
EMULATOR_RAIN_PROB=0.008 \
cargo run -p emulator -- --readings 30
# optional demo pace: --tick-ms 400 (default 100)
```

UDS paths resolve under `<cwd>/tmp/` (default `./tmp/observation.sock`). Headless capture: `cargo run -p gateway -- --no-live` still writes `./observations/<run-id>/`.
Automated smoke: [`scripts/smoke-two-process.sh`](../scripts/smoke-two-process.sh).

With `--readings N`, the emulator sends PowerOn, then `N` RPM/lux/rain cycles, then RPM zero and
PowerOff (`3N + 3` frames). Inter-tick wait defaults to **100 ms** (`--tick-ms`); raise it to slow
demos without changing Gateway/Dashboard. Without `--readings`, it runs until Ctrl+C on the
emulator process,
then sends the same trailer. PowerOff transmission does not guarantee acceptance: if FSM guards
reject it, the observer-only Dashboard displays the twin's actual unchanged state and rejection
evidence. The existing twin startup barrier, verified by contract test, orders immediate
post-PowerOn readings behind assembly startup.

### 4.1 Observation capture

The `observation` crate is an L6 persistence adapter. It depends downward only on
`common::facade`, which exposes the live diagnostic and transition-record types; `common` never
depends on `observation`. The detailed pyramid boundary is documented in
the Simulation 5 library-pyramid notes in [`sdv_simulation_5`](https://github.com/nsengupta/sdv_simulation_5).

**Gateway** owns capture via `ObservationTee` (convert once → `RunWriter` + optional `LiveSink`).
Each run has a versioned `manifest.json` and separate `diagnostic.jsonl` / `ledger.jsonl`
streams. Dashboard consumes the live UDS feed only (apply-before-display). See
[`DESIGN.md`](DESIGN.md) and [`PLAN.md`](PLAN.md) (observation capture, Gateway/Dashboard split,
Zenoh live link).

---

## 5. Decisions recorded

| Date | Decision |
|------|----------|
| 2026-07-15 | Lifecycle on bus via **emulator** (not dashboard → twin direct injection). |
| 2026-07-15 | **Silent ignore** while FSM `Off`. |
| 2026-07-15 | Keep **`tui_dashboard`** crate name. |
| 2026-07-15 | CAN first; **Zenoh/uProtocol deferred** until CAN defect-free. |
| 2026-07-15 | Dashboard **embeds emulator core**; TUI driver buttons control emulator, not twin mailbox. |
| 2026-07-15 | **Replay** consumes stored observation files — standalone dashboard, no live twin. |
| 2026-07-15 | Capabilities agreed **one at a time** before implementation. |
| 2026-07-16 | Canonical external twin input is `TwinIngressEvent`; `VssSignal` remains telemetry-only for future KUKSA path interpretation. |
| 2026-07-16 | `VirtualCarActor` silently drops every non-PowerOn FSM event while `Off`. |
| 2026-07-16 | Emulator session uses a required finite `--readings N`; CSV/echo is deferred. |
| 2026-07-16 | Dashboard lifecycle keys were removed; it observes twin-authored outcomes only. |
| 2026-07-17 | Observation capture stores a separate `manifest.json`, `diagnostic.jsonl`, and `ledger.jsonl`; production run IDs are UUID v4 while tests inject deterministic IDs; transitional Dashboard capture ownership moves to Gateway with the process split. |
| 2026-07-18 | Dashboard presentation reworked (driver/engineer/ledger tail); Gateway↔Dashboard split deferred until process-split work. |
| 2026-07-18 | Twin-authored wall times use a live `UnixTimestamp` (`Duration` since Unix Epoch). Schema v1 stores `{unix_seconds,nanosecond}` objects; summary/UI presentation is `yyyy-mm-dd | HH:mm:ss:nnnnnnnnn (UTC)`. Manifest keeps capture `created_at` and Twin `session_started_at`; Dashboard requires the boot diagnostic before creating a run. |
| 2026-07-19 | Gateway/Dashboard split: Gateway sole twin owner + capture tee; Dashboard UDS consumer; sockets under `<cwd>/tmp/`; detachable `LiveSink`/`LiveSource`. |
| 2026-07-19 | Embedded emulator/TUI driver **cancelled**; replay **TBD next simulation**; Zenoh live link is next carrier work (uProtocol optional). |
| 2026-07-19 | Zenoh live link: both `gateway` and `tui_dashboard` require explicit live-mode flags (no default) to avoid mixed transports. |
| 2026-07-20 | Zenoh roadmap: observation-only Zenoh; peer sessions; one keyexpr; uProtocol and vehicle-bus Zenoh out of scope for that slice. |
| 2026-07-20 | Zenoh design approved: mutually exclusive `--uds` / `--zenoh` / `--no-live` (Gateway); `--uds` / `--zenoh` (Dashboard); required `--keyexpr` with Zenoh; subscriber-wait install gate. |
| 2026-07-20 | Zenoh live link **Done**: `ZenohLiveSink`/`ZenohLiveSource`; exclusive CLI; peer Zenoh; G10 closed. |
| 2026-07-20 | Weather/wiper published on ledger (observation schema versioned); Dashboard glyphs; livedocs `PLAN.md` / `DESIGN.md`. |
---

## 6. Key code locations

| Topic | Path |
|-------|------|
| Observation-only Dashboard | `crates/tui_dashboard/src/main.rs` |
| Gateway binary (capture + optional UDS) | `crates/gateway/src/main.rs` |
| Gateway runtime / builder | `crates/gateway/src/gateway_runtime.rs` |
| CAN reader / dispatch | `crates/gateway/src/gateway_runtime.rs` |
| Twin ingress → FSM projection | `crates/common/src/twin_runtime/connectors/ingress_to_fsm.rs` |
| CAN signal IDs | `crates/common/src/signals.rs` |
| Silent ignore enforcement (CAN lifecycle) | `crates/common/src/twin_runtime/controller/virtual_car_actor.rs` |
| Finite emulator composition | `crates/emulator/src/main.rs`, `crates/emulator/src/runner.rs` |
| Headlamp / wiper actuators | `crates/front_headlamp_actuator/`, `crates/wiper_actuator/` |
| Observation schema, tee, live UDS | `crates/observation/` |
| Two-process smoke | `scripts/smoke-two-process.sh` |

---

*Last updated: 2026-07-19 — Gateway/Dashboard process split.*
