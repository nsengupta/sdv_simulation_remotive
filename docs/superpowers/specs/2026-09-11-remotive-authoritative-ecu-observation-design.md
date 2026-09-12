# Phase II: Authoritative Remotive ECU Observation

## Objective

Phase II turns the Phase I connectivity proof into a meaningful integration
boundary. Remotive remains authoritative for ECU behavior, while the Rust
Digital Twin builds deterministic, coherent, observable car-wide state from
the signals produced by those ECUs.

The target remains Remotive's unchanged `getting_started` hazard scenario.
Phase II does not add another implementation of Remotive's BCM state machine
and does not add a Twin-to-Remotive publication path.

## Responsibility boundary

### Remotive

- Runs the unchanged `getting_started` topology and Python BCM model.
- Uses the unchanged pytest/Restbus stimulus to publish the SCCM hazard-button
  input.
- Computes BCM left and right turn-light requests.
- Routes `TurnLightControl` to FLCM, where the unchanged pytest verifies it.

### Rust bridge

- Replaces the original Emulator as the integration runtime stimulus owner.
- Retains the Phase I lifecycle and RPM behavior unchanged: `PowerOn`, the
  synthetic RPM profile, and controlled `RPM=0`/`PowerOff` shutdown.
- Establishes one Remotive subscription containing all required signal
  identities.
- Translates each broker signal into a typed internal SocketCAN observation
  frame as it arrives.
- Contains no vehicle policy or ECU state-machine logic.

### Digital Twin

- Receives lifecycle, RPM, and Remotive-derived observations through Gateway.
- Owns the top-level lifecycle FSM, deterministic event processing, actor
  conversation, reorder barrier, aggregate context, transition history, and
  observation delivery.
- Uses child actors to hold externally observed ECU state, not to reproduce
  Remotive ECU decisions.

### TUI

- Remains an observation-only consumer.
- Displays the Twin's latest coherent SCCM and BCM state.

## Runtime architecture

```text
unchanged pytest
    -> SCCM Restbus
    -> HazardLightButton
    -> Remotive Python BCM
    -> TurnLightControl
    +-> unchanged pytest assertion at FLCM
    `-> Rust bridge
          -> SocketCAN
          -> Gateway
          -> Twin actors
          -> persisted/live observation
          -> TUI
```

Phase I remains recoverable at Git commit `4cc75fb`; it is not retained as a
separately selectable runtime profile. Phase II evolves the same source tree
and removes Twin-side BCM computation from the active integration.

## Signal contracts

The bridge subscribes through one `SubscriberConfig` to exactly:

- `SCCM-DriverCan0:HazardLightButton.HazardLightButton`
- `BCM-BodyCan0:TurnLightControl.LeftTurnLightRequest`
- `BCM-BodyCan0:TurnLightControl.RightTurnLightRequest`

The Remotive DBC identifies the source messages as hazard message 100 decimal
and `TurnLightControl` message 103 decimal. The broker API delivers the
subscribed signal values, however, and does not require the bridge to
reconstruct those source CAN frames.

The bridge therefore maps each signal independently onto the existing internal
bridge-to-Gateway SocketCAN carrier:

- Hazard button uses the existing Phase I internal ID `0x105`.
- Left turn request uses new internal ID `0x106`.
- Right turn request uses new internal ID `0x107`.

Each internal observation frame carries one strict boolean value. The bridge
preserves broker delivery order when writing these frames to SocketCAN and
does not wait for, combine, or cache complementary left/right values. Malformed
payloads are rejected with bounded diagnostics. The bridge does not infer that
a particular output was caused by a particular input.

Gateway maps the three frames independently to:

- `HazardButtonObserved(bool)`
- `LeftTurnRequestObserved(bool)`
- `RightTurnRequestObserved(bool)`

The BCM requests are commanded ECU state. Physical lamp state is not part of
the Phase II integration vocabulary.

## Actor model and deterministic processing

The active integration children are:

- `SccmActor`, holding the observed hazard-button value.
- `BcmActor`, holding the observed left and right turn-light requests.

Neither child implements the corresponding Remotive ECU policy. The parent
assembles their replies into coherent car-wide state.

Every decoded observation creates a typed turn routed to its owning child. The
child classifies it as an initial value, a changed value, or a duplicate. Left
and right BCM signals remain separate turns and are aggregated only in BCM
context. Replies use the existing turn barrier and reorder path so changed
ledger commits remain deterministic even when child replies complete out of
order.

The bridge owns lifecycle stimulation. The Twin receives those events and owns
the resulting lifecycle state and transition history. Startup gating and
controlled shutdown remain active.

The legacy headlamp and wiper actors and physical-state fields are inactive and
not displayed. Their code and deprecated schema fields remain available only
for historical compatibility. Historical `SetTurnLights` actions remain
readable but are not produced by the Phase II observation path.

## Duplicate handling

Restbus can repeatedly emit the current signal value. The SCCM actor keeps one
hazard streak, and the BCM actor keeps independent left and right streaks:

- Before its first valid observation, a child exposes the value as unknown.
- The first valid observation establishes ON/OFF state, produces one
  transition record, and starts the duplicate streak at zero.
- A changed value updates context and produces one transition record.
- An identical value increments the streak but produces no context change,
  domain action, transition record, or live-observation message.
- The next changed value reports the completed streak, resets its counter, and
  starts the new value's streak at zero.
- Any remaining streak is summarized during controlled shutdown.

On change, logs show the completed value, its duplicate count, and the new
value using bounded output. The exact live duplicate count is not sent to the
TUI because doing so would require a new high-frequency status transport.

## Observation schema and TUI

Phase II introduces schema v5 with:

- `HazardButtonObserved(bool)`
- `LeftTurnRequestObserved(bool)`
- `RightTurnRequestObserved(bool)`
- Aggregate post-turn SCCM and BCM observed state.

Readers and fixtures for schemas v1 through v4 remain supported. New records
must serialize and deserialize losslessly; Phase II does not add TUI playback
or deterministic FSM re-execution.

The TUI driver view displays:

- Hazard button: UNKNOWN until first observed, then ON/OFF.
- Left request: UNKNOWN until first observed, then ON/OFF.
- Right request: UNKNOWN until first observed, then ON/OFF.

The engineer view displays SCCM and BCM child readiness and their latest
observed values. The ledger tail formats both observed events explicitly.
Legacy headlamp and wiper physical state is not displayed in this integration.

## Subscription readiness

The bridge constructs the exact three-signal subscription and awaits
`subscribe_to_signals`. Only after the call succeeds and returns a stream does
the bridge:

1. log a readiness line listing all three identities;
2. return the connected source to the session; and
3. permit `run_session` to write `PowerOn`.

This proves that Remotive accepted the exact request before Twin startup.
Actual hazard and left/right events observed during live acceptance prove that
the identities also produce data.

## Failure and shutdown behavior

- A required-subscription failure prevents `PowerOn`.
- Broker stream termination or CAN write failure ends the bridge session with
  a nonzero result after the existing controlled shutdown trailer is attempted.
- Malformed signal values are rejected with bounded diagnostics.
- Phase II adds no Twin-to-Remotive publisher and no physical actuator
  confirmation.
- Cleanup must leave no bridge, Gateway, topology, test, capture, or TUI
  process owned by the acceptance run.

## Verification

All implementation work follows behavior-focused RED-to-GREEN tests before
production changes.

Automated coverage includes:

- Exact multi-signal subscription configuration and readiness-before-PowerOn.
- Independent hazard, left-request, and right-request decoding.
- CAN frame 100 and 103 contracts.
- SCCM and BCM observed-state child behavior.
- Deterministic parent commit ordering through the existing barrier/reorder
  path.
- Independent duplicate streaks, reset-on-change, bounded summaries, and zero
  duplicate transition emission.
- Schema v5 round-trip and v1-v4 compatibility.
- TUI SCCM/BCM rendering.
- Strict non-Zenoh workspace regressions.

Live acceptance:

1. Run unchanged Remotive `getting_started`, including its Python BCM.
2. Start the TUI, Gateway, and bridge.
3. Verify the exact subscription is accepted before bridge `PowerOn`.
4. Run the unchanged Remotive pytest.
5. Verify pytest observes `TurnLightControl` at `FLCM-BodyCan0`.
6. Verify the Twin independently observes hazard ON from SCCM and left/right ON
   from BCM.
7. Verify exactly one changed transition for hazard, left request, and right
   request despite repeated Restbus frames.
8. Verify the TUI displays hazard, left, and right as ON.
9. Stop the bridge and verify controlled `RPM=0`/`PowerOff`, a schema-valid
   capture, and complete cleanup.
10. Compare full before/after Remotive manifests to prove Remotive remained
    unchanged.

## Explicitly deferred

- Twin-to-Remotive publication.
- Physical lamp state and actuator confirmation.
- Ambient visibility and other bridge-generated environment signals.
- Additional Remotive signals and generic multi-ECU routing.
- RemotiveCar Hello World and the side-by-side 3D car/Twin TUI demonstration.
- Fault injection and missing, late, or inconsistent ECU-response detection.
- TUI replay and deterministic execution replay.

These are candidates for Phase III and later phases after the authoritative
post-ECU observation boundary is proven.
