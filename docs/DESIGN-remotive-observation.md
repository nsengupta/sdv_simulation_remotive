# Remotive observation design

**Status:** Implemented on Twin `remotive-integration` with Remotive branch
`demo/flcm-lamp-status-feedback` (observation schema **v8**; live demo verified).

This document is the durable design home for the RemotiveCar Hello World
integration. Narrative overview: [`../README.md`](../README.md). Catalogue:
[`design-documents.md`](design-documents.md).

---

## Goal

Show Twin integration value: **BCM still commands front low beams** and the
**3D cute car still looks ON**, while **FLCM** reports lamp **Status** (Ok / Fail)
and cyclic **liveness**. When FLCM fails or goes silent, **only the Twin TUI
warns** — Remotive BCM and cute car stay unaware.

Remotive owns fault inject on a PR-friendly branch. Twin **observes** only
(no Twin→Remotive actuation).

---

## Integration boundary

Interact at **RemotiveBroker signal level** (gRPC via `remotivelabs-broker`).
Do not embed the Twin inside Remotive actors or Docker networking.

```text
RemotiveCar Hello World  →  RemotiveBroker  →  remotive_bridge  →  vcan
                                                         →  Gateway twin
                                                         →  observation (UDS/files)
                                                         →  TUI
```

**Remotive → Twin:** subscribe selected signals; Twin owns its own contexts,
ledger, and fault semantics.

**Twin → Remotive:** none in this exercise. Any future publish must use signals
the topology already expects — not invented for symmetry.

---

## Problem (why FLCM status feedback)

Hello World today (before the FLCM stub):

- BodyCan light *request* frames are **sent by BCM**; cute car maps those requests.
- Empty `FLCM: {}` had **no behavioral model** and **no FLCM-authored TX**.
- No Hello World brain times out another ECU’s “I-am-working” signal.

So the webpage can look healthy while a front lamp ECU is dead or lying. Twin
observation of **FLCM truth** is the value-add.

---

## Architecture

```text
Jupyter light stalk → SCCM → BCM
  → BodyCan LowBeamLightControl *Request*
       ├→ 3D cute car (UNCHANGED mapping) → beams look ON
       └→ FLCM stub (Remotive demo branch)
            → BodyCan LowBeamLightStatus L/R (50 ms cycle)
                 ├→ BCM / GWM: do NOT consume (blind)
                 └→ remotive_bridge → Gateway → Twin → TUI Warning

Fault inject (Jupyter FLCM Ok | Fail | Silent):
  Status=Fail and/or mute cyclic TX
  BCM requests keep flowing → cute car still ON
```

| Component | Owns |
|-----------|------|
| Remotive Hello World (`demo/flcm-lamp-status-feedback`) | DBC status frame; FLCM stub; `flcm_fault` inject; Jupyter buttons |
| `remotive_bridge` | Five locked subscriptions; carriers `0x105`–`0x109`; PowerOn / RPM / PowerOff |
| Gateway / Twin | FLCM context, silence watchdog, `FlcmLampFault` Warning |
| TUI | Attended Front Light + Low Beam rows; Notice Warning |

### Locked Remotive identities

| Item | Value |
|------|--------|
| Frame | `LowBeamLightStatus` |
| DBC `BO_` | `111` |
| Sender | `FLCM` |
| Cycle | **50** ms |
| Signals | `LeftLowBeamLightStatus`, `RightLowBeamLightStatus` |
| Encoding | `0` = Ok, `1` = Fail |
| Namespace | `FLCM-BodyCan0` |
| Fault control | `flcm_fault` with `ok` \| `fail` \| `silent` |

### Locked Twin carriers

| Signal | CAN ID | Payload |
|--------|--------|---------|
| Hazard button | `0x105` | Two-byte boolean (`[1,0]` / `[0,0]`) |
| Left / right turn request | `0x106` / `0x107` | same two-byte boolean style |
| Left / right low-beam status | `0x108` / `0x109` | `[1,0]` = Ok, `[0,0]` = Fail |

### Bridge subscribe order (final)

1. `SCCM-DriverCan0:HazardLightButton.HazardLightButton`
2. `BCM-BodyCan0:TurnLightControl.LeftTurnLightRequest`
3. `BCM-BodyCan0:TurnLightControl.RightTurnLightRequest`
4. `FLCM-BodyCan0:LowBeamLightStatus.LeftLowBeamLightStatus`
5. `FLCM-BodyCan0:LowBeamLightStatus.RightLowBeamLightStatus`

### Twin rules (shipped)

- **Warning** on Status=Fail **or** silence longer than **T = 500 ms**.
- Silence watchdog arms only after the **first observed** FLCM status — never at
  PowerOn alone (no FLCM TX → Unknown / not-silent).
- Clear Warning when both sides Ok and not silent (`is_confirmed_healthy`).
- Low-beam rows bind to `PublishedFlcmContext`, not BCM requests / headlamp.
- Observation schema **v8** (FLCM context + named ledger events); v1–v7 readable.
- Do **not** reuse `FrontHeadlampActuationIncomplete` for this story.
- Do **not** teach BCM/GWM or remap `3d_car_mapping.yaml`.

### Remotive topology changes (summary)

Offered as a PR on branch `demo/flcm-lamp-status-feedback`:

- `body_can.dbc` — `BO_ 111 LowBeamLightStatus`
- FLCM Python stub + `flcm.bm.instance.yaml`
- Hello World include (replace empty `FLCM: {}`)
- Jupyter **FLCM Ok / Fail / Silent** buttons

---

## What this integration builds on

| Capability | Result |
|------------|--------|
| Broker attach | Broker → `remotive_bridge` → `vcan` → Gateway; bridge-owned lifecycle |
| Observed controls | Authoritative SCCM hazard + BCM L/R turn requests; no Twin→Remotive turn actuation |
| Side-by-side demo | Hello World + 3D car + TUI; attended hazard / turns |
| Hazard latch | SCCM `hazard_mode` so TUI Hazard stays ON after a Restbus pulse |
| FLCM health | Status + silence; Twin-only Warning; cute car stays ON on BCM requests |

---

## Explicitly deferred

- Teaching BCM (or remapping cute car) to FLCM Status
- RLCM↔RL slave-reply validation; SCCM brake E2E timeouts
- Asymmetric hazard; Twin→Remotive actuation; environment-from-ECU
- Blinking Hazard paint; continuous stimulus moved fully into Remotive

---

## Acceptance (met)

1. Remotive edits only on the dedicated Remotive branch.
2. Low beams commanded ON → cute car ON; TUI FLCM Status OK when healthy.
3. Fault inject → TUI Warning (Fail and/or silence within T) while cute car remains ON.
4. Resume → Warning clears.
5. Hazard button + latch still work with the five-signal bridge.
6. No Twin actuation into Remotive.

---

## Operator runbook

### Placeholders

| Placeholder | Meaning |
|-------------|---------|
| `$REMOTIVE_EXAMPLES` | Clone of `remotivelabs-topology-examples` |
| `$TWIN_REPO` | This Twin repo |
| `$REMOTIVE_BRANCH` | `demo/flcm-lamp-status-feedback` |
| `$BROKER_URL` | `http://127.0.0.1:50051` |
| `$CAN_IFACE` | `vcan0` |
| `$COMPOSE_PROJECT` | `remotive_car_hello_world` |
| `$HELLO_WORLD_BUILD` | `$REMOTIVE_EXAMPLES/remotive_car/build/remotive_car_hello_world` |

### Start

```bash
sudo systemctl start remotivebusd
ls -l /run/docker/plugins/remotivebus.sock

cd "$REMOTIVE_EXAMPLES"
git checkout "$REMOTIVE_BRANCH"
remotive topology build --no-workspace \
  -f remotive_car/instances/hello_world/main.instance.yaml \
  remotive_car/build
# build from examples repo root — not from inside $HELLO_WORLD_BUILD

cd "$HELLO_WORLD_BUILD"
docker compose -p "$COMPOSE_PROJECT" -f docker-compose.yml \
  --profile jupyter --profile 3dcar up --build -d

# Twin (three terminals; Gateway first)
cd "$TWIN_REPO"
sudo ip link add dev "$CAN_IFACE" type vcan 2>/dev/null || true
sudo ip link set up "$CAN_IFACE"
cargo run -p gateway -- --uds observation.sock --connect-timeout 120
cargo run -p tui_dashboard -- --uds observation.sock
cargo run -p remotive_bridge -- --broker-url "$BROKER_URL" --can-interface "$CAN_IFACE"
```

Expect one subscribed signal per line before relying on PowerOn. Jupyter:
`http://127.0.0.1:8888` (token `remotivelabs`) → `car.ipynb` → **Restart & Run All**.
Cute car: `http://127.0.0.1:3000/car`.

If the car page stays on **Loading**, once per container:

```bash
docker exec "${COMPOSE_PROJECT}-3d-car-1" \
  cp /usr/share/nginx/html/remotivecar5.glb /usr/share/nginx/html/remotivecar3.glb
```

### Demo order (verified)

| Step | Jupyter action | Expect |
|------|----------------|--------|
| 1 | Light stalk → **Low Beam** | Cute car ON; TUI low beams OK; no FLCM Warning |
| 2 | **FLCM Fail** | TUI Warning + FAIL; cute car **still ON** |
| 3 | **FLCM Ok** | Warning clears; OK |
| 4 | **FLCM Silent** | Within ~500 ms Warning / stale; cute car **still ON** |
| 5 | **FLCM Ok** | Healthy again |
| 6 | Hazard **`!`** | Cute car blinks; TUI Hazard ON (latched) |

### Stop

```bash
# Ctrl+C remotive_bridge, Gateway, TUI
cd "$HELLO_WORLD_BUILD"
docker compose -p "$COMPOSE_PROJECT" -f docker-compose.yml \
  --profile jupyter --profile 3dcar down
sudo systemctl stop remotivebusd
```

Do not stop `remotivebusd` while Hello World containers are still up.
