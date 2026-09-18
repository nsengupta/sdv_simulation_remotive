# Design — SDV Simulation 5

**Livedoc** for process/observation/Dashboard decisions in this simulation.
Remotive Hello World observation design and runbook:
[`DESIGN-remotive-observation.md`](DESIGN-remotive-observation.md).
Topology and run order: [`ARCHITECTURE-OVERVIEW.md`](ARCHITECTURE-OVERVIEW.md).
Roadmap + deferred TBDs: [`PLAN.md`](PLAN.md).
Catalogue: [`design-documents.md`](design-documents.md).

## Problem this simulation solves

Iteration 4 proved multi-assembly coordination inside one process. Simulation 5 makes the
prototype **operable as separate processes** with a durable observation trail:

1. **Gateway** owns the twin, CAN ingress, actuators, and capture.
2. **Dashboard** is observation-only (no twin mailbox injection).
3. **Observation** is versioned JSON (files + live UDS or peer Zenoh).
4. **Emulator** drives finite or Ctrl+C sessions on CAN.

## Core decisions

### Processes and live link

- Gateway is the sole twin owner; Dashboard never injects lifecycle.
- Live modes are **mutually exclusive** and **required** on each binary:
  - Gateway: `--uds PATH` | `--zenoh --keyexpr EXPR` | `--no-live`
  - Dashboard: `--uds PATH` | `--zenoh --keyexpr EXPR` (no `--no-live`)
- UDS paths resolve under `<cwd>/tmp/`. Zenoh day-one: **peer** sessions, one keyexpr,
  NDJSON payloads compatible with file envelopes. Vehicle bus stays **CAN**.
- Install gate: wait for UDS accept **or** first Zenoh matching subscriber (`--connect-timeout`).
- File tee (`ObservationTee` → `RunWriter`) runs whenever the twin runs, independent of live mode.

### Observation streams

- Two Twin-authored streams: **diagnostics** (facts) and **transition ledger** (hops + context).
- **Wide streams, selective collectors** — emit rich facts; UI filters. Do not thin the Twin
  to one pane’s needs.
- **Common published structs are source of truth** — observation JSON mirrors published Rust
  types field-for-field (observation schema is versioned; currently **v8** on the Remotive path).
- Presentation (glyphs, colours, labels) is **receiver-side** only.

### Weather and wiper on Dashboard

- Durable `WeatherContext { raining }` on live `VehicleContext`; set in `zone_turn` on rain edges.
- Publish full wiper state (`Off` / `Ready` / `Running`) and weather on ledger `current_ctx`.
- Publish real `RainsStarted` / `RainsStopped` ledger events (not `TimerTick` aliases).
- Keep edge diagnostics `RainChanged` / `WiperMotionChanged`.
- Driver: glyphs + text (`☀ Sunny` / `☁ Raining`, `x stopped` / `≋ moving`); cyan **labels**;
  blank spacers between segments; two blank lines above Notice.
- Engineer: state, last event, Headlamp + Wiper context — **no** Weather line, **no** Active ROB line.

### Explicitly deferred

See [`PLAN.md`](PLAN.md) § Important missing TBDs (Active ROB turns, coloured `Swatch`, Notice
colours, glyph ASCII/animation, richer assemblies, zone-encased headlamp unconfirmed,
shutdown/disband).

## Crates (truth of the tree)

| Crate | Role |
|-------|------|
| `common` | Twin, FSM, published observation records, facade |
| `observation` | Schema DTOs, file writer/reader, UDS/Zenoh live sink/source, tee |
| `gateway` | Twin owner, CAN, capture tee, optional live publish |
| `tui_dashboard` | Observation TUI consumer |
| `emulator` | CAN lifecycle + telemetry producer (`--readings`, `--tick-ms`) |
| `front_headlamp_actuator` / `wiper_actuator` | Hardware-facing actuators on CAN |

## Related documents

- Remotive path: [`DESIGN-remotive-observation.md`](DESIGN-remotive-observation.md)
- Catalogue: [`design-documents.md`](design-documents.md)
