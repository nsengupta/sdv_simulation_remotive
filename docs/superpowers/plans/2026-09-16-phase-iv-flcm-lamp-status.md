# Phase IV FLCM Lamp Status Feedback Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:subagent-driven-development` (recommended) or
> `superpowers:executing-plans` to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add FLCM-authored front low-beam **Status** + cyclic TX on a Remotive
PR branch, observe it in the Twin, and raise a TUI **Warning** on Fail or
silence while the cute car stays ON from BCM *requests*.

**Architecture:** Remotive DBC + tiny FLCM stub publish
`FLCM-BodyCan0:LowBeamLightStatus.*`. BCM and `3d_car_mapping.yaml` ignore that
feedback. `remotive_bridge` forwards Status on Twin carriers `0x108`/`0x109`.
Twin `FlcmContext` + silence timer emit `DiagnosticKind::FlcmLampFault`; TUI
Notice shows Warning; attended low-beam status lines bind to FLCM, not BCM
requests.

**Tech Stack:** RemotiveTopology + Python behavioral stub, DBC, Jupyter control
API; Rust 2024 Twin (`common` / `observation` / `remotive_bridge` / `gateway` /
`tui_dashboard`).

## Global Constraints

- Spec:
  `docs/superpowers/specs/2026-09-16-phase-iv-flcm-liveness-design.md`.
- **Remotive** edits only on branch `demo/flcm-lamp-status-feedback` under
  `RemotiveLabs/remotivelabs-topology-examples` (or the operator’s local clone
  path). Do not mix unrelated Remotive commits on that branch.
- **Twin** edits on `remotive-integration`.
- Do not modify BCM `BeamsStateMachine` or `common/3d_car/3d_car_mapping.yaml`.
- Do not teach BCM/GWM to consume FLCM Status in this phase.
- Do not add Twin→Remotive actuation.
- Do not reuse Twin→actuator `FrontHeadlampActuationIncomplete` for this story.
- Silence threshold **T = 500 ms** (≈10× 50 ms cycle).
- Observation schema bump to **7** when emitting FLCM fields (v1–v6 remain
  readable; missing FLCM deserializes as Unknown / not-silent). The post-review fix
  pass added the two FLCM observation *events* to the ledger, which a v7 reader
  cannot parse, so the shipped version is **8** (v1–v7 remain readable).
- The silence watchdog is armed by the **first observed FLCM status**, never by
  PowerOn: a topology with no FLCM TX stays Unknown / not-silent, so Phase I–III
  demos and emulator-only runs raise no FLCM Warning.
- Prefer RED→GREEN tests before production changes.
- Make no significant Twin commit without explicit user approval.
- Do not overwrite user-owned untracked junk (`assets/*`, `palette.png`, etc.).

## Locked identities

### Remotive BodyCan (new)

| Item | Value |
|------|--------|
| Frame | `LowBeamLightStatus` |
| CAN ID (DBC `BO_`) | `111` |
| Sender | `FLCM` |
| Cycle | `GenMsgCycleTime` **50** ms |
| Signals | `LeftLowBeamLightStatus`, `RightLowBeamLightStatus` (1 bit each) |
| Encoding | `0` = Ok, `1` = Fail |
| Broker namespace | `FLCM-BodyCan0` |
| Full signal names | `LowBeamLightStatus.LeftLowBeamLightStatus`, `LowBeamLightStatus.RightLowBeamLightStatus` |
| Fault control | `ControlRequest` type `flcm_fault`, argument `ok` \| `fail` \| `silent` |

### Twin internal carriers (new)

| Signal | CAN ID | Payload |
|--------|--------|---------|
| Left low-beam status | `0x108` | `[1,0]` = Ok, `[0,0]` = Fail (same 2-byte style as `0x105`) |
| Right low-beam status | `0x109` | same |

Phase II carriers `0x105`/`0x106`/`0x107` unchanged.

### Bridge subscribe order (final)

1. `SCCM-DriverCan0:HazardLightButton.HazardLightButton`
2. `BCM-BodyCan0:TurnLightControl.LeftTurnLightRequest`
3. `BCM-BodyCan0:TurnLightControl.RightTurnLightRequest`
4. `FLCM-BodyCan0:LowBeamLightStatus.LeftLowBeamLightStatus`
5. `FLCM-BodyCan0:LowBeamLightStatus.RightLowBeamLightStatus`

---

## File Structure

| Path | Responsibility |
|------|----------------|
| Remotive `platform/databases/body_can.dbc` | FLCM `LowBeamLightStatus` frame |
| Remotive `models/flcm/python/flcm/__main__.py` | Tiny FLCM stub + restbus + `flcm_fault` |
| Remotive `models/flcm.bm.instance.yaml` | Container model wiring |
| Remotive `instances/hello_world/main.instance.yaml` | `FLCM` → behavioral include (not `{}`) |
| Remotive Jupyter / demo cell notes | Documented in Twin plan Task 8 |
| Twin `crates/common/src/signals.rs` | `0x108`/`0x109` + `ObservedEcuSignal` variants |
| Twin `crates/common/src/vehicle_state/flcm.rs` | `FlcmContext` + silence bookkeeping helpers |
| Twin `crates/common/src/twin_runtime/` | FLCM apply path + silence Warning emit |
| Twin `crates/common/src/observation_records/` | Published FLCM + `DiagnosticKind::FlcmLampFault` |
| Twin `crates/observation/` | Schema v7, then v8 for the FLCM observation events |
| Twin `crates/remotive_bridge/` | Subscribe + decode + session write |
| Twin `crates/gateway/` | Ingress map `0x108`/`0x109` |
| Twin `crates/tui_dashboard/` | Notice Warning + low-beam status lines |
| Twin `docs/superpowers/plans/2026-09-16-phase-iv-flcm-lamp-status.md` | This plan + live run steps |

---

### Task 1: Remotive — DBC + FLCM stub + branch

**Repos:** RemotiveLabs topology-examples only (not Twin).

**Files:**
- Create branch: `demo/flcm-lamp-status-feedback` from Hello World tip
- Modify: `remotive_car/platform/databases/body_can.dbc`
- Create: `remotive_car/models/flcm/python/flcm/__main__.py`
- Create: `remotive_car/models/flcm/python/flcm/__init__.py` (empty)
- Create: `remotive_car/models/flcm/python/flcm/log.py` (copy pattern from `rlcm/log.py`)
- Create: `remotive_car/models/flcm.bm.instance.yaml`
- Modify: `remotive_car/instances/hello_world/main.instance.yaml`

**Interfaces:**
- Produces broker signals listed in **Locked identities**
- Control: `flcm_fault` / `ok|fail|silent`

- [x] **Step 1: Create Remotive branch**

```bash
cd /home/nirmalya/Workspace-Rust/Eclipse-SDV/RemotiveLabs/remotivelabs-topology-examples
git checkout -b demo/flcm-lamp-status-feedback
```

Expected: on new branch.

- [x] **Step 2: Extend `body_can.dbc`**

After existing `BO_` blocks (before `VAL_` section), add:

```text
BO_ 111 LowBeamLightStatus: 1 FLCM
  SG_ LeftLowBeamLightStatus : 0|1@1+ (1,0) [0|1] ""  BCM, DIM
  SG_ RightLowBeamLightStatus : 1|1@1+ (1,0) [0|1] ""  BCM, DIM
```

Add `VAL_` entries Ok/Fail and:

```text
BA_ "GenSigStartValue" SG_ 111 LeftLowBeamLightStatus 0;
BA_ "GenSigStartValue" SG_ 111 RightLowBeamLightStatus 0;
BA_ "GenMsgCycleTime" BO_ 111 50;
```

- [x] **Step 3: Minimal FLCM behavioral stub**

Create `models/flcm/python/flcm/__main__.py` modeled on BCM restbus + control
handler (keep under ~120 lines). Required behavior:

```python
# Pseudocode locked for implementers:
# - CanNamespace("FLCM-BodyCan0", restbus SenderFilter(ecu_name="FLCM"))
# - Subscribe BodyCan LowBeamLightControl (optional mirror); default Status=Ok
# - Every restbus cycle / on timer: update_signals Left/Right status unless silent
# - control_handlers: ("flcm_fault", on_flcm_fault)
#   ok     -> mode=normal, status=0/0, restbus running
#   fail   -> mode=fail, status=1/1, restbus still TX
#   silent -> mode=silent, stop/pause FLCM restbus TX for LowBeamLightStatus
```

Prefer `restbus.update_signals` + `restbus.reset()` patterns from
`models/bcm/python/bcm/__main__.py`. If pause API is unclear, spike with
broker docs; fallback: set an internal flag and skip `update_signals` while
silent (restbus may still send start values — prefer explicit restbus stop if
available).

- [x] **Step 4: Wire instance**

`models/flcm.bm.instance.yaml` (mirror `bcm.bm.instance.yaml` with
`MODEL_PATH=flcm/python`, `python -m flcm`).

In `instances/hello_world/main.instance.yaml`:

- include `../../models/flcm.bm.instance.yaml` (or equivalent path)
- change `FLCM: {}` to use the behavioral model entry consistent with other
  BM ECUs (follow how BCM is pulled in via `bcm_gwm_ihu.instance.yaml` —
  either add FLCM to a small `flcm` include used by hello_world, or set
  `FLCM: { models: ... }` per Remotive instance schema used by sibling ECUs)

- [ ] **Step 5: Smoke Remotive alone** — **BLOCKED** (`remotivebusd` inactive; see Task 8)

Build/start Hello World on this branch (same compose profile as Phase III).
In Jupyter or a tiny script: set low beams ON; confirm
`FLCM-BodyCan0` / `LowBeamLightStatus` cycles with Ok; send `flcm_fault=fail`
then `silent`; confirm frames change / stop; cute car low beams still ON.

- [x] **Step 6: Commit on Remotive branch only**

```bash
git add platform/databases/body_can.dbc models/flcm models/flcm.bm.instance.yaml \
  instances/hello_world/main.instance.yaml
git commit -m "$(cat <<'EOF'
feat(flcm): publish low-beam status with demo fault inject

Add FLCM-authored LowBeamLightStatus and flcm_fault control for Twin liveness demos.
EOF
)"
```

---

### Task 2: Twin carriers `0x108` / `0x109`

**Files:**
- Modify: `crates/common/src/signals.rs`
- Modify: `crates/common/src/test/remotive_observed_signal_contract.rs`
- Modify: `crates/gateway/src/ingress/mapping.rs`

**Interfaces:**
- Produces:

```rust
pub const ID_LEFT_LOW_BEAM_STATUS: u16 = 0x108;
pub const ID_RIGHT_LOW_BEAM_STATUS: u16 = 0x109;

pub enum ObservedEcuSignal {
    // existing...
    LeftLowBeamStatus(bool),  // true = Ok, false = Fail
    RightLowBeamStatus(bool),
}
```

- [x] **Step 1: Failing round-trip tests**

Extend `remotive_observed_signal_contract.rs` with Ok/Fail encode/decode for
both IDs (mirror hazard tests).

- [x] **Step 2: Run tests — expect FAIL**

```bash
cargo test -p common --test remotive_observed_signal_contract -- --nocapture
```

- [x] **Step 3: Implement signal encode/decode + gateway mapping**

Update `ObservedEcuSignal::{from_can_frame,to_can_frame}` and gateway
`ingress/mapping.rs` unit tests that list `0x105..=0x107` to include
`0x108`/`0x109`.

- [x] **Step 4: Run tests — expect PASS**

```bash
cargo test -p common --test remotive_observed_signal_contract
cargo test -p gateway --lib ingress::mapping
```

- [x] **Step 5: Commit (after user approval)**

```bash
git add crates/common/src/signals.rs \
  crates/common/src/test/remotive_observed_signal_contract.rs \
  crates/gateway/src/ingress/mapping.rs
git commit -m "feat: add Twin carriers for FLCM low-beam status"
```

---

### Task 3: `FlcmContext` + silence Warning

**Files:**
- Create: `crates/common/src/vehicle_state/flcm.rs`
- Modify: `crates/common/src/vehicle_state/mod.rs` (`VehicleContext.flcm`)
- Create: `crates/common/src/test/flcm_observation_contract.rs`
- Modify: `crates/common/src/observation_records/diagnostic/mod.rs`
- Modify: twin runtime — add a small **`FlcmActor`** mirroring `BcmActor` /
  `SccmActor` (same tell-back style); do not invent a one-off ingress path
- Wire `TimerTick` / deadline so after **500 ms** without status update while
  powered, emit Warning diagnostic once until cleared

**Interfaces:**
- Produces:

```rust
pub struct FlcmContext {
    pub left_low_beam_status: ObservedBool,  // On=Ok, Off=Fail, Unknown=none/silent
    pub right_low_beam_status: ObservedBool,
    pub silent: bool,                        // true when watchdog expired
}

pub enum DiagnosticKind {
    // existing...
    FlcmLampFault {
        silent: bool,
        left_fail: bool,
        right_fail: bool,
    },
}
```

Warning when `silent || left_fail || right_fail` (`Fail` ≡ `ObservedBool::Off`
after an observation). Clear when both sides Ok and not silent.

PowerOn / BecomeOn: statuses Unknown, silent false. BecomeOff: reset.

- [x] **Step 1: Write failing pure contracts**

Tests: default Unknown; Ok observation; Fail sets Off; silence flag helper;
diagnostic formatting includes `flcm` / `silent` / `fail` facts (ASCII, no UI
chrome).

- [x] **Step 2: Implement context + diagnostic variant + runtime wiring**

Emit `DiagnosticRecord::warning(..., DiagnosticKind::FlcmLampFault { .. })`
on transition into fault; emit a clearing Info/Text or rely on Notice
replacement when healthy again — match how other warnings are cleared on TUI
(driver Notice shows latest eligible diagnostic).

- [x] **Step 3: Tests PASS**

```bash
cargo test -p common --test flcm_observation_contract
cargo test -p common diagnostic
```

- [x] **Step 4: Commit (after user approval)**

```bash
git commit -m "feat: FlcmContext liveness and FlcmLampFault diagnostic"
```

---

### Task 4: Observation schema v7 + publish facade

**Files:**
- Modify: `crates/observation/src/schema/mod.rs` → `CURRENT_SCHEMA_VERSION = 7`
- Modify: `crates/observation/src/schema/v1.rs` — `FlcmContextV1` with
  `left_low_beam_status_ok`, `right_low_beam_status_ok` as tri-state enums
  consistent with SCCM/BCM v5+ fields; `silent: bool`
- Modify: `crates/common/.../transition/mod.rs` — `PublishedFlcmContext`
- Modify: observation golden / compatibility / round_trip / support tests
- Add golden `testdata/golden/v7/...` if required by `golden_files` tests

**Interfaces:**
- Missing FLCM object on old ledgers → Unknown / `silent=false`
- New emits include `flcm` object

- [x] **Step 1: Failing schema compatibility tests for v7**

- [x] **Step 2: Implement + regenerate goldens**

- [x] **Step 3:**

```bash
cargo test -p observation
```

Expected: PASS

- [x] **Step 4: Commit (after user approval)**

```bash
git commit -m "feat(observation): schema v7 FLCM low-beam status"
```

- [x] **Step 5 (post-review): schema v8 for the FLCM observation events**

The final review found FLCM observations published as `TimerTick`, making them
indistinguishable from ticks in the ledger. `PublishedFsmEvent` /
`FsmEventV1::{LeftLowBeamStatusObserved, RightLowBeamStatusObserved}` were added
(`{"type":"left_low_beam_status_observed","ok":true}`), which a v7 reader cannot
parse — hence `CURRENT_SCHEMA_VERSION = 8` plus a v8 golden run and a v7 golden
read-back test. `AssemblyZoneReady` (which carries the silence verdict) stays an
unpublished `TimerTick` placeholder; a silence row is identified by
`current_ctx.flcm.silent`.

---

### Task 5: `remotive_bridge` subscribe + session

**Files:**
- Modify: `crates/remotive_bridge/src/source.rs`
- Modify: `crates/remotive_bridge/src/session.rs`
- Modify: `crates/remotive_bridge/tests/config_decoder_cli.rs`
- Modify: `crates/remotive_bridge/tests/session_contract.rs` if present

**Interfaces:**
- `BrokerObservation::{LeftLowBeamStatus(bool), RightLowBeamStatus(bool)}`
- Decode DBC `0`/`1` integers like existing boolean decoder (confirm payload
  type in spike; extend decoder if integer-only)
- Readiness string lists all **five** identities in locked order

- [x] **Step 1: Failing subscription tests (exactly five ordered IDs)**

Replace Phase III “rejects widening” test: still reject **unlisted** Hello
World distractors, but allow the two FLCM status signals.

- [x] **Step 2: Implement subscribe/decode/session writes to `0x108`/`0x109`**

- [x] **Step 3:**

```bash
cargo test -p remotive_bridge --test config_decoder_cli
cargo test -p remotive_bridge --test session_contract
```

- [x] **Step 4: Commit (after user approval)**

```bash
git commit -m "feat(remotive_bridge): observe FLCM low-beam status"
```

---

### Task 6: TUI Warning + attended low-beam status

**Files:**
- Modify: `crates/tui_dashboard/src/view/driver.rs`
- Modify: `crates/tui_dashboard/src/view/engineer.rs`
- Modify: `crates/tui_dashboard/src/main.rs` if notice filter needs
  `FlcmLampFault`

**Interfaces:**
- Driver Notice: eligible for `DiagnosticKind::FlcmLampFault` at Warning level
  (body must include fault facts; e.g. `FLCM low beam fault silent=...`)
- Driver/engineer: show `Low beam L:` / `Low beam R:` from
  `PublishedFlcmContext` as OK/FAIL/UNKNOWN — **not** legacy headlamp actuator
  state, **not** BCM request lines
- Keep Phase III rule: do not resurrect weather/wiper/lux as attended rows

- [x] **Step 1: Failing view tests**

- [x] **Step 2: Implement**

- [x] **Step 3:**

```bash
cargo test -p tui_dashboard
```

- [x] **Step 4: Commit (after user approval)**

```bash
git commit -m "feat(tui): show FLCM low-beam status and Warning"
```

---

### Task 7: Gateway assembly e2e

**Files:**
- Modify: `crates/gateway/tests/assembly_interaction_e2e.rs`

**Interfaces:**
- Inject `ObservedEcuSignal::LeftLowBeamStatus` / `RightLowBeamStatus`
- Assert ledger/context FLCM fields
- Inject Fail → diagnostic/Warning path observable if test harness reads
  diagnostics; at minimum assert context Fail
- Optional: advance time / TimerTick to assert `silent` if harness allows

- [x] **Step 1: Add e2e cases**

- [x] **Step 2:**

```bash
cargo test -p gateway --test assembly_interaction_e2e -- --nocapture
```

- [x] **Step 3: Commit (after user approval)**

```bash
git commit -m "test(gateway): FLCM status observation e2e"
```

---

### Task 8: Live acceptance + run steps (docs in this plan)

**Status:** automated Twin coverage is GREEN; **live acceptance is operator-blocked**
pending `remotivebusd`. Steps 1–8 below are the committed run steps — nothing needed
from a gitignored report. Evidence for the blocked attempt lives (locally, gitignored)
under `.superpowers/sdd/phase-iv/`.

**Blocker (unchanged since 2026-09-16):** `remotivebusd` is `inactive` and
`/run/docker/plugins/remotivebus.sock` is absent, so Hello World compose cannot create
the RemotiveBus CAN networks:

```text
failed to create network remotive_car_hello_world_BodyCan0:
dial unix /run/docker/plugins/remotivebus.sock: connect: no such file or directory
```

`sudo -n systemctl start remotivebusd` requires a password on this host. Docker itself is
healthy; this is a host-privilege gate, not a code defect, and re-running it later needs
no Twin change.

#### Live run order

- [ ] **Step 0: RemotiveBus prerequisite (root)**

Passwordless Docker is not enough — start the RemotiveBus Docker network plugin first:

```bash
sudo systemctl start remotivebusd
systemctl is-active remotivebusd            # expect: active
ls -l /run/docker/plugins/remotivebus.sock  # must exist before compose
```

- [ ] **Step 1: Remotive branch + Hello World**

```bash
cd /home/nirmalya/Workspace-Rust/Eclipse-SDV/RemotiveLabs/remotivelabs-topology-examples
git checkout demo/flcm-lamp-status-feedback

remotive topology build --no-workspace \
  -f remotive_car/instances/hello_world/main.instance.yaml \
  remotive_car/build

cd remotive_car/build/remotive_car_hello_world
docker compose -p remotive_car_hello_world -f docker-compose.yml \
  --profile jupyter --profile 3dcar \
  up --build -d

ss -ltn | grep -E '3000|8888|50051'
```

Cute car `http://127.0.0.1:3000`; Jupyter `http://127.0.0.1:8888` (token
`remotivelabs`). Do **not** `curl` the broker port `50051`.

- [ ] **Step 2: Twin attach (Gateway first)**

```bash
sudo ip link add dev vcan0 type vcan 2>/dev/null || true
sudo ip link set up vcan0

cd /home/nirmalya/Workspace-Rust/Eclipse-SDV/Self-handson-project/sdv_simulation_remotive
# Terminal A
cargo run -p gateway -- --uds observation.sock --connect-timeout 120
# Terminal B
cargo run -p tui_dashboard -- --uds observation.sock
# Terminal C
cargo run -p remotive_bridge -- \
  --broker-url http://127.0.0.1:50051 \
  --can-interface vcan0
# optional motion: --rpm-clamp 3000
```

Require this **five-signal** readiness line (locked order) **before** PowerOn — it is
asserted byte-for-byte by `cargo test -p remotive_bridge --test config_decoder_cli
subscription_ready_status_proves_connection_and_exact_target`:

```text
[remotive_bridge] connected; subscribed signals=\
SCCM-DriverCan0:HazardLightButton.HazardLightButton,\
BCM-BodyCan0:TurnLightControl.LeftTurnLightRequest,\
BCM-BodyCan0:TurnLightControl.RightTurnLightRequest,\
FLCM-BodyCan0:LowBeamLightStatus.LeftLowBeamLightStatus,\
FLCM-BodyCan0:LowBeamLightStatus.RightLowBeamLightStatus
```

- [ ] **Step 3: Healthy baseline**

Jupyter (existing light-stalk widgets): set light mode / low beams ON.  
Expect: cute car front low beams ON; TUI low-beam L/R **OK**; `FLCM: alive`; no FLCM
Warning.

- [ ] **Step 4: Fault — Fail**

Same `ControlClient` pattern as BCM `emergency_mode`; the notebook kernel already holds
`c`. Do **not** invent a second inject mechanism.

```python
from remotivelabs.topology.control import ControlClient, ControlRequest

async with ControlClient(client=c) as cc:
    await cc.send("FLCM", ControlRequest(type="flcm_fault", argument="fail"))
    # expect: TUI Warning + Low beam L/R FAIL; cute car still ON
```

- [ ] **Step 5: Fault — Silent**

```python
    await cc.send("FLCM", ControlRequest(type="flcm_fault", argument="silent"))
    # expect within 500 ms: TUI Warning silent=true, rows read "OK (stale)" /
    # "FLCM: SILENT"; cute car still ON
```

- [ ] **Step 6: Resume**

```python
    await cc.send("FLCM", ControlRequest(type="flcm_fault", argument="ok"))
    # expect: Warning clears (Info FlcmLampFault); status OK; FLCM: alive
```

- [ ] **Step 7: Phase III still works**

Existing notebook hazard pulse → latch ON; cute car blinks; TUI Hazard ON.

- [ ] **Step 8: Shutdown (existing order)**

SIGINT `remotive_bridge` first, then:

```bash
cd /home/nirmalya/Workspace-Rust/Eclipse-SDV/RemotiveLabs/remotivelabs-topology-examples/remotive_car/build/remotive_car_hello_world
docker compose -p remotive_car_hello_world -f docker-compose.yml \
  --profile jupyter --profile 3dcar down
sudo systemctl stop remotivebusd
```

Do not stop `remotivebusd` while Hello World containers are still up.

---

### Task 9: Verification gate

- [x] **Step 1: Twin tests**

```bash
cargo test -p common --test flcm_observation_contract
cargo test -p common --test remotive_observed_signal_contract
cargo test -p common                 # includes the transition-table self-loop test
cargo test -p observation
cargo test -p remotive_bridge
cargo test -p tui_dashboard
cargo test -p gateway --test assembly_interaction_e2e
```

Expected: PASS. `cargo test -p observation --test live_zenoh_roundtrip` needs real
loopback networking; it fails under a network-restricted sandbox and passes on the host.

- [x] **Step 2: Confirm deferred items untouched**

- No BCM consumption of FLCM Status
- No cute-car remap
- No RLCM/E2E/asymmetric hazard work
- No Twin→Remotive actuation

- [x] **Step 3: Review checkpoint — commit docs only if user asks**

---

## Spec coverage (self-check)

| Spec requirement | Task |
|------------------|------|
| Remotive dedicated branch | 1 |
| DBC FLCM Status TX + stub + inject | 1 |
| BCM/cute car blind | 1, 8 |
| Bridge subscribe Status | 5 |
| Twin Status + silence Warning | 3, 6 |
| Schema publish | 4 |
| Carriers / gateway | 2, 7 |
| Run steps + acceptance | 8, 9 |
| No Twin actuation / no BCM teach | Global + 9 |

## Acceptance status

| # | Acceptance | Status |
|---|-----------|--------|
| 1 | Remotive edits only on `demo/flcm-lamp-status-feedback` | Met |
| 2 | Cute car ON + TUI Status OK when healthy | Automated GREEN (Twin side); live pending |
| 3 | Fault inject → TUI Warning, cute car still ON | Automated GREEN; live pending |
| 4 | Resume → Warning clears | Automated GREEN (Info clear) |
| 5 | Phase III hazard + latch still work on the five-signal subscribe | Automated GREEN |
| 6 | Plan/run steps document branch, inject, attach order | Met — Task 8 above |
| 7 | No Twin actuation into Remotive | Met (e2e asserts an empty actuation channel) |

Posture: Twin code merges as "automated GREEN, live acceptance pending
(`remotivebusd` privilege)". Only Task 8 Steps 0–8 remain, and they need no Twin change.

## Jupyter / pytest control snippet (locked API shape)

Same pattern as BCM `emergency_mode` in
`remotive_car/tests/pytest/test_simulate_driver.py` and `common/jupyter/car.ipynb`:

```python
from remotivelabs.topology.control import ControlClient, ControlRequest

async with ControlClient(url) as cc:
    await cc.send("FLCM", ControlRequest(type="flcm_fault", argument="fail"))
    await cc.send("FLCM", ControlRequest(type="flcm_fault", argument="silent"))
    await cc.send("FLCM", ControlRequest(type="flcm_fault", argument="ok"))
```

Optional: add three buttons to a local copy of the Hello World notebook on the
Remotive demo branch only. Do not invent a second inject mechanism.
