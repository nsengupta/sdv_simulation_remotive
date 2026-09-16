# RemotiveCar Side-by-Side Observation Demo Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:subagent-driven-development` (recommended) or
> `superpowers:executing-plans` to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Run unchanged RemotiveCar Hello World (3D cute car + Jupyter) beside
the Twin TUI so one Jupyter hazard press is visible on both surfaces, while the
Twin continues to observe only the three Phase II signals.

**Architecture:** Reuse the Phase II bridge/Gateway/Twin observation path
against Hello World’s broker. Remotive owns ECU policy and the 3D surface;
the Twin owns attended TUI presentation (hazard / left / right + bridge RPM
speed). No Twin→Remotive publisher. No RemotiveLabs source edits. No dedicated
Phase III runbook markdown file — ordered multi-shell commands live in this
plan.

**Tech Stack:** Rust 2024, Tokio, Ractor, SocketCAN, Remotive gRPC Rust API,
Ratatui TUI, RemotiveTopology + Docker Compose, Jupyter (Hello World profile).

## Global Constraints

- Work on branch `remotive-integration` from Phase II head `8cc3da4` (or later
  Phase II tip on this branch).
- Do not modify any file under either RemotiveLabs repository.
- Do not overwrite, stage, or reformat existing user-owned untracked files
  (`assets/*`, `docs/REMOTIVE-INTEGRATION.md`, `docs/emulator-CAN-messages.md`,
  `palette.png`, `remotive_rust_digital_twin_integration_boundary.md`).
- Keep the approved design and this plan uncommitted until implementation and
  live acceptance are complete unless the user explicitly asks to commit docs
  earlier.
- Make no significant commit without explicit user approval.
- Bridge subscription must remain exactly these three identities, in order:
  1. `SCCM-DriverCan0:HazardLightButton.HazardLightButton`
  2. `BCM-BodyCan0:TurnLightControl.LeftTurnLightRequest`
  3. `BCM-BodyCan0:TurnLightControl.RightTurnLightRequest`
- Internal carriers stay `0x105` / `0x106` / `0x107`; lifecycle `0x100`, RPM
  `0x102`.
- Do not widen the broker subscription to other Hello World signals.
- Do not add Twin→Remotive actuation (`SetTurnLights` or equivalent).
- Do not create `docs/superpowers/runbooks/*phase-iii*` (or any new Phase III
  runbook markdown).
- Prefer behavior-focused RED-to-GREEN tests before TUI production changes.
- After each task, stop for a review checkpoint; do not commit unless the user
  approved a commit for that checkpoint.

## File Structure

| Path | Responsibility |
|------|----------------|
| `crates/tui_dashboard/src/view/driver.rs` | Attended driver surface: notice, speed, hazard/left/right only |
| `crates/tui_dashboard/src/view/engineer.rs` | Attended engineer surface: SCCM hazard + BCM L/R (no headlamp/wiper/weather rows) |
| `crates/tui_dashboard/src/view/line.rs` | Line roles used by driver/engineer panes |
| `crates/tui_dashboard/src/main.rs` | Live feed wiring; integration-style pane smoke if present |
| `crates/remotive_bridge/src/source.rs` | Exact three-signal subscribe + profile RPM with `--rpm-clamp` |
| `crates/remotive_bridge/src/cli.rs` | Bridge CLI including `--rpm-clamp` (default 1000 / Idle) |
| `crates/remotive_bridge/tests/config_decoder_cli.rs` | Subscription freeze + clamp CLI regression |
| `docs/superpowers/specs/2026-09-14-remotive-car-side-by-side-demo-design.md` | Approved design (read-only for implementers) |
| This plan | Shell orchestration + acceptance (no separate runbook) |

No new Twin observation types, schema version, or Gateway mapping are required
for Phase III unless a live Hello World broker identity mismatch is proven —
then stop and escalate; do not invent alternate namespaces.

---

### Task 1: Lock the attended TUI surface

**Files:**
- Modify (tests first): `crates/tui_dashboard/src/view/driver.rs`
- Modify (tests first): `crates/tui_dashboard/src/view/engineer.rs`
- Modify only if a test forces it: `crates/tui_dashboard/src/view/line.rs`
- Modify only if a test forces it: `crates/tui_dashboard/src/main.rs`

**Interfaces:**
- Consumes: `PublishedTransitionRecord` with `sccm.hazard_button_on`,
  `bcm.left_turn_request_on`, `bcm.right_turn_request_on`,
  `powertrain.speed_kph`; optional `DiagnosticRecord` for Notice.
- Produces: Driver pane text that always includes `Speed:` and the three
  observed labels; never presents attended headlamp/wiper/weather/lux/rain
  rows. Engineer pane continues to expose SCCM hazard + BCM left/right only.

Phase II already removed most legacy driver rows. This task hardens the
contract so Phase III cannot regress while polishing any remaining
unattended presentation (for example Notice text that still surfaces
Rain/Headlamp/Wiper diagnostics as primary demo language).

- [ ] **Step 1: Write the failing attended-surface regression**

In `crates/tui_dashboard/src/view/driver.rs` tests, add (or extend) a test that
asserts the **exact attended label set** after PowerOn-shaped ledger data:

```rust
#[test]
fn driver_attended_labels_are_exactly_speed_hazard_left_right() {
    let mut ledger = sample_ledger(42, 150, PublishedHeadlampState::On);
    ledger.current_ctx.sccm.hazard_button_on = PublishedObservedBool::On;
    ledger.current_ctx.bcm.left_turn_request_on = PublishedObservedBool::On;
    ledger.current_ctx.bcm.right_turn_request_on = PublishedObservedBool::On;
    ledger.current_ctx.weather.raining = true;
    ledger.current_ctx.visibility.ambient_lux = 999;
    ledger.current_ctx.wiper.state = PublishedWiperState::Running;

    let text = pane_text(&driver_pane(None, Some(&ledger), 64));
    assert!(text.contains("Speed:"), "{text}");
    assert!(text.contains("42/"), "{text}");
    assert!(text.contains("Hazard: ON"), "{text}");
    assert!(text.contains("Left request: ON"), "{text}");
    assert!(text.contains("Right request: ON"), "{text}");

    for forbidden in [
        "Visibility:",
        "Headlamps:",
        "Headlamp:",
        "Weather:",
        "Wipers:",
        "Ambient",
        "lux",
        "Raining",
        "Rain ",
    ] {
        assert!(!text.contains(forbidden), "found {forbidden:?} in {text}");
    }
}
```

In `crates/tui_dashboard/src/view/engineer.rs` tests, add:

```rust
#[test]
fn engineer_attended_assemblies_are_only_sccm_hazard_and_bcm_turns() {
    let mut ledger = sample_ledger();
    ledger.current_ctx.sccm.hazard_button_on = PublishedObservedBool::On;
    ledger.current_ctx.bcm.state = PublishedBcmState::Ready;
    ledger.current_ctx.bcm.left_turn_request_on = PublishedObservedBool::On;
    ledger.current_ctx.bcm.right_turn_request_on = PublishedObservedBool::On;
    ledger.current_ctx.visibility.ambient_lux = 999;
    ledger.current_ctx.weather.raining = true;
    ledger.current_ctx.headlamp.state = PublishedHeadlampState::On;
    ledger.current_ctx.wiper.state = PublishedWiperState::Running;

    let text = pane_text(&engineer_pane(Some(&ledger), 64));
    assert!(text.contains("SCCM: Hazard ON"), "{text}");
    assert!(text.contains("BCM: Ready"), "{text}");
    assert!(text.contains("Left ON"), "{text}");
    assert!(text.contains("Right ON"), "{text}");
    for forbidden in ["Headlamp:", "Wiper:", "Weather:", "Visibility:", "lux"] {
        assert!(!text.contains(forbidden), "found {forbidden:?} in {text}");
    }
}
```

Reuse the existing `sample_ledger` / `pane_text` helpers already in that module;
do not invent a second fixture style.

- [ ] **Step 2: Run tests to verify failure or confirm already-green baseline**

```bash
cd /home/nirmalya/Workspace-Rust/Eclipse-SDV/Self-handson-project/sdv_simulation_remotive
cargo test -p tui_dashboard \
  driver_attended_labels_are_exactly_speed_hazard_left_right \
  engineer_attended_assemblies_are_only_sccm_hazard_and_bcm_turns \
  -- --nocapture
```

Expected:

- If labels already match: PASS — keep the tests; proceed to Step 3 only for
  Notice cleanup if Step 4’s Notice test fails.
- If FAIL: implement the minimal pane change in Step 3.

- [ ] **Step 3: Minimal TUI fix (only if Step 2 failed)**

Keep `speed_pane_line` and the three `observed_pane_line` calls. Do not add
headlamp/wiper/weather/lux rows. If Notice still formats
`DiagnosticKind::RainChanged`, `WiperMotionChanged`, or
`HeadlampActuationUnconfirmed` into driver-facing demo text, map those bodies
to a neutral standby string for Phase III attended demos, for example:

```rust
DiagnosticKind::HeadlampActuationUnconfirmed { .. }
| DiagnosticKind::RainChanged { .. }
| DiagnosticKind::WiperMotionChanged { .. } => "(no notice yet)".to_owned(),
```

Prefer that over deleting diagnostic enum variants (schema/runtime may still
emit them).

- [ ] **Step 4: Add Notice non-presentation coverage**

```rust
#[test]
fn driver_notice_does_not_surface_unattended_domain_diagnostics() {
    for kind in [
        DiagnosticKind::RainChanged { raining: true },
        DiagnosticKind::WiperMotionChanged { wiping: true },
        DiagnosticKind::HeadlampActuationUnconfirmed {
            on: true,
            cause: FrontHeadlampIncompleteCause::TimedOut,
        },
    ] {
        let text = pane_text(&driver_pane(Some(&sample_diag(kind)), None, 64));
        assert!(!text.to_lowercase().contains("rain"), "{text}");
        assert!(!text.to_lowercase().contains("wiper"), "{text}");
        assert!(!text.to_lowercase().contains("headlamp"), "{text}");
    }
}
```

Run:

```bash
cargo test -p tui_dashboard -- --nocapture
```

Expected: all `tui_dashboard` tests PASS.

- [ ] **Step 5: Review checkpoint (no commit unless user approves)**

Summarize: attended labels locked; any Notice mapping applied; no Remotive
edits; no bridge subscription change.

---

### Task 2: Freeze the three-signal bridge contract against Hello World reuse

**Files:**
- Modify only if a regression is missing:
  `crates/remotive_bridge/tests/config_decoder_cli.rs`
- Read-only verification (do not edit Remotive trees):
  - `.../remotivelabs-topology-examples/remotive_car/platform/databases/driver_can.dbc`
  - `.../remotivelabs-topology-examples/remotive_car/platform/databases/body_can.dbc`
  - `.../remotivelabs-topology-examples/remotive_car/models/bcm/python/bcm/` (hazard → L/R policy)
- Do **not** modify `crates/remotive_bridge/src/source.rs` unless a live broker
  identity mismatch is proven and the user approves a contract change (out of
  preferred Phase III scope).

**Interfaces:**
- Consumes: existing `subscription_config()` /
  `subscription_ready_status()`.
- Produces: unchanged readiness string listing exactly the three Phase II
  identities.

- [ ] **Step 1: Confirm Hello World still publishes the Phase II identities**

Without editing Remotive files, verify by reading:

- Driver CAN DBC still defines `HazardLightButton.HazardLightButton`.
- Body CAN DBC still defines `LeftTurnLightRequest` /
  `RightTurnLightRequest` on `TurnLightControl`.
- Hello World BCM still mirrors hazard onto both turn requests (same product
  behavior Phase II relied on).

Document the finding in the task review notes. If names differ, stop — do not
quietly remap.

- [ ] **Step 2: Strengthen the subscription freeze test comment + assertion**

Ensure `subscription_config_contains_exactly_three_ordered_signal_ids` and
`subscription_ready_status_proves_connection_and_exact_target` remain present.
If missing an explicit length lock, add:

```rust
#[test]
fn subscription_config_rejects_hello_world_signal_widening() {
    let config = subscription_config();
    let ids = config.signals.unwrap().signal_id;
    assert_eq!(ids.len(), 3, "Phase III must not subscribe extra Hello World signals");
    // Explicitly assert absence of common Hello World distractors by name.
    let names: Vec<&str> = ids.iter().map(|id| id.name.as_str()).collect();
    for distractor in [
        "LowBeam",
        "HighBeam",
        "Brake",
        "TurnStalk",
        "Wiper",
        "Speed",
    ] {
        assert!(
            names.iter().all(|n| !n.contains(distractor)),
            "unexpected distractor {distractor} in {names:?}"
        );
    }
}
```

- [ ] **Step 3: Run bridge contract tests**

```bash
cargo test -p remotive_bridge --test config_decoder_cli -- --nocapture
cargo test -p remotive_bridge --test session_contract -- --nocapture
```

Expected: PASS. Default `--rpm-clamp` is `RPM_DRIVING_THRESHOLD` (1000) so the
Twin stays Idle; raise the flag when the demo needs speed / Driving.

- [ ] **Step 4: Review checkpoint (no commit unless user approves)**

---

### Task 3: Twin-only smoke for attended observation path

**Files:**
- No production file changes expected.
- Optional evidence dir (gitignored): `.superpowers/sdd/phase-iii/`

**Interfaces:**
- Consumes: Gateway on `vcan0` + UDS `observation.sock`; cansend injectors for
  `0x100` / `0x102` / `0x105` / `0x106` / `0x107`.
- Produces: TUI showing speed + hazard/left/right ON without Remotive.

- [ ] **Step 1: Prepare `vcan0`**

```bash
sudo ip link add dev vcan0 type vcan 2>/dev/null || true
sudo ip link set up vcan0
ip link show vcan0
```

Expected: `vcan0` is UP.

- [ ] **Step 2: Start Gateway, then TUI (separate shells)**

Gateway `--uds` binds and waits for the Dashboard client. Do **not** combine
`--uds` with `--print-transitions-only` on this path — ledger-only mode never
binds the socket. (Optional ledger-only smoke without TUI may use
`--print-transitions-only` alone.)

Shell G (Gateway first):

```bash
cd /home/nirmalya/Workspace-Rust/Eclipse-SDV/Self-handson-project/sdv_simulation_remotive
mkdir -p tmp
cargo run -p gateway -- --uds observation.sock --connect-timeout 120
```

Wait until Gateway reports waiting for Dashboard / listening on `vcan0`.

Shell T (TUI):

```bash
cd /home/nirmalya/Workspace-Rust/Eclipse-SDV/Self-handson-project/sdv_simulation_remotive
cargo run -p tui_dashboard -- --uds observation.sock
```

- [ ] **Step 3: Inject lifecycle + three observations**

```bash
# PowerOn
cansend vcan0 100#0100000000000000
# Driving RPM (live bridge default clamp is 1000; raise --rpm-clamp for motion; smoke: any non-zero)
cansend vcan0 102#E803000000000000
# Hazard ON, Left ON, Right ON
cansend vcan0 105#0100
cansend vcan0 106#0100
cansend vcan0 107#0100
```

Expected TUI:

- `Hazard: ON`, `Left request: ON`, `Right request: ON`
- `Speed:` visible (non-missing after RPM)
- No attended Visibility/Headlamp/Weather/Wiper rows

Expected Gateway: observation events / context fields for the three signals;
no `SetTurnLights`.

- [ ] **Step 4: Clean Twin-only processes**

Ctrl+C Gateway and TUI. Confirm:

```bash
pgrep -af 'gateway|tui_dashboard' || true
```

- [ ] **Step 5: Review checkpoint**

---

### Task 4: Live Hello World + cute car + TUI acceptance

**Files:**
- Create evidence only under `.superpowers/sdd/phase-iii/` (gitignored).
- Do **not** create a Phase III runbook markdown file.
- Do **not** edit RemotiveLabs trees.
- Do **not** require a compose override for Hello World alone. The Phase II
  `getting_started-host-bridge.override.yml` exists to avoid colliding with
  Hello World’s control bridge name — leave Hello World on its default compose
  networks. Do **not** run `getting_started` and Hello World concurrently.

**Paths:**

```text
TWIN_ROOT=/home/nirmalya/Workspace-Rust/Eclipse-SDV/Self-handson-project/sdv_simulation_remotive
REMOTIVE_EXAMPLES=/home/nirmalya/Workspace-Rust/Eclipse-SDV/RemotiveLabs/remotivelabs-topology-examples
COMPOSE_DIR=$REMOTIVE_EXAMPLES/remotive_car/build/remotive_car_hello_world
COMPOSE_PROJECT=remotive_car_hello_world
```

**Interfaces:**
- Remotive Hello World broker on host `http://127.0.0.1:50051` (gRPC; do not
  `curl` it).
- 3D car UI: `http://127.0.0.1:3000`
- Jupyter: `http://127.0.0.1:8888`
- Bridge readiness must print the exact three-signal line before Twin
  `PowerOn`.

**RemotiveBus prerequisite (Linux preferred path):** Hello World’s generated
compose uses `driver: remotivebus` for `DriverCan0` / `BodyCan0` / etc. (host
devices such as `vdrivercan0`). If SCCM/BCM brokers log
`Ng.Can … :port_timed_out` or GenServer `{:write, [{106, …}]}` timeouts, that
is RemotiveBus DriverCan0 dying — frame `106` there is DBC
`GearShiftPaddles` on DriverCan, **not** Twin internal `0x106`. Run
[Appendix A: RemotiveBus recovery](#appendix-a-remotivebus-recovery-hello-world)
before Step 3. Do not treat Twin `vcan0` as a substitute for `vdrivercan0`.

- [ ] **Step 1: Prepare `vcan0` (if not already up)**

```bash
sudo ip link add dev vcan0 type vcan 2>/dev/null || true
sudo ip link set up vcan0
```

- [ ] **Step 2: Build unchanged Hello World**

From `$REMOTIVE_EXAMPLES` (common RemotiveTopology root). Prefer Linux +
RemotiveBus when available:

```bash
cd "$REMOTIVE_EXAMPLES"
remotive topology build --no-workspace \
  -f remotive_car/instances/hello_world/main.instance.yaml \
  remotive_car/build
```

If this host needs CAN-over-UDP / VLAN bridge settings instead:

```bash
cd "$REMOTIVE_EXAMPLES"
remotive topology build --no-workspace \
  -f remotive_car/instances/hello_world/main.instance.yaml \
  -f remotive_car/settings/can_over_udp.settings.instance.yaml \
  -f remotive_car/settings/vlan_using_bridge.settings.instance.yaml \
  remotive_car/build
```

Expected: generated compose at
`$COMPOSE_DIR/docker-compose.yml`. Do not hand-edit that file in the Remotive
tree; regenerate if needed.

- [ ] **Step 3: Start Hello World with Jupyter + 3D car profiles**

Shell R (Remotive):

```bash
cd "$COMPOSE_DIR"
docker compose -p "$COMPOSE_PROJECT" -f docker-compose.yml \
  --profile jupyter --profile 3dcar \
  up --build
```

Wait until brokers are healthy and `0.0.0.0:50051` is published. Confirm:

```bash
ss -ltn | grep 50051
ss -ltn | grep 3000
ss -ltn | grep 8888
docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_DIR/docker-compose.yml" ps
```

Do **not** `curl http://127.0.0.1:50051`. gRPC “malformed” / HTTP rejection
lines from healthchecks are not topology failures (same Phase II lesson).

Fresh Restbus state matters: if hazard appears stuck ON before Jupyter, bring
the compose project down and up again so SCCM starts OFF.

- [ ] **Step 4: Open the 3D car**

In a browser: `http://127.0.0.1:3000`

Confirm the cute car UI loads before stimulating hazard.

- [ ] **Step 5: Start Gateway, TUI, then bridge (three Twin shells)**

Do **not** pass `--print-transitions-only` with `--uds` (UDS never binds in
ledger-only mode). Order: Gateway binds first, then TUI connects, then bridge.

Shell G (Gateway):

```bash
cd "$TWIN_ROOT"
mkdir -p tmp
cargo run -p gateway -- --uds observation.sock --connect-timeout 120
```

Wait for Gateway waiting-for-Dashboard / `vcan0` listen.

Shell T (TUI):

```bash
cd "$TWIN_ROOT"
cargo run -p tui_dashboard -- --uds observation.sock
```

Shell B (bridge):

```bash
cd "$TWIN_ROOT"
cargo run -p remotive_bridge -- \
  --broker-url http://127.0.0.1:50051 \
  --can-interface vcan0
# Optional: --rpm-clamp 3000 for Twin speed / Driving (default 1000 keeps Idle)
```

Capture bridge stderr. Before any Twin `PowerOn` ledger row, require:

```text
[remotive_bridge] connected; subscribed signals=SCCM-DriverCan0:HazardLightButton.HazardLightButton,BCM-BodyCan0:TurnLightControl.LeftTurnLightRequest,BCM-BodyCan0:TurnLightControl.RightTurnLightRequest
```

Exactly those three. Then bridge emits lifecycle/`PowerOn` and RPM capped by
`--rpm-clamp` (default 1000).

Optional: `candump -L vcan0` in a spare shell for carrier IDs only.

- [ ] **Step 6: Jupyter hazard ON**

Open `http://127.0.0.1:8888`, open the Hello World notebook, and press hazard
ON (use the notebook’s existing hazard control — do not edit the notebook in
the Remotive tree).

Expected together:

1. 3D car shows hazard / turn-indicator effect.
2. TUI shows `Hazard: ON`, `Left request: ON`, `Right request: ON`.
3. TUI still shows `Speed:` from bridge RPM.
4. Gateway ledger/context has no `SetTurnLights` Twin actuation.
5. Bridge subscription list remains three signals only.

Save evidence under `.superpowers/sdd/phase-iii/` (screenshots or notes +
bridge readiness snippet + Gateway excerpt). Do not commit binary media unless
the user asks.

- [ ] **Step 7: Jupyter hazard OFF (optional clarity)**

Press hazard OFF. Expect TUI to move hazard/left/right toward OFF once Remotive
BCM clears turn requests (allow Restbus timing). Cute car should clear the
hazard effect.

- [ ] **Step 8: Controlled shutdown**

1. SIGINT bridge (Ctrl+C) → expect `RPM=0` then `PowerOff`; Gateway to `Off`.
2. Ctrl+C Gateway and TUI (and candump if used). Confirm Twin host processes
   are gone:

```bash
pgrep -af 'gateway|remotive_bridge|tui_dashboard|candump' || true
# Expected: empty
```

3. Tear down **only** this compose project (**keep** `remotivebusd` running
   until compose is confirmed down so RemotiveBus can delete networks cleanly):

```bash
cd "$COMPOSE_DIR"
docker compose -p "$COMPOSE_PROJECT" -f docker-compose.yml \
  --profile jupyter --profile 3dcar \
  down --remove-orphans
```

Do **not** delete unrelated Docker networks (for example leftover
`getting_started` / `gs_local_ctl` resources).

Confirm compose side is down:

```bash
docker ps --filter name=remotive_car_hello_world
# Expected: empty / header only
ip link show vdrivercan0 2>&1 || true
# Expected: device does not exist
```

4. **Full session end — tear down RemotiveBus** (after compose is empty only):

```bash
sudo systemctl stop remotivebusd
systemctl is-active remotivebusd || true
# Expected: inactive (dead)
ls /run/docker/plugins/remotivebus.sock 2>&1 || true
# Expected: No such file or directory
```

If you plan to restart Hello World soon in the same session, you may leave
`remotivebusd` running and skip step 4; for a complete application shutdown,
always stop RemotiveBus.

- [ ] **Step 9: Acceptance checklist (must all pass)**

1. Remotive Hello World source tree unmodified (generated `build/` outputs may
   change and stay in the Remotive tree).
2. Bridge readiness listed exactly the three Phase II identities before
   `PowerOn`.
3. Jupyter hazard ON → cute car effect + TUI hazard/left/right ON.
4. No Twin `SetTurnLights` (or equivalent) into Remotive.
5. TUI did not present headlamp/wiper/weather/lux/rain as attended
   observations; speed remained visible.
6. Shutdown left no Phase III–owned Twin processes / this compose project
   leftovers; for a full session end, `remotivebusd` is stopped as in Step 8.4.

- [ ] **Step 10: Review checkpoint — ask user before any commit**

---

### Task 5: Final verification gate

**Files:** none required beyond prior tasks.

- [ ] **Step 1: Run focused automated gates**

```bash
cd /home/nirmalya/Workspace-Rust/Eclipse-SDV/Self-handson-project/sdv_simulation_remotive
cargo test -p tui_dashboard
cargo test -p remotive_bridge --test config_decoder_cli
cargo test -p remotive_bridge --test session_contract
```

Expected: PASS.

- [ ] **Step 2: Confirm out-of-scope items stayed deferred**

- No ECU/tunnel-generated visibility work.
- No Twin→Remotive hazard actuation.
- No observation expansion to beams/brake/stalk/etc.
- No move of continuous stimulus ownership into Remotive beyond Jupyter.
- No new Phase III runbook markdown file.

- [ ] **Step 3: Offer commit only if user asks**

Suggested commit message (do not run until approved):

```text
feat: side-by-side RemotiveCar Hello World and Twin observation demo

Lock attended TUI surface and reuse the three-signal bridge against
unchanged Hello World so Jupyter hazard is visible on the 3D car and Twin.
```

Include only Twin-owned paths touched by Tasks 1–2 (and this plan/spec if the
user wants docs in the same commit). Leave user-owned untracked assets alone.

---

## Appendix A: RemotiveBus recovery (Hello World)

Use this when Hello World is built for **Linux + RemotiveBus** (compose networks
`DriverCan0` / `BodyCan0` / … with `driver: remotivebus`) and brokers show
`Ng.Can … :port_timed_out` or GenServer write timeouts on cyclic DriverCan
frames (e.g. ID `106` = `GearShiftPaddles`). This recovers host RemotiveBus
endpoints; it does **not** edit RemotiveLabs source trees. Twin `vcan0` stays
separate for Gateway/bridge.

**Paths (same as Task 4):**

```text
REMOTIVE_EXAMPLES=/home/nirmalya/Workspace-Rust/Eclipse-SDV/RemotiveLabs/remotivelabs-topology-examples
COMPOSE_DIR=$REMOTIVE_EXAMPLES/remotive_car/build/remotive_car_hello_world
COMPOSE_PROJECT=remotive_car_hello_world
```

### A1. Stop Twin processes (if any)

In each Twin shell (`remotive_bridge`, `gateway`, `tui_dashboard`, optional
`candump`): `Ctrl+C`. Confirm:

```bash
pgrep -af 'gateway|remotive_bridge|tui_dashboard|candump' || true
# Expected: no matching Twin host processes (empty output)
```

That `pgrep` proves only that **Twin host processes** are gone. It does **not**
prove Hello World compose is down — Docker containers and remotivebus networks
are a separate check in A2.

### A2. Tear down only Hello World compose

Keep `remotivebusd` **running** during this step so Docker can call
RemotiveBus `DeleteNetwork` / destroy `vdrivercan0` (and siblings) cleanly.
Stopping RemotiveBus first often leaves orphan remotivebus networks or a hung
`compose down`.

```bash
cd "$COMPOSE_DIR"
docker compose -p "$COMPOSE_PROJECT" -f docker-compose.yml \
  --profile jupyter --profile 3dcar \
  down --remove-orphans
```

Do **not** prune unrelated Docker networks. Confirm compose side is down:

```bash
docker ps --filter name=remotive_car_hello_world
# Expected: empty / header only (no containers)

docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_DIR/docker-compose.yml" ps
# Expected: no running services

ip link show vdrivercan0 2>&1 || true
# Expected: "Device … does not exist" for vdrivercan0
```

**Gate before A3:** only when `docker ps --filter name=remotive_car_hello_world`
shows **no containers** is the compose side down. Then tear down RemotiveBus
(A3). Do not stop `remotivebusd` while Hello World containers are still up.

Do **not** run `getting_started` (or any other RemotiveBus topology) while
recovering or re-running Hello World.

### A3. Tear down then restart RemotiveBus

After A2 confirms empty/no Hello World containers (compose side down), stop
the daemon (tear down), then start it fresh so CAN/VLAN endpoint state is not
carried over from a long-lived process:

```bash
sudo systemctl stop remotivebusd
systemctl is-active remotivebusd || true
# Expected: inactive (dead)

sudo systemctl start remotivebusd
systemctl is-active remotivebusd
# Expected: active
```

(`systemctl restart remotivebusd` is equivalent once compose is already down;
the stop/start split makes the tear-down explicit.)

Confirm the Docker network-driver socket exists:

```bash
ls -la /run/docker/plugins/remotivebus.sock
# Expected: socket file present
```

Optional kernel modules (RemotiveBus unit already requires these at start):

```bash
lsmod | grep -E 'vxcan|can_gw|vcan' || true
```
### A4. Rebuild Hello World for RemotiveBus (no UDP fallback)

Regenerate compose so networks still use `driver: remotivebus` (not
`can_over_udp` unless you deliberately choose the fallback later):

```bash
cd "$REMOTIVE_EXAMPLES"
remotive topology build --no-workspace \
  -f remotive_car/instances/hello_world/main.instance.yaml \
  remotive_car/build
```

Sanity-check generated networks:

```bash
grep -A6 '^  DriverCan0:' "$COMPOSE_DIR/docker-compose.yml"
# Expected: driver: remotivebus and host_device: vdrivercan0
```

### A5. Bring Hello World up again

```bash
cd "$COMPOSE_DIR"
docker compose -p "$COMPOSE_PROJECT" -f docker-compose.yml \
  --profile jupyter --profile 3dcar \
  up --build
```

In another shell, after containers settle:

```bash
ss -ltn | grep -E '50051|3000|8888'
ip link show vdrivercan0
ip link show vbodycan0
docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_DIR/docker-compose.yml" ps
```

Expected: ports published; `vdrivercan0` / `vbodycan0` exist and are UP;
SCCM/BCM broker logs do **not** repeat `port_timed_out`. Do **not**
`curl http://127.0.0.1:50051`.

### A6. Resume Phase III Twin sequence

Continue Task 4 from **Step 4** (open 3D car) through Gateway → TUI → bridge →
Jupyter hazard. Twin `vcan0` (Task 4 Step 1) is still required for the bridge.

### A7. If RemotiveBus recovery still fails

Only then use the CAN-over-UDP + VLAN bridge rebuild from Task 4 Step 2
(second command block). That avoids host `vdrivercan0`; regenerate and
re-up; do not mix a RemotiveBus-built compose with a half-applied UDP
settings tree — rebuild cleanly, then `down` / `up`.

### A8. Full session shutdown (stop RemotiveBus)

When ending the demo for the session (not mid-recovery restart), after Task 4
Step 8 items 1–3 confirm Twin processes gone and Hello World compose empty,
stop RemotiveBus:

```bash
sudo systemctl stop remotivebusd
systemctl is-active remotivebusd || true
# Expected: inactive (dead)
ls /run/docker/plugins/remotivebus.sock 2>&1 || true
# Expected: No such file or directory while stopped
```

Do **not** stop `remotivebusd` while Hello World containers are still up.
Order is always: Twin → compose down → RemotiveBus stop.

---

## Self-Review (plan vs spec)

| Spec requirement | Task |
|------------------|------|
| Side-by-side Hello World 3D car + Twin TUI | Task 4 |
| Jupyter hazard stimulus (this phase only) | Task 4 Step 6 |
| Exactly three Phase II bridge identities | Tasks 2, 4 |
| No RemotiveLabs source edits | Global + Task 4 |
| No Twin→Remotive publisher | Tasks 3–4 acceptance |
| TUI: hazard/left/right + speed; hide unattended domains | Tasks 1, 3, 4 |
| Ordered shells; no dedicated runbook MD | Task 4 (commands in plan) |
| Controlled shutdown; don’t delete unrelated networks | Task 4 Step 8 |
| Full session end stops RemotiveBus after compose down | Task 4 Step 8.4 / Appendix A8 |
| RemotiveBus recovery for `port_timed_out` / DriverCan | Appendix A |
| Deferred items remain deferred | Task 5 Step 2 |

Placeholder scan: no TBD/TODO implement-later steps; commands and test code
are concrete. Type/name consistency: Phase II signal strings and TUI labels
(`Hazard:`, `Left request:`, `Right request:`, `Speed:`) match existing
code and the approved design.
