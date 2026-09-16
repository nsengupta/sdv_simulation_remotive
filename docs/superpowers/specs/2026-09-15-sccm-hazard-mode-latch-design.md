# SCCM hazard-mode latch (flip-switch) for Twin / TUI

**Date:** 2026-09-15  
**Branch baseline:** `remotive-integration` (Phase III side-by-side demo in place)  
**Status:** Design approved in brainstorming; awaiting implementation plan

## Goal

Make Twin/TUI **Hazard** reflect Remotive Hello World’s flip-switch **hazard mode**: ON after a button press until the next press toggles it OFF—even though the SCCM hazard **button wire** is only a short Restbus pulse.

Remotive remains the sole source of button presses and of BCM blink / lamp requests. The Twin does not actuate Remotive; it latches mode from observed button rising edges in the **SCCM actor**.

## Problem

Phase II/III observation mirrors the hazard **button** wire into TUI `Hazard:`. After a Jupyter pulse, the wire returns OFF while Remotive BCM stays in hazard mode and blinks left/right. TUI then shows `Hazard: OFF` while the cute car still blinks—correct for the wire, misleading for “hazards armed.”

## Architecture

```text
Jupyter / Remotive SCCM
  → HazardLightButton rising edge (pulse)
  → Remotive BCM toggles hazard mode + blinks L/R turn requests
  → remotive_bridge (unchanged three signals)
  → Gateway → Twin SCCM actor
       hazard_button  := last wire (pulse-faithful)
       hazard_mode    := flip on rising edge (latched)
  → TUI Driver "Hazard:" := hazard_mode (steady ON/OFF)
  → TUI Left/Right     := BCM observations (may blink)
```

### Ownership

| Component | Owns |
|-----------|------|
| Remotive Hello World | Button presses; BCM hazard blink policy; L/R turn request signaling (including stop-blink on second press) |
| Twin SCCM actor | Latched `hazard_mode` from observed button rising edges; last `hazard_button` wire |
| Twin TUI | Steady `Hazard:` from `hazard_mode`; L/R from BCM |
| `remotive_bridge` | Unchanged three-signal subscribe only |

## Approach (locked)

**Approach A / SCCM latch (Approach 1):** On each rising edge of observed `HazardLightButton` (`previous ≠ ON` and new value `true`), toggle `hazard_mode`. Falling edges and repeated OFF do not toggle. Driver TUI shows **only** latched mode (steady), not a composite “mode + button” string.

Rejected for this stage:

- Infer mode only from L∩R blink (no button latch)
- TUI-only latch (not Twin truth)
- Blinking `Hazard:` label (cosmetic or L∩R-tied)—**revisit later**
- Twin-emitted blink phase field

## SCCM data model

Extend `SccmContext` (held by `SccmActor`):

| Field | Meaning |
|-------|---------|
| `hazard_button` | Last observed wire (`UNKNOWN` / `OFF` / `ON`) |
| `hazard_mode` (new) | Latched flip-switch; `OFF` after PowerOn / BecomeOn |

On `SccmMessage::HazardButtonObserved(value)`:

1. Classify disposition vs previous `hazard_button` (duplicate suppression unchanged).
2. If rising edge (`hazard_button` was not ON and `value == true`): toggle `hazard_mode`.
3. Set `hazard_button` from `value` (and keep any legacy `hazard_button_on` mirror in sync if still required).

Lifecycle:

- PowerOn / `BecomeOn`: `hazard_mode = OFF`; `hazard_button = UNKNOWN` (default).
- PowerOff / `BecomeOff`: `hazard_mode = OFF` (no sticky mode across sessions).

## Publish / TUI

- Published SCCM context includes `hazard_button` and `hazard_mode`.
- Driver pane `Hazard:` ← **`hazard_mode` only** (steady `UNKNOWN`/`OFF`/`ON` as applicable; mode starts OFF after PowerOn, not UNKNOWN, unless product chooses Unknown before first edge—**default OFF after PowerOn**).
- Left request / Right request unchanged.
- Engineer SCCM line: primary hazard state is **mode**; raw button may remain visible for debugging.
- Ledger events remain `HazardButtonObserved(bool)` for accepted wire changes; each row’s context includes both fields after SCCM applies the edge.

Historical schema: older ledgers without `hazard_mode` read as mode absent/OFF.

## Second press and Remotive lamps

On the second Remotive hazard press (rising edge again):

1. Remotive BCM exits hazard mode, stops blink ticker, drives L/R turn requests OFF (cute car stops blinking)—**Remotive ECU signaling**, not Twin actuation.
2. The same observed rising edge toggles Twin `hazard_mode` → OFF so TUI `Hazard:` goes OFF.

## Explicitly deferred

- Blinking driver `Hazard:` paint (steady ON while mode armed; revisit later).
- Later phase: simulate hazard mode ON while **one** side does not blink. Remotive Python may misbehave; Twin/TUI must still indicate **car-wide inconsistency** (mode vs asymmetric L/R), because the Twin shows whole-car state. Not part of this latch implementation.
- Twin→Remotive hazard actuation.
- New bridge subscriptions beyond the three Phase II identities.
- Inferring mode from L/R alone.

## Acceptance

1. First Jupyter hazard pulse → TUI `Hazard: ON` and **stays ON** after button wire returns OFF; cute car and/or L/R may blink.
2. Second pulse → TUI `Hazard: OFF`; Remotive stops blink (L/R observed OFF).
3. No Twin `SetTurnLights` (or equivalent) into Remotive.
4. Bridge still lists exactly the three Phase II signal identities.
5. PowerOn resets `hazard_mode` to OFF.
6. Focused SCCM/TUI tests cover rising-edge toggle, no toggle on falling edge, and driver pane binding to mode not wire.

## Relationship to Phase III

Phase III proved side-by-side observation with pulse-faithful button display. This change keeps that observation path and adds SCCM **mode** so TUI Hazard matches flip-switch semantics without widening the Remotive subscribe set.
