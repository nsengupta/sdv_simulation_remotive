# Remotive Authoritative ECU Observation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:subagent-driven-development` (recommended) or
> `superpowers:executing-plans` to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the unchanged Remotive `getting_started` BCM authoritative for
hazard behavior while the Rust Twin deterministically records and displays the
observed SCCM input and BCM turn-light output.

**Architecture:** The bridge keeps its Phase I lifecycle/RPM responsibility and
expands one broker subscription to three independently delivered signals.
Gateway projects strict internal SocketCAN observation frames into events;
SCCM and BCM child actors hold and aggregate observed ECU state without
implementing ECU policy. The existing barrier, ledger, schema compatibility,
and live TUI remain the Twin's value boundary.

**Tech Stack:** Rust 2024, Tokio, Ractor, SocketCAN, Remotive gRPC Rust API,
Serde/JSONL observation schema, Ratatui, pytest, Docker Compose.

## Global Constraints

- Work on branch `remotive-integration` from Phase I head `4cc75fb`.
- Evolve the current runtime; do not retain a selectable Phase I runtime profile.
- Do not modify any file under either RemotiveLabs repository.
- Do not overwrite, stage, or reformat existing user-owned untracked files.
- Keep the approved design and this plan uncommitted until implementation,
  tests, and live acceptance are complete.
- Make no significant commit without explicit user approval.
- Use behavior-focused RED-to-GREEN tests before every production change.
- Keep bridge-generated lifecycle and RPM behavior unchanged.
- Add no Twin-to-Remotive publisher and no physical-lamp state.
- Preserve the Phase I internal hazard carrier ID `0x105`; add internal IDs
  `0x106` and `0x107` for independently delivered left and right requests.
- Do not reconstruct Remotive DBC messages 100/103 inside the bridge; the
  broker API already delivers the selected signal values.
- Preserve historical schema v1-v4 readability and historical
  `SetTurnLights` records.
- Keep legacy headlamp/wiper code and schema fields for compatibility, but do
  not activate or display them in the Phase II integration.
- Keep Zenoh-dependent discovery tests outside the binding regression gate.
- After each task, stop for a review checkpoint; do not commit.

---

### Task 1: Independent Remotive signal and bridge contracts

**Files:**
- Create: `crates/common/src/test/remotive_observed_signal_contract.rs`
- Modify: `crates/common/src/test/mod.rs`
- Modify: `crates/common/src/signals.rs`
- Modify: `crates/common/src/domain_types.rs`
- Modify: `crates/common/src/facade.rs`
- Modify: `crates/remotive_bridge/src/decoder.rs`
- Modify: `crates/remotive_bridge/src/source.rs`
- Modify: `crates/remotive_bridge/src/session.rs`
- Modify: `crates/remotive_bridge/src/main.rs`
- Modify: `crates/remotive_bridge/tests/config_decoder_cli.rs`
- Modify: `crates/remotive_bridge/tests/session_contract.rs`

**Interfaces:**
- Produces:

```rust
pub const ID_HAZARD: u16 = 0x105;
pub const ID_LEFT_TURN_REQUEST: u16 = 0x106;
pub const ID_RIGHT_TURN_REQUEST: u16 = 0x107;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservedEcuSignal {
    HazardButton(bool),
    LeftTurnRequest(bool),
    RightTurnRequest(bool),
}

impl ObservedEcuSignal {
    pub fn from_can_frame(frame: &CanFrame) -> Option<Self>;
    pub fn to_can_frame(self) -> Result<CanFrame, socketcan::Error>;
}
```

- `TwinIngressEvent::ObservedEcu(ObservedEcuSignal)` carries validated
  transport-independent observations into the Twin.
- Bridge source produces ordered `BrokerObservation` values:

```rust
pub enum BrokerObservation {
    HazardButton(bool),
    LeftTurnRequest(bool),
    RightTurnRequest(bool),
    End,
}

#[async_trait]
pub trait ObservationSource {
    async fn next_observation(&mut self) -> anyhow::Result<BrokerObservation>;
}

#[async_trait]
pub trait ObservationConnector {
    type Source: ObservationSource;
    async fn connect(self) -> anyhow::Result<Self::Source>;
}
```

- [ ] **Step 1: Add failing exact-wire tests**

Test the three standard internal IDs, strict two-byte boolean payloads
`[value, 0]`, invalid-value/reserved-byte rejection, extended-ID rejection, and
round trips:

```rust
#[test]
fn remotive_hazard_preserves_the_phase_one_internal_carrier() {
    let frame = ObservedEcuSignal::HazardButton(true)
        .to_can_frame()
        .expect("encode hazard");
    assert_eq!(frame.raw_id(), 0x105);
    assert_eq!(frame.data(), &[1, 0]);
    assert_eq!(
        ObservedEcuSignal::from_can_frame(&frame),
        Some(ObservedEcuSignal::HazardButton(true))
    );
}

#[test]
fn remotive_left_request_has_an_independent_internal_carrier() {
    let frame = ObservedEcuSignal::LeftTurnRequest(true)
    .to_can_frame()
        .expect("encode left request");
    assert_eq!(frame.raw_id(), 0x106);
    assert_eq!(frame.data(), &[1, 0]);
}
```

- [ ] **Step 2: Run the wire tests and confirm RED**

Run:

```bash
cargo test -p common remotive_observed_signal_contract -- --nocapture
```

Expected: compilation fails because `ObservedEcuSignal` and the two IDs do not
exist.

- [ ] **Step 3: Implement the strict codecs and ingress vocabulary**

Add `ObservedEcuSignal`; accept hazard payloads `0`/`1`, turn payloads `0..=3`,
exactly one byte, and standard IDs only. Leave Phase I `ControlSignal` readable
only where historical tests require it, but route new bridge frames through
`TwinIngressEvent::ObservedEcu`.

- [ ] **Step 4: Run focused common tests and confirm GREEN**

```bash
cargo test -p common remotive_observed_signal_contract -- --nocapture
cargo test -p common hazard_signal_contract -- --nocapture
cargo test -p common lifecycle_signal_contract -- --nocapture
```

Expected: all selected tests pass; lifecycle remains `0x100`, RPM remains
`0x102`, legacy ambient lux remains `0x103`, and hazard remains `0x105`.

- [ ] **Step 5: Add failing three-signal subscription and batch-decoder tests**

Assert exactly these IDs in one `SubscriberConfig`, in this order:

```text
SCCM-DriverCan0:HazardLightButton.HazardLightButton
BCM-BodyCan0:TurnLightControl.LeftTurnLightRequest
BCM-BodyCan0:TurnLightControl.RightTurnLightRequest
```

Add fixtures proving:

- integer, unsigned integer, `"Off"`, and `"On"` decode;
- left and right each emit immediately without waiting for the other;
- a batch containing only one request remains valid;
- values from different batches remain independent;
- a hazard, right, and left batch preserves exact broker signal order;
- the readiness line lists all three exact identities.

- [ ] **Step 6: Run bridge decoder tests and confirm RED**

```bash
cargo test -p remotive_bridge --test config_decoder_cli -- --nocapture
```

Expected: exact-subscription and independent turn-signal tests fail against the
Phase I hazard-only source.

- [ ] **Step 7: Implement the multi-signal source**

Rename hazard-only source traits/types to observation equivalents. Decode each
broker batch into a `VecDeque<BrokerObservation>` in signal arrival order.
Emit left and right observations immediately and independently, with no
pairing cache or incomplete-pair rejection. Retain the existing power-of-two
diagnostic throttling for wrong identities and malformed payloads.

`connect()` must await `subscribe_to_signals(subscription_config())`, then log
`subscription_ready_status()`, then return the source.

- [ ] **Step 8: Add failing bridge-session ordering tests**

Prove:

- connection failure writes no CAN frame;
- after successful connection, `PowerOn` is first;
- hazard, left, and right observations use IDs `0x105`, `0x106`, and `0x107`
  in source order;
- RPM fairness remains;
- stream end/error attempts `RPM=0` then `PowerOff`;
- controlled shutdown behavior is unchanged.

- [ ] **Step 9: Run session tests and confirm RED**

```bash
cargo test -p remotive_bridge --test session_contract -- --nocapture
```

Expected: observation-source and independent left/right carrier cases fail
before session generalization.

- [ ] **Step 10: Generalize the session and confirm GREEN**

Map each `BrokerObservation` to one independent `ObservedEcuSignal` CAN frame.
Preserve the existing three-way fair selection between broker observations,
RPM, and shutdown; do not add publisher behavior or CLI flags.

```bash
cargo test -p remotive_bridge -- --nocapture
cargo test -p common remotive_observed_signal_contract -- --nocapture
git diff --check
```

Expected: bridge and focused common suites pass.

- [ ] **Step 11: Review checkpoint**

Review Task 1 diff for exact upstream signal identities, DBC payload accuracy,
unchanged RPM/lifecycle behavior, and absence of Remotive file changes. Do not
commit.

---

### Task 2: Observation projection and pure ECU state

**Files:**
- Create: `crates/common/src/vehicle_state/observed.rs`
- Create: `crates/common/src/test/sccm_observation_contract.rs`
- Create: `crates/common/src/test/bcm_observation_contract.rs`
- Modify: `crates/common/src/test/mod.rs`
- Modify: `crates/common/src/vehicle_state/mod.rs`
- Modify: `crates/common/src/vehicle_state/sccm.rs`
- Modify: `crates/common/src/vehicle_state/bcm.rs`
- Modify: `crates/common/src/fsm/machineries.rs`
- Modify: `crates/common/src/fsm/transition_map.rs`
- Modify: `crates/common/src/twin_runtime/connectors/ingress_to_fsm.rs`
- Modify: `crates/gateway/src/ingress/mapping.rs`

**Interfaces:**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ObservedBool {
    #[default]
    Unknown,
    Off,
    On,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationDisposition {
    Initial,
    Changed { completed_duplicates: u64 },
    Duplicate { current_duplicates: u64 },
    Lifecycle,
}

pub enum FsmEvent {
    // existing lifecycle/RPM events remain
    HazardButtonObserved(bool),
    LeftTurnRequestObserved(bool),
    RightTurnRequestObserved(bool),
}
```

`SccmContext` stores `ObservedBool`; `BcmContext` retains lifecycle readiness
and stores two `ObservedBool` values. Neither context maps hazard input into
turn output or returns a `DomainAction`.

- [ ] **Step 1: Add failing Gateway/projector tests**

Assert:

```rust
ObservedEcuSignal::HazardButton(true)
    -> TwinIngressEvent::ObservedEcu(...)
    -> FsmEvent::HazardButtonObserved(true)

ObservedEcuSignal::LeftTurnRequest(true)
    -> FsmEvent::LeftTurnRequestObserved(true)

ObservedEcuSignal::RightTurnRequest(false)
    -> FsmEvent::RightTurnRequestObserved(false)
```

Also assert malformed IDs/DLCs return `None`, and Phase I
`HazardButtonChanged` is no longer produced by active ingress.

- [ ] **Step 2: Run mapping/projection tests and confirm RED**

```bash
cargo test -p gateway ingress -- --nocapture
cargo test -p common projection_contract -- --nocapture
```

Expected: new observed variants are not mapped.

- [ ] **Step 3: Implement Gateway and projector mappings**

Decode `ObservedEcuSignal` before legacy `ControlSignal`/`VssSignal`, then map
the validated observation to the new FSM event. Keep lifecycle and RPM
projections unchanged.

- [ ] **Step 4: Add failing pure-state tests**

For SCCM and BCM independently, cover:

- default value is `Unknown`;
- first false value is `Initial`, not a duplicate;
- repeated equal value is `Duplicate`;
- complementary/different value is `Changed`;
- BCM updates left and right independently without changing the other side;
- BCM observation creates no `BcmOutcome::TurnLightsChanged` and no
  `SetTurnLights`;
- `BecomeOff` restores readiness state without fabricating observed values.

- [ ] **Step 5: Run pure-state tests and confirm RED**

```bash
cargo test -p common sccm_observation_contract -- --nocapture
cargo test -p common bcm_observation_contract -- --nocapture
```

Expected: SCCM actor vocabulary and observation-only BCM behavior do not exist.

- [ ] **Step 6: Implement observation state without ECU policy**

Use `ObservedBool::from(bool)` for value projection. Keep duplicate counters
out of the published contexts; counters belong to actor runtime state in Task
3. Remove `HazardButtonChanged` from active BCM message handling and remove
turn-light outcomes from the observation path.

- [ ] **Step 7: Run Task 2 verification**

```bash
cargo test -p common sccm_observation_contract -- --nocapture
cargo test -p common bcm_observation_contract -- --nocapture
cargo test -p common projection_contract -- --nocapture
cargo test -p gateway ingress -- --nocapture
git diff --check
```

Expected: all selected tests pass.

- [ ] **Step 8: Review checkpoint**

Confirm Remotive owns the hazard-to-turn decision and no active Twin path emits
`SetTurnLights`. Do not commit.

---

### Task 3: SCCM/BCM child actors, duplicate streaks, and deterministic commits

**Files:**
- Create: `crates/common/src/twin_runtime/sccm_actor.rs`
- Create: `crates/common/src/test/sccm_actor_contract.rs`
- Create: `crates/common/src/test/observed_duplicate_contract.rs`
- Create: `crates/common/src/test/observed_reorder_contract.rs`
- Modify: `crates/common/src/test/mod.rs`
- Modify: `crates/common/src/digital_twin/mod.rs`
- Modify: `crates/common/src/twin_runtime/mod.rs`
- Modify: `crates/common/src/twin_runtime/bcm_actor.rs`
- Modify: `crates/common/src/twin_runtime/zone_turn.rs`
- Modify: `crates/common/src/twin_runtime/zone_replies.rs`
- Modify: `crates/common/src/twin_runtime/twin_turn.rs`
- Modify: `crates/common/src/twin_runtime/outcome_map.rs`
- Modify: `crates/common/src/twin_runtime/controller/virtual_car_actor.rs`
- Modify: `crates/common/src/twin_runtime/controller/vehicle_controller.rs`
- Modify: `crates/common/src/fsm/machineries.rs`
- Modify: `crates/common/src/test/bcm_actor_contract.rs`
- Modify: `crates/common/src/test/fsm_preparation_contract.rs`
- Modify: `crates/common/src/test/hazard_fsm_contract.rs`
- Modify: `crates/common/src/test/hazard_actuation_contract.rs`

**Interfaces:**

```rust
pub enum AssemblyId {
    Sccm,
    Bcm,
    // legacy variants remain constructible
}

pub struct ObservationStreak<T> {
    current: Option<T>,
    duplicates: u64,
}

impl<T: Copy + Eq> ObservationStreak<T> {
    pub fn observe(&mut self, value: T) -> ObservationDisposition;
    pub fn pending_summary(&self) -> Option<(T, u64)>;
}
```

Both zone replies include `ObservationDisposition`. `BcmActorState` owns
separate left and right streaks. Lifecycle replies use `Lifecycle`; every
duplicate still tells back so no barrier can remain blocked.

- [ ] **Step 1: Add failing generic streak tests**

Prove:

```text
Unknown + false -> Initial, count 0
false + false   -> Duplicate, count 1
false + false   -> Duplicate, count 2
false + true    -> Changed(completed_duplicates=2), new count 0
shutdown        -> summary(true, current_count)
```

Use separate SCCM and BCM streak instances to prove counters cannot interfere.
Within BCM, use separate left and right streaks and prove that repeating one
side does not increment or reset the other.

- [ ] **Step 2: Run streak tests and confirm RED**

```bash
cargo test -p common observed_duplicate_contract -- --nocapture
```

Expected: observation streak/runtime state does not exist.

- [ ] **Step 3: Implement streak classification in actor state**

Add `SccmActorState` and extend `BcmActorState` with independent streaks.
Children must always send a correlated `ZoneReady`, including duplicate
classification. On initial/change, update context; on duplicate, return the
unchanged context.

- [ ] **Step 4: Add failing SCCM/BCM actor tests**

Cover lifecycle readiness, exact message ownership, correlated tell-back,
duplicate replies, completed-streak reporting, and shutdown summaries. Rewrite
Phase I BCM tests so they reject hazard-to-turn computation.

- [ ] **Step 5: Run child tests and confirm RED**

```bash
cargo test -p common sccm_actor_contract -- --nocapture
cargo test -p common bcm_actor_contract -- --nocapture
```

Expected: SCCM actor and observed BCM behavior are absent.

- [ ] **Step 6: Wire the active actor topology**

Make `[AssemblyId::Sccm, AssemblyId::Bcm]` the single active assembly set.
Spawn both children. Route:

```text
HazardButtonObserved       -> SccmMessage::HazardButtonObserved
LeftTurnRequestObserved    -> BcmMessage::LeftTurnRequestObserved
RightTurnRequestObserved   -> BcmMessage::RightTurnRequestObserved
```

Keep headlamp/wiper types available but inactive. Do not introduce a Phase II
runtime selector.

- [ ] **Step 7: Add failing duplicate-commit and reorder tests**

Drive:

```text
hazard false, hazard false, hazard true,
left false, left false, right false, right false, left true, right true
```

Assert six initial/changed records, zero duplicate records, no domain actions,
three independent completed streaks, and monotonic record sequence. Delay the
SCCM reply for an earlier turn while allowing the BCM reply for a later turn;
assert the ledger still commits ingress order.

- [ ] **Step 8: Run orchestration tests and confirm RED**

```bash
cargo test -p common observed_duplicate_contract -- --nocapture
cargo test -p common observed_reorder_contract -- --nocapture
cargo test -p common turn_barrier_contract -- --nocapture
```

Expected: duplicate self-loop records still appear or active topology lacks
SCCM.

- [ ] **Step 9: Suppress duplicate commits without blocking the barrier**

Resolve and pop every duplicate barrier normally, but skip context mutation,
domain-action mapping, transition-record emission, and live delivery when the
resolved reply disposition is `Duplicate`. Initial and changed replies use the
normal deterministic commit path. Emit bounded log summaries on a change and
remaining streak summaries during controlled actor shutdown.

- [ ] **Step 10: Remove active Phase I BCM computation**

Retain historical action/schema vocabulary, but ensure no active mapping from
`HazardButtonObserved` to BCM and no active observed BCM outcome maps to
`DomainAction::SetTurnLights`.

- [ ] **Step 11: Run Task 3 verification**

```bash
cargo test -p common -- --nocapture
git diff --check
```

Expected: the complete common suite passes with deterministic ordering and
zero duplicate rows.

- [ ] **Step 12: Review checkpoint**

Review barrier resolution carefully: duplicate children must tell back, the
head-of-buffer must advance, and later changed observations must not starve.
Do not commit.

---

### Task 4: Schema v5, transition logging, and historical compatibility

**Files:**
- Modify: `crates/common/src/observation_records/transition/mod.rs`
- Modify: `crates/common/src/test/hazard_observation_contract.rs`
- Modify: `crates/observation/src/schema/mod.rs`
- Modify: `crates/observation/src/schema/v1.rs`
- Modify: `crates/observation/src/reader.rs`
- Modify: `crates/observation/tests/schema_compatibility.rs`
- Modify: `crates/observation/tests/hazard_schema.rs`
- Create: `crates/observation/testdata/golden/v5/00000000-0000-4000-8000-000000000001/manifest.json`
- Create: `crates/observation/testdata/golden/v5/00000000-0000-4000-8000-000000000001/ledger.jsonl`
- Create: `crates/observation/testdata/golden/v5/00000000-0000-4000-8000-000000000001/diagnostic.jsonl`
- Modify: `crates/gateway/src/transition_log.rs`

**Interfaces:**

```rust
pub enum PublishedObservedBool {
    Unknown,
    Off,
    On,
}

pub enum PublishedFsmEvent {
    HazardButtonObserved(bool),
    LeftTurnRequestObserved(bool),
    RightTurnRequestObserved(bool),
    // historical variants remain readable
}
```

Schema v5 writes tri-state observed values. Its deserializer accepts historical
v3/v4 boolean SCCM/BCM fields as `Off`/`On`. Old event and action variants stay
in the DTO.

- [ ] **Step 1: Add failing v5 and historical-read tests**

Assert:

- `CURRENT_SCHEMA_VERSION == 5`;
- v5 initial values serialize as `"unknown"`;
- observed events round-trip losslessly;
- v4 `HazardButtonChanged` plus `SetTurnLights` still reads;
- v1-v3 golden fixtures still read;
- unsupported v6 manifests fail clearly.

- [ ] **Step 2: Run observation tests and confirm RED**

```bash
cargo test -p observation schema_compatibility -- --nocapture
cargo test -p observation hazard_schema -- --nocapture
cargo test -p common hazard_observation_contract -- --nocapture
```

Expected: current schema is v4 and observed DTO variants are absent.

- [ ] **Step 3: Implement v5 projections and compatibility**

Update published context/event conversions, manifest support, golden fixture,
and live envelope version. Use explicit serde aliases/custom deserialization
where required; do not delete historical variants or fixtures.

- [ ] **Step 4: Add failing Gateway log-format tests**

Require readable output containing:

```text
HazardButtonObserved(true)
sccm.hazard_button=on
LeftTurnRequestObserved(true)
bcm.left_turn_request=on
RightTurnRequestObserved(true)
bcm.right_turn_request=on
```

No duplicate event reaches this formatter.

- [ ] **Step 5: Implement logging and confirm GREEN**

```bash
cargo test -p common hazard_observation_contract -- --nocapture
cargo test -p observation -- --skip zenoh
cargo test -p gateway transition_log -- --nocapture
git diff --check
```

Expected: all commands pass and old recordings remain readable.

- [ ] **Step 6: Review checkpoint**

Inspect golden v5 content and explicitly compare v4 historical action
round-trip. Do not commit.

---

### Task 5: SCCM/BCM-focused TUI

**Files:**
- Modify: `crates/tui_dashboard/src/view/driver.rs`
- Modify: `crates/tui_dashboard/src/view/engineer.rs`
- Modify: `crates/tui_dashboard/src/view/ledger_tail.rs`
- Modify: `crates/tui_dashboard/src/main.rs`

**Interfaces:**

```rust
fn format_observed_bool(value: PublishedObservedBool) -> &'static str {
    match value {
        PublishedObservedBool::Unknown => "UNKNOWN",
        PublishedObservedBool::Off => "OFF",
        PublishedObservedBool::On => "ON",
    }
}
```

- [ ] **Step 1: Add failing driver-pane tests**

Assert UNKNOWN before first observation and ON/OFF combinations afterward for
hazard, left request, and right request. Assert legacy visibility, headlamp,
weather, and wiper rows are absent from the active driver view.

- [ ] **Step 2: Add failing engineer/ledger tests**

Assert SCCM/BCM readiness and latest values are present. Require explicit
formatting for `HazardButtonObserved`, `LeftTurnRequestObserved`, and
`RightTurnRequestObserved`.

- [ ] **Step 3: Run TUI tests and confirm RED**

```bash
cargo test -p tui_dashboard -- --nocapture
```

Expected: current panes still render headlamp/wiper and omit SCCM/BCM.

- [ ] **Step 4: Implement the focused views**

Reuse the latest committed `PublishedTransitionRecord`; add no separate
duplicate-count transport and no direct TUI connection to Remotive.

- [ ] **Step 5: Run TUI tests and confirm GREEN**

```bash
cargo test -p tui_dashboard -- --nocapture
git diff --check
```

Expected: all TUI tests pass.

- [ ] **Step 6: Review checkpoint**

Check narrow and normal terminal widths and verify the display does not imply
physical lamp confirmation. Do not commit.

---

### Task 6: Cross-crate integration and strict automated regression

**Files:**
- Modify: `crates/gateway/tests/assembly_interaction_e2e.rs`
- Modify or create focused integration tests under `crates/gateway/tests/`
- Modify: `.superpowers/sdd/progress.md`
- Create: `.superpowers/sdd/phase-2-report.md`
- Create evidence logs under `.superpowers/sdd/phase-2-evidence/`

**Interfaces:**
- Consumes all prior task contracts.
- Produces one automated proof that broker observations can cross bridge-format
  CAN, Gateway projection, child actors, deterministic ledger, and TUI DTOs
  without any Twin actuation.

- [ ] **Step 1: Add a failing assembly interaction test**

Feed lifecycle startup, hazard frame `0x105`, duplicate hazard frames,
independent left frame `0x106`, independent right frame `0x107`, and duplicate
request frames through the public Gateway/Twin seam.
Assert:

- SCCM and BCM become ready;
- first valid OFF observations establish known state;
- ON observations update both children;
- left and right update independently in ingress order;
- ledger order follows ingress order;
- duplicates create no ledger rows;
- actions are empty;
- controlled PowerOff reaches `Off`.

- [ ] **Step 2: Run integration test and confirm RED**

```bash
cargo test -p gateway --test assembly_interaction_e2e -- --nocapture
```

Expected: public assembly still follows Phase I BCM computation or emits
duplicate/action rows.

- [ ] **Step 3: Complete minimal integration wiring**

Change only missing assembly seams exposed by the test. Do not add an egress
mode, runtime profile, publisher, replay runner, or physical-state vocabulary.

- [ ] **Step 4: Run focused cross-crate tests**

```bash
cargo test -p remotive_bridge
cargo test -p common
cargo test -p gateway
cargo test -p observation -- --skip zenoh
cargo test -p tui_dashboard
```

Expected: all commands exit 0.

- [ ] **Step 5: Run strict workspace verification**

```bash
cargo test --workspace -- --skip zenoh
cargo check --workspace
git diff --check
```

Expected: all commands exit 0. Record exact commands, outputs, and exit codes
under `.superpowers/sdd/phase-2-evidence/`.

- [ ] **Step 6: Review checkpoint**

Review the complete uncommitted diff against the design specification. Confirm
no user-owned untracked file or Remotive file appears in the diff or staging
area. Do not commit.

---

### Task 7: Unchanged live Remotive acceptance and final delivery gate

**Files:**
- Do not modify Remotive files.
- Add only evidence under `.superpowers/sdd/phase-2-evidence/`.
- Finalize `.superpowers/sdd/phase-2-report.md` and
  `.superpowers/sdd/progress.md`.

**Acceptance contract:**

```text
unchanged pytest
  -> SCCM HazardLightButton
  -> unchanged Python BCM
  -> BCM TurnLightControl
  -> FLCM pytest assertion
  -> same broker observations through Rust bridge
  -> Gateway
  -> SCCM/BCM Twin children
  -> schema-v5 ledger/live TUI
```

- [ ] **Step 1: Capture a full Remotive before-manifest**

Enumerate the entire RemotiveLabs root deterministically, including entry type,
mode, size, symlink target, and streamed SHA-256 for regular files. Store the
manifest outside the Remotive tree in Phase II evidence.

- [ ] **Step 2: Build and start unchanged `getting_started`**

Use the same proven common-root Remotive CLI strategy and CAN-over-UDP settings
from Phase I. Isolate only resources owned by this acceptance run; do not
remove or alter pre-existing Docker resources.

- [ ] **Step 3: Start TUI, Gateway, and bridge**

Use Gateway's UDS live observation path. Capture raw bridge stderr and prove
the three-signal readiness line occurs before bridge `PowerOn` appears in the
Gateway ledger.

- [ ] **Step 4: Run unchanged pytest**

Run the topology's existing:

```text
test_light_turns_on_when_hazard_button_is_pressed
```

Expected: 1 passed; FLCM receives both turn requests ON.

- [ ] **Step 5: Assert Twin state and deduplication**

From the schema-v5 ledger and Gateway output, prove:

- SCCM hazard is known and ON;
- BCM left/right requests are known and ON;
- Remotive's Python BCM, not the Twin, produced the output;
- initial/change records occur once per hazard, left, and right value;
- repeated hazard/left/right signals create no transition records;
- no `SetTurnLights` action is emitted;
- duplicate streak summaries are bounded.

- [ ] **Step 6: Verify the live TUI**

Capture the live TUI showing hazard ON, left ON, and right ON. Confirm it does
not display physical headlamp/wiper state or a duplicate count.

- [ ] **Step 7: Perform controlled shutdown and cleanup**

Send SIGINT to the bridge. Verify `RPM=0`, then `PowerOff`, bridge exit 0,
schema-valid completed capture, and no acceptance-owned process/container/
network remains.

- [ ] **Step 8: Prove Remotive remained unchanged**

Generate the after-manifest with the exact before-manifest algorithm. Require
equal entry counts, equal manifest SHA-256 values, and empty `diff -u`.

- [ ] **Step 9: Finalize the combined report**

The report must link:

- each RED and GREEN log;
- strict regression results;
- exact subscription readiness;
- unchanged pytest result;
- causal SCCM/BCM ledger evidence;
- duplicate suppression evidence;
- TUI evidence;
- cleanup evidence;
- full Remotive integrity proof;
- known concerns, if any.

- [ ] **Step 10: Final review and commit permission gate**

Run:

```bash
git status --short
git diff --check
git diff --stat
git diff --cached --exit-code
```

Present the complete design/implementation/test/acceptance result and proposed
staged file list to the user. Ask explicit permission before staging or making
one final significant commit. Never stage pre-existing user-owned untracked
files or evidence unless the user explicitly approves them.

