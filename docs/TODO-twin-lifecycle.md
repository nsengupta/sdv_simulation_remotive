# TODO — Twin lifecycle (Install → Start → Operate → Stop → Disband)

Design reference: **`DESIGN.md` §16** (session scope, Dashboard UX, CAN-before-Start noise,
Park-before-Stop, shutdown coordinator).

Each item below is a **compile-ready unit**: merging it should leave `cargo build` / `cargo test`
green. Functionality may be incomplete until later items land.

Shell types live in `crates/gateway/src/twin_lifecycle.rs` — Gateway-only
`ShutdownCoordinator` stub (no Dashboard lifecycle enum).

---

## TL-0 — Gateway shutdown stub

**Status:** Done (minimal).  
**Files:** `crates/gateway/src/twin_lifecycle.rs`, `crates/gateway/src/lib.rs`

- `ShutdownCoordinator` with stub `ensure_stopped_before_exit(&VehicleController)`.
- No process-phase enum; Dashboard does not participate in twin availability.

---

## TL-1 — Split runtime: optional auto-`PowerOn`

**Status:** Done.  
**Goal:** Dashboard install does **not** auto-start the FSM; gateway CI may opt in.

**Files:**

- `crates/gateway/src/gateway_runtime.rs`
- `crates/gateway/src/twin_lifecycle.rs` (optional: link coordinator)

**Changes:**

1. Add `TwinRuntimeBuilder::with_auto_power_on(bool)` (default **`true`** for backward-compatible gateway/CI).
2. Gate the existing `tokio::spawn` + `send_power_on()` in `spawn_runtime` on that flag.
3. Dashboard passes `.with_auto_power_on(false)`.
4. Add unit test: builder flag defaults true; false skips spawn (mock or flag inspection).

**Acceptance:** `cargo test -p gateway` passes; dashboard/gateway binaries build.

---

## TL-2 — Status bar from boot diagnostic (install without Start)

**Status:** Done.  
**Goal:** After install, status line shows session start + T+ from boot diagnostic; panels stay empty until Start.

**Files:**

- `crates/tui_dashboard/src/main.rs`

**Changes:**

1. After `install_controller()`, drain or await first diagnostic before first draw (with short timeout).
2. `format_status_line`: if diagnostic present, show `session_start_unix_nanos` + `elapsed_since_session` even when `latest_transition` is `None`.
3. Panel placeholders: “Press Start to power on” when lifecycle phase is `Installed` (local enum or `TwinLifecycleCoordinator` copy until TL-3).

**Acceptance:** `cargo build -p tui_dashboard`; manual smoke: status bar populated before any ledger row.

---

## TL-3 — Dashboard Start / Stop keys + lifecycle coordinator

**Status:** Done.  
**Goal:** Operator Start sends `PowerOn`; Stop sends `PowerOff`; phase tracked in UI.

**Files:**

- `crates/tui_dashboard/src/main.rs`
- `crates/gateway/src/twin_lifecycle.rs` (use coordinator from gateway crate)

**Changes:**

1. Hold `TwinLifecycleCoordinator` in dashboard main; `after_install()` on startup.
2. Key **`s`** (Start): if `Installed`, `controller.send_power_on().await`, `mark_started()`.
3. Key **`o`** (Stop): if `Started`, `mark_stopping()`, `send_power_off().await`.
4. Empty panels until `Started`; then existing diagnostic/transition panes.
5. Do **not** call `send_power_on` from `spawn_runtime` (requires TL-1).

**Acceptance:** `cargo build -p tui_dashboard`; contract tests unchanged.

---

## TL-4 — Stop via lifecycle key (Dashboard)

**Status:** Done.  
**Goal:** Dashboard observes twin streams only; **`o`** sends `PowerOff` without dashboard-side FSM guards — twin records accept/reject on ledger + diagnostic.

**Implemented:**

1. **`o`** — `send_power_off()` when lifecycle allows; twin is source of truth for outcome.
2. No Dashboard park / RPM / vehicle keys; FSM label and panes from latest twin emissions.
3. `CarSnapshot::is_idle()` retained in common for tests/other callers, not used by dashboard.

---

## TL-5 — Document CAN-before-Start in README

**Status:** Done.  
**Goal:** README paragraph: no accessory-power mode; pre-Start CAN is noise; dashboard lifecycle documented.

**Implemented:**

- [`README.md`](../README.md) — section **Dashboard app and twin lifecycle** (composition, trust model, keys, CAN-before-Start, run order, `q` limitation, gateway vs dashboard).
- Twin brain / ROB background: original Iteration 4 twin (see `$ORIGINAL_TWIN_REPO`).

**Acceptance:** docs only; no code change required.

---

## TL-6 — Gateway shutdown (Ctrl+C → Stop → disband)

**Status:** Not started (stub exists). **Deferred** — see [`PLAN.md`](PLAN.md) / README TBD.  
**Goal:** Gateway signal (Ctrl+C or equivalent) runs Stop if needed, waits for `Off`, tears
down actors and ingress, finishes observation capture. **Dashboard is oblivious** (live link
drops; no twin mailbox from the TUI).

**Files:**

- `crates/gateway/src/twin_lifecycle.rs` — implement `ShutdownCoordinator::ensure_stopped_before_exit`
- `crates/gateway/src/gateway_runtime.rs` / `main.rs` — wire Ctrl+C (or cancel) to the coordinator
- `crates/common/src/twin_runtime/controller/vehicle_controller.rs` — `stop()` / await actor join if missing

**Changes:**

1. On Gateway exit signal: `send_power_off` if FSM not already `Off`; poll until `Off` or timeout.
2. Stop `VirtualCarActor` (join spawn handle); stop CAN reader / dispatch loop; finish tee.
3. Do **not** call this from Dashboard quit.

**Acceptance:** `cargo test -p gateway -p common`; manual: Ctrl+C Gateway while emulator runs leaves a clean observation run and no hung actors.

---

## TL-7 — Disband on FSM `Off` (actor teardown hook)

**Status:** Not started.  
**Goal:** When Stop completes (`Off`), runtime disbands without requiring process exit.

**Files:**

- `crates/common/src/twin_runtime/controller/virtual_car_actor.rs` (optional hook on terminal `Off`)
- `crates/gateway/src/gateway_runtime.rs`

**Changes:**

1. After last `PreparingToStop` commit lands on `Off`, signal Gateway completion (channel or
   callback the `ShutdownCoordinator` can await).
2. TL-6 coordinator waits on this signal instead of polling alone.

**Acceptance:** compile + unit/integration test: power_on_to_idle → power_off → assert actor stopped.

---

## TL-8 — Gateway headless mode flag

**Status:** Not started.  
**Goal:** CLI or builder flag documents two modes from DESIGN §16.5.

**Files:**

- `crates/gateway/src/main.rs`
- `crates/gateway/src/gateway_runtime.rs`

**Changes:**

1. `--auto-power-on` (default true for gateway binary) vs Dashboard manual Start.
2. Document in `README.md` / gateway `--help`.

**Acceptance:** `cargo build -p gateway`; existing e2e tests pass with default auto-start.

---

## Dependency order

```text
TL-0 (done)
  → TL-1 → TL-2 → TL-3 → TL-4 → TL-5 (done)
  → [emulator echo + observation log — explore first]
  → TL-6 → TL-7
TL-8 parallel anytime after TL-1
```

---

## Out of scope (separate docs)

| Item | Doc / capability |
|---|---|
| CAN `0x100` → PowerOn/PowerOff | CAN lifecycle — [`ARCHITECTURE-OVERVIEW.md`](ARCHITECTURE-OVERVIEW.md); history in [`sdv_simulation_5`](https://github.com/nsengupta/sdv_simulation_5) |
| Emulator echo / CSV | Emulator session (CSV deferred) |
| Observation files + replay | Observation capture; replay deferred (TBD next simulation) |
| Engineer ledger heartbeat | TUI engineer / ledger panes (this repo) |
| Gateway process split | Gateway/Dashboard split |
| TL-6 / TL-7 / TL-8 | Shutdown/disband (this file) |
| Target architecture overview | [`ARCHITECTURE-OVERVIEW.md`](ARCHITECTURE-OVERVIEW.md) |
