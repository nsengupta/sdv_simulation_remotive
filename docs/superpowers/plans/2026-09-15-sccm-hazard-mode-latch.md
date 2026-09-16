# SCCM Hazard-Mode Latch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:subagent-driven-development` (recommended) or
> `superpowers:executing-plans` to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Latch Remotive Hello World’s flip-switch hazard **mode** in the Twin
SCCM actor from observed button rising edges, and drive TUI `Hazard:` from that
mode (steady ON until the next press) while keeping the button wire
pulse-faithful.

**Architecture:** `SccmContext` / `SccmActor` gain `hazard_mode`. On each
accepted `HazardButtonObserved(true)` that is a rising edge relative to the
previous `hazard_button`, toggle mode. Publish mode beside the button in
schema/facade; TUI driver `Hazard:` binds to mode only. Bridge subscription
and Remotive trees stay unchanged. Blinking Hazard paint and asymmetric-lamp
fault demos stay deferred.

**Tech Stack:** Rust 2024, Ractor SCCM actor, Serde observation schema,
Ratatui TUI, existing Gateway assembly e2e.

## Global Constraints

- Work on branch `remotive-integration`.
- Spec:
  `docs/superpowers/specs/2026-09-15-sccm-hazard-mode-latch-design.md`.
- Do not modify RemotiveLabs trees.
- Do not widen the three Phase II bridge subscriptions.
- Do not add Twin→Remotive actuation.
- Do not implement blinking driver `Hazard:` paint in this plan.
- Do not implement the later asymmetric one-headlight-dead fault demo here;
  only leave a deferred note if needed.
- Prefer RED→GREEN tests before production changes.
- Make no significant commit without explicit user approval.
- Keep historical ledgers readable: missing `hazard_mode_on` deserializes as
  OFF.
- Bump observation `CURRENT_SCHEMA_VERSION` to **6** when emitting the new
  field (v1–v5 remain readable).

---

## File Structure

| Path | Responsibility |
|------|----------------|
| `crates/common/src/vehicle_state/sccm.rs` | Latch logic + lifecycle reset |
| `crates/common/src/test/sccm_observation_contract.rs` | Pure SCCM RED/GREEN contracts |
| `crates/common/src/observation_records/transition/mod.rs` | `PublishedSccmContext` |
| `crates/observation/src/schema/mod.rs` | `CURRENT_SCHEMA_VERSION = 6` |
| `crates/observation/src/schema/v1.rs` | `SccmContextV1.hazard_mode_on` + defaults |
| `crates/observation/tests/schema_compatibility.rs` | v5 missing-field / v6 emit |
| `crates/tui_dashboard/src/view/driver.rs` | Driver `Hazard:` ← mode |
| `crates/tui_dashboard/src/view/engineer.rs` | Engineer SCCM primary = mode |
| `crates/gateway/tests/assembly_interaction_e2e.rs` | Mode survives button OFF pulse |

---

### Task 1: SCCM rising-edge latch

**Files:**
- Modify: `crates/common/src/vehicle_state/sccm.rs`
- Modify: `crates/common/src/test/sccm_observation_contract.rs`
- Modify if needed: `crates/common/src/test/sccm_actor_contract.rs`

**Interfaces:**
- Produces:

```rust
pub struct SccmContext {
    pub hazard_button_on: bool,           // legacy mirror of last wire
    pub hazard_button: ObservedBool,      // last wire
    pub hazard_mode: ObservedBool,        // latched; Off after default/PowerOn
}

// On HazardButtonObserved(value):
// 1. disposition = hazard_button.classify(value)
// 2. if hazard_button != On && value == true { toggle hazard_mode Off<->On }
// 3. update hazard_button / hazard_button_on
// BecomeOn / BecomeOff: hazard_mode = Off; BecomeOn also hazard_button = Unknown
//   (and hazard_button_on = false)
```

Rising edge means previous `hazard_button` is **not** `ObservedBool::On` and
incoming `value` is `true` (covers `Unknown→On` and `Off→On`).

- [ ] **Step 1: Write failing latch tests**

In `sccm_observation_contract.rs` add (keep existing tests green by updating
defaults where they assert full context):

```rust
#[test]
fn sccm_default_hazard_mode_is_off() {
    assert_eq!(SccmContext::default().hazard_mode, ObservedBool::Off);
}

#[test]
fn sccm_rising_edge_toggles_mode_on_and_pulse_off_leaves_mode_on() {
    let on = SccmContext::default()
        .on_receiving_message(SccmMessage::HazardButtonObserved(true));
    assert_eq!(on.ctx.hazard_button, ObservedBool::On);
    assert_eq!(on.ctx.hazard_mode, ObservedBool::On);

    let wire_off = on
        .ctx
        .on_receiving_message(SccmMessage::HazardButtonObserved(false));
    assert_eq!(wire_off.ctx.hazard_button, ObservedBool::Off);
    assert_eq!(wire_off.ctx.hazard_mode, ObservedBool::On);
}

#[test]
fn sccm_second_rising_edge_toggles_mode_off() {
    let after_pulse = SccmContext::default()
        .on_receiving_message(SccmMessage::HazardButtonObserved(true))
        .ctx
        .on_receiving_message(SccmMessage::HazardButtonObserved(false));
    let second = after_pulse
        .on_receiving_message(SccmMessage::HazardButtonObserved(true));
    assert_eq!(second.ctx.hazard_mode, ObservedBool::Off);
    assert_eq!(second.ctx.hazard_button, ObservedBool::On);
}

#[test]
fn sccm_become_on_resets_mode_off_and_button_unknown() {
    let armed = SccmContext::default()
        .on_receiving_message(SccmMessage::HazardButtonObserved(true))
        .ctx;
    let reset = armed.on_receiving_message(SccmMessage::BecomeOn);
    assert_eq!(reset.ctx.hazard_mode, ObservedBool::Off);
    assert_eq!(reset.ctx.hazard_button, ObservedBool::Unknown);
    assert_eq!(reset.disposition, ObservationDisposition::Lifecycle);
}
```

- [ ] **Step 2: Run tests — expect FAIL**

```bash
cd /home/nirmalya/Workspace-Rust/Eclipse-SDV/Self-handson-project/sdv_simulation_remotive
cargo test -p common sccm_ -- --nocapture
```

Expected: new tests fail (missing `hazard_mode` / no toggle / BecomeOn no reset).

- [ ] **Step 3: Implement minimal `sccm.rs`**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SccmContext {
    pub hazard_button_on: bool,
    pub hazard_button: ObservedBool,
    pub hazard_mode: ObservedBool,
}

impl Default for SccmContext {
    fn default() -> Self {
        Self {
            hazard_button_on: false,
            hazard_button: ObservedBool::Unknown,
            hazard_mode: ObservedBool::Off,
        }
    }
}

impl SccmContext {
    pub fn on_receiving_message(&self, message: SccmMessage) -> SccmZoneReply {
        match message {
            SccmMessage::BecomeOn => SccmZoneReply {
                ctx: Self {
                    hazard_button_on: false,
                    hazard_button: ObservedBool::Unknown,
                    hazard_mode: ObservedBool::Off,
                },
                disposition: ObservationDisposition::Lifecycle,
            },
            SccmMessage::BecomeOff => SccmZoneReply {
                ctx: Self {
                    hazard_button_on: self.hazard_button_on,
                    hazard_button: self.hazard_button,
                    hazard_mode: ObservedBool::Off,
                },
                disposition: ObservationDisposition::Lifecycle,
            },
            SccmMessage::HazardButtonObserved(value) => {
                let disposition = self.hazard_button.classify(value);
                let mut ctx = self.clone();
                let rising = self.hazard_button != ObservedBool::On && value;
                if rising {
                    ctx.hazard_mode = match ctx.hazard_mode {
                        ObservedBool::On => ObservedBool::Off,
                        _ => ObservedBool::On,
                    };
                }
                ctx.hazard_button = ObservedBool::from(value);
                ctx.hazard_button_on = value;
                SccmZoneReply { ctx, disposition }
            }
        }
    }
}
```

Adjust `BecomeOff` button fields if existing actor tests require full clear;
spec requires mode OFF on PowerOff — button clear is allowed if tests prefer
symmetry with BecomeOn.

- [ ] **Step 4: Run tests — expect PASS**

```bash
cargo test -p common sccm_ -- --nocapture
```

- [ ] **Step 5: Review checkpoint (no commit unless user approves)**

---

### Task 2: Publish schema v6 + facade

**Files:**
- Modify: `crates/common/src/observation_records/transition/mod.rs`
  (`PublishedSccmContext`)
- Modify: `crates/observation/src/schema/mod.rs` (`CURRENT_SCHEMA_VERSION = 6`)
- Modify: `crates/observation/src/schema/v1.rs` (`SccmContextV1`)
- Modify: `crates/observation/tests/schema_compatibility.rs`
- Modify: `crates/observation/tests/hazard_schema.rs` / `round_trip.rs` as
  needed for version asserts
- Modify: any `PublishedSccmContext { hazard_button_on: ... }` literals across
  the workspace to include `hazard_mode_on`

**Interfaces:**
- Produces:

```rust
pub struct PublishedSccmContext {
    pub hazard_button_on: PublishedObservedBool,
    pub hazard_mode_on: PublishedObservedBool,
}

// SccmContextV1 { hazard_button_on, #[serde(default)] hazard_mode_on }
// default ObservedBoolV1::Off for missing field on v1–v5 JSON
```

- [ ] **Step 1: Failing schema / publish tests**

```rust
#[test]
fn current_schema_version_is_v6() {
    assert_eq!(CURRENT_SCHEMA_VERSION, 6);
}

#[test]
fn v5_sccm_json_without_hazard_mode_defaults_off() {
    let sccm: SccmContextV1 =
        serde_json::from_str(r#"{"hazard_button_on":"on"}"#).unwrap();
    assert_eq!(sccm.hazard_mode_on, ObservedBoolV1::Off);
}

#[test]
fn published_sccm_maps_mode_from_context() {
    let mut ctx = SccmContext::default();
    ctx.hazard_mode = ObservedBool::On;
    let published = PublishedSccmContext::from(&ctx);
    assert_eq!(published.hazard_mode_on, PublishedObservedBool::On);
}
```

Place the third test in `hazard_observation_contract.rs` or a small publish
test module already used for SCCM mapping.

- [ ] **Step 2: Run focused tests — expect FAIL**

```bash
cargo test -p observation current_schema_version_is_v6 -- --nocapture
cargo test -p observation v5_sccm_json_without_hazard_mode -- --nocapture
```

- [ ] **Step 3: Implement schema + facade**

- Set `CURRENT_SCHEMA_VERSION` to `6`.
- Add `hazard_mode_on` with `#[serde(default)]` (Off) on `SccmContextV1` and
  on `PublishedSccmContext` if needed for older live JSON.
- Map `From<&SccmContext>` → `hazard_mode_on: s.hazard_mode.into()`.
- Update writer paths that construct `SccmContextV1` / published SCCM.
- Fix compile errors in test fixtures by adding `hazard_mode_on: …Off` (or
  `Default`).

- [ ] **Step 4: Run observation + common publish tests**

```bash
cargo test -p observation -- --skip zenoh
cargo test -p common hazard_ -- --nocapture
```

Expected: PASS (update any hard-coded `schema_version == 5` asserts to 6).

- [ ] **Step 5: Review checkpoint**

---

### Task 3: TUI binds Hazard to mode

**Files:**
- Modify: `crates/tui_dashboard/src/view/driver.rs`
- Modify: `crates/tui_dashboard/src/view/engineer.rs`
- Modify: `crates/tui_dashboard/src/view/ledger_tail.rs` (if it prints SCCM
  hazard — prefer mode for attended summary, keep event names as wire events)
- Modify: `crates/tui_dashboard/src/main.rs` live-pane smoke test

**Interfaces:**
- Consumes: `row.current_ctx.sccm.hazard_mode_on`
- Driver label `Hazard:` must follow mode, not `hazard_button_on`.

- [ ] **Step 1: Failing TUI tests**

```rust
#[test]
fn driver_hazard_follows_mode_not_button_wire() {
    let mut ledger = sample_ledger(10, 100, PublishedHeadlampState::Off);
    ledger.current_ctx.sccm.hazard_button_on = PublishedObservedBool::Off;
    ledger.current_ctx.sccm.hazard_mode_on = PublishedObservedBool::On;
    let text = pane_text(&driver_pane(None, Some(&ledger), 64));
    assert!(text.contains("Hazard: ON"), "{text}");
}

#[test]
fn engineer_sccm_line_shows_mode() {
    let mut ledger = sample_ledger();
    ledger.current_ctx.sccm.hazard_button_on = PublishedObservedBool::Off;
    ledger.current_ctx.sccm.hazard_mode_on = PublishedObservedBool::On;
    let text = pane_text(&engineer_pane(Some(&ledger), 64));
    assert!(text.contains("SCCM: Hazard ON"), "{text}");
}
```

Update existing tests that set only `hazard_button_on` expecting `Hazard: ON`
to set `hazard_mode_on` instead (or both).

- [ ] **Step 2: Run — expect FAIL**

```bash
cargo test -p tui_dashboard driver_hazard_follows_mode -- --nocapture
```

- [ ] **Step 3: Bind panes to `hazard_mode_on`**

Replace driver/engineer reads of `hazard_button_on` for the attended Hazard
label with `hazard_mode_on`. Do **not** add blinking paint.

- [ ] **Step 4: Full TUI package**

```bash
cargo test -p tui_dashboard -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Review checkpoint**

---

### Task 4: Gateway assembly e2e — mode survives wire OFF

**Files:**
- Modify: `crates/gateway/tests/assembly_interaction_e2e.rs`
- Modify: `crates/gateway/src/transition_log.rs` only if log should show mode
  (optional: add `sccm.hazard_mode=`; if added, keep button field too)

**Interfaces:**
- After cansend/path that observes hazard true then false, assert
  `hazard_mode_on == On` and `hazard_button_on == Off` on published context
  (or runtime `ObservedBool` equivalents).

- [ ] **Step 1: Extend e2e assertions**

After the existing hazard ON observation, inject/observe hazard OFF wire and
assert mode remains On. After a second ON edge, assert mode Off.

If the e2e harness only sends one hazard edge today, add the complementary
OFF then second ON using the same SocketCAN / event path already used for
`0x105`.

- [ ] **Step 2: Run e2e**

```bash
cargo test -p gateway --test assembly_interaction_e2e -- --nocapture
```

Expected: FAIL until runtime publishes mode; then PASS after Tasks 1–2.

- [ ] **Step 3: Fix any projection gaps**

If zone turn / publish path copies only `hazard_button`, ensure
`PublishedSccmContext::from` (Task 2) is used so mode is not dropped.

- [ ] **Step 4: Review checkpoint**

---

### Task 5: Verification gate + optional live check

**Files:** none required beyond prior tasks.

- [ ] **Step 1: Automated gates**

```bash
cargo test -p common sccm_
cargo test -p observation -- --skip zenoh
cargo test -p tui_dashboard
cargo test -p gateway --test assembly_interaction_e2e
cargo test -p remotive_bridge --test config_decoder_cli
```

Expected: all PASS; bridge still exactly three signals.

- [ ] **Step 2: Optional live Hello World check** (manual / same Phase III
  shell order)

1. RemotiveBus healthy (plan Appendix A if needed).
2. Hello World jupyter+3dcar up; Gateway UDS → TUI → bridge.
3. First Jupyter hazard pulse → TUI `Hazard: ON` stays ON while L/R blink.
4. Second pulse → `Hazard: OFF`; car blink stops.
5. No `SetTurnLights`.

- [ ] **Step 3: Confirm deferred items untouched**

- No blinking Hazard label.
- No asymmetric one-lamp fault demo.
- No Remotive source edits; no bridge widen; no Twin publisher.

- [ ] **Step 4: Offer commit only if user asks**

Suggested message:

```text
feat: latch SCCM hazard mode for flip-switch TUI Hazard

Toggle hazard_mode on observed button rising edges and bind driver
Hazard to mode so Hello World pulses keep hazards armed until next press.
```

---

## Self-Review (plan vs spec)

| Spec requirement | Task |
|------------------|------|
| SCCM actor latch on rising edge | Task 1 |
| Wire stays pulse-faithful | Task 1 |
| PowerOn resets mode OFF | Task 1 |
| Publish button + mode; historical default OFF | Task 2 |
| Schema bump / readability | Task 2 (v6) |
| TUI Hazard ← mode steady | Task 3 |
| L/R unchanged blink | Task 3–4 |
| Second press mode OFF + Remotive stops blink | Task 4–5 |
| No bridge widen / no Twin actuation | Global + Task 5 |
| Blinking Hazard deferred | Global + Task 5 |
| Asymmetric lamp fault deferred | Global + Task 5 |

Placeholder scan: concrete tests and code. Types: `hazard_mode` /
`hazard_mode_on` naming is consistent (runtime `hazard_mode`, published
`hazard_mode_on` matching existing `hazard_button` / `hazard_button_on`).
