# Phase II Stage 2 — local runbook

Repeatable steps for the current uncommitted tree on `remotive-integration`.
This is not yet the final Phase II acceptance run. Schema v5 and TUI SCCM/BCM
panes are not implemented. Use Gateway transition output to inspect Twin state.

Do not modify files under either RemotiveLabs tree.

## What is running

```text
Remotive getting_started pytest
  -> SCCM HazardLightButton
  -> Remotive Python BCM
  -> BCM left/right TurnLightControl
  -> remotive_bridge (subscribe + vcan0)
  -> Gateway Twin (SCCM/BCM children)
  -> stdout transition log
```

The original Emulator is not used. The bridge owns `PowerOn`, the RPM profile,
and `RPM=0` / `PowerOff` on SIGINT.

Internal CAN IDs on `vcan0`:

| Signal | ID |
|--------|----|
| Lifecycle PowerOn/PowerOff | `0x100` |
| Engine RPM | `0x102` |
| Hazard button | `0x105` |
| Left turn request | `0x106` |
| Right turn request | `0x107` |

## Prerequisites

- Linux host with SocketCAN (`can-utils`)
- Docker and Remotive CLI `0.29.1` for the live Remotive path
- Working tree:

```text
/home/nirmalya/Workspace-Rust/Eclipse-SDV/Self-handson-project/sdv_simulation_remotive
```

- Unchanged Remotive example:

```text
/home/nirmalya/Workspace-Rust/Eclipse-SDV/RemotiveLabs/remotivelabs-topology-examples/getting_started
```

## 1. Prepare `vcan0`

```bash
sudo ip link add dev vcan0 type vcan 2>/dev/null || true
sudo ip link set up vcan0
ip link show vcan0
```

Optional watch in a spare terminal:

```bash
candump -L vcan0
```

## 2. Twin-only smoke (no Remotive)

Use this when you only want to exercise Gateway + bridge CAN contracts.

Terminal A — Gateway ledger:

```bash
cd /home/nirmalya/Workspace-Rust/Eclipse-SDV/Self-handson-project/sdv_simulation_remotive
cargo run -p gateway -- --print-transitions-only
```

Wait until it prints that it is listening on `vcan0`.

Terminal B — inject lifecycle, then independent observations:

```bash
# PowerOn
cansend vcan0 100#0100000000000000

# Hazard ON  (0x105, payload [1, 0])
cansend vcan0 105#0100

# Left request ON  (0x106)
cansend vcan0 106#0100

# Right request ON (0x107)
cansend vcan0 107#0100

# Repeats should not add another transition row
cansend vcan0 105#0100
cansend vcan0 106#0100
cansend vcan0 107#0100

# Complementary values
cansend vcan0 105#0000
cansend vcan0 106#0000
cansend vcan0 107#0000

# PowerOff
cansend vcan0 102#0000
cansend vcan0 100#0000000000000000
```

Expected in Gateway:

- `PowerOn` then SCCM/BCM startup to `Idle`
- one row for first hazard ON, first left ON, first right ON
- no extra rows for the three repeats
- one row each for the complementary OFF values
- `PowerOff` to `Off`

Until Stage 3, published left/right event names may still appear as
`TimerTick`. The BCM context fields are the trustworthy observation.

## 3. Remotive `getting_started` side

Do not edit files inside either RemotiveLabs tree. All commands below either
read those trees or write generated output under `getting_started/build`.

Useful paths:

```bash
export ECLIPSE_SDV=/home/nirmalya/Workspace-Rust/Eclipse-SDV
export GETTING_STARTED=$ECLIPSE_SDV/RemotiveLabs/remotivelabs-topology-examples/getting_started
export COMPOSE_DIR=$GETTING_STARTED/build/getting-started
export TWIN_REPO=$ECLIPSE_SDV/Self-handson-project/sdv_simulation_remotive
export COMPOSE_OVERRIDE=$TWIN_REPO/docs/superpowers/runbooks/getting_started-host-bridge.override.yml
export COMPOSE_PROJECT=getting_started_local
```

The Twin-owned override only renames the host Linux bridge to `gs_local_ctl`.
It does not change Remotive services, signals, profiles, or tests. Use it so
the run does not collide with the pre-existing
`remotive_car_hello_world` `control_network`. Do not delete that network.

Terminal map for a full live run:

| Terminal | Role |
|----------|------|
| 1 | Remotive compose: build, `up -d`, later `down` |
| 2 | Optional Remotive logs |
| 3 | Gateway |
| 4 | `remotive_bridge` |
| 5 | Remotive `tester` pytest |
| 6 | Optional `candump -L vcan0` |

### 3a. Check the Remotive CLI

```bash
remotive --version
# Expected around 0.29.1
```

Optional resolved-instance view (read-only):

```bash
cd "$ECLIPSE_SDV"
remotive topology show instance --no-workspace \
  RemotiveLabs/remotivelabs-topology-examples/getting_started/instances/main.instance.yaml
```

### 3b. Build the unchanged topology

Remotive CLI 0.29.1 requires source and output under one mounted working
directory. Absolute input paths fail with E020/E021. Build from the common
`Eclipse-SDV` parent using relative paths.

Current CLI versions also refuse to run unless a Remotive workspace exists.
Do **not** run `remotive topology workspace init` under RemotiveLabs; that
writes a `.remotive` cache into the Remotive tree. Pass `--no-workspace`
instead. That uses the current directory as a temporary implicit workspace
and disables caching for this run.

```bash
cd "$ECLIPSE_SDV"

remotive topology build --no-workspace \
  -f RemotiveLabs/remotivelabs-topology-examples/getting_started/instances/main.instance.yaml \
  -f RemotiveLabs/remotivelabs-topology-examples/getting_started/settings/can_over_udp.settings.instance.yaml \
  RemotiveLabs/remotivelabs-topology-examples/getting_started/build
```

Expected: `Generated topology at: .../getting_started/build/getting-started`.
The subdirectory name comes from `instances/main.instance.yaml` (`name: getting-started`).
Do not look for `build/getting_started`.

Confirm the generated compose file exists:

```bash
ls -l "$COMPOSE_DIR/docker-compose.yml"
```

### 3c. Start Remotive and keep it running

Start without the `tester` profile so SCCM, Python BCM, FLCM, and the brokers
stay up. The Twin and the pytest container attach later.

```bash
cd "$COMPOSE_DIR"

docker compose -p "$COMPOSE_PROJECT" \
  -f docker-compose.yml \
  -f "$COMPOSE_OVERRIDE" \
  up --build -d

docker compose -p "$COMPOSE_PROJECT" -f docker-compose.yml -f "$COMPOSE_OVERRIDE" ps
```

Wait until these are `healthy` or `Up`:

```text
topology-broker.com
SCCM-broker.com
BCM-broker.com
FLCM-broker.com
bcm
sccm-mock
topology-api
```

`topology-api` should publish the host broker:

```text
0.0.0.0:50051->8080/tcp
```

Confirm the host URL the Rust bridge uses:

```bash
ss -ltn | grep 50051
```

Do **not** `curl` `http://127.0.0.1:50051`. That port is gRPC behind nginx.
An HTTP GET produces this broker log and is not a topology failure:

```text
topology-broker.com ... Exception raised while handling /:
** (GRPC.RPCError) Message is malformed.
```

The bridge URL is still:

```text
http://127.0.0.1:50051
```

Do **not** use `docker compose logs -f` as a health check. That command only
reads already-written container logs. It does not send gRPC. On first attach
it reprints startup history, including this one-time broker line:

```text
topology-broker.com ... Exception raised while handling /:
** (GRPC.RPCError) Message is malformed.
```

That line is Remotive's broker rejecting an HTTP GET to `/`. Typical sources
are the generated Docker healthcheck (`curl http://topology-broker.com:50051`)
or any host probe of port `50051`. Phase I saw the same line while brokers
stayed `healthy`. Ignore it and continue.

Use compose status, not a live log follower:

```bash
cd "$COMPOSE_DIR"
docker compose -p "$COMPOSE_PROJECT" -f docker-compose.yml -f "$COMPOSE_OVERRIDE" ps
docker compose -p "$COMPOSE_PROJECT" -f docker-compose.yml -f "$COMPOSE_OVERRIDE" \
  logs --tail=20 bcm sccm-mock
```

Expected snippets:

```text
sccm-mock ... Starting ECUMock at http://SCCM-broker.com:50051
bcm       ... python -m bcm
```

### 3d. Optional Remotive-only pytest

Use this to prove Remotive itself before starting Twin processes. The tester
container is ephemeral; the rest of the topology stays up.

```bash
cd "$COMPOSE_DIR"

docker compose -p "$COMPOSE_PROJECT" \
  -f docker-compose.yml \
  -f "$COMPOSE_OVERRIDE" \
  --profile tester run --rm tester
```

Expected:

```text
test_light_turns_on_when_hazard_button_is_pressed PASSED
1 passed
```

That is Remotive's own check: SCCM Restbus hazard ON → Python BCM →
`TurnLightControl` left/right ON at `FLCM-BodyCan0`.

To run the official one-shot Remotive README path instead (topology exits
when the tester exits), use:

```bash
cd "$COMPOSE_DIR"
docker compose -p "$COMPOSE_PROJECT" \
  -f docker-compose.yml \
  -f "$COMPOSE_OVERRIDE" \
  --profile tester up --abort-on-container-exit --build
```

Do not use that one-shot form when you also want the Twin attached. Keep
section 3c running, then continue below.

### 3e. Start Gateway

Default bridge RPM uses `--rpm-clamp 1000` (`RPM_DRIVING_THRESHOLD`), so the
Twin stays in `Idle`. Raise it (for example `--rpm-clamp 3000`) when the demo
needs motion / `Driving`. `UpdateRpm` self-loops can still appear; they are not
pytest. Restart Gateway with those lines stripped if they get in the way.
`--line-buffered` is required because `grep` otherwise waits for a full block:

```bash
cd /home/nirmalya/Workspace-Rust/Eclipse-SDV/Self-handson-project/sdv_simulation_remotive
cargo run -p gateway -- --print-transitions-only 2>&1 \
  | grep --line-buffered -v 'UpdateRpm'
```

Wait for:

```text
⚡ Gateway on vcan0
[gateway] actuation egress: null drain
```

To keep only pytest-relevant rows after that:

```bash
cargo run -p gateway -- --print-transitions-only 2>&1 \
  | grep --line-buffered -E 'PowerOn|PowerOff|HazardButton|TimerTick|hazard_button_on=true|left_turn_request_on=true|right_turn_request_on=true|Preparing|Idle'
```

Until Stage 3, a hazard observation prints as `HazardButtonChanged(true)`.
Independent left/right observations print as `TimerTick`, with the BCM flags
changing to `true`. Do not treat a lone `TimerTick` as a clock tick unless
those BCM flags also change.

If this Gateway prints only the banner and no `PowerOn`, it came up after the
bridge already armed the previous Twin. A new Gateway starts in `Off` and
ignores hazard/left/right until it receives a fresh `PowerOn`. Remotive
pytest can still pass; that test never talks to the Twin.

Ctrl+C the old bridge, keep this Gateway, start a new bridge, wait for
`PowerOn` / `Idle` in Gateway, then run pytest.

### 3f. Start the bridge after Gateway is up

```bash
cd "$TWIN_REPO"
cargo run -p remotive_bridge -- \
  --broker-url http://127.0.0.1:50051 \
  --can-interface vcan0
# Optional motion: add --rpm-clamp 3000 (default 1000 keeps Twin Idle)
```

Required before any Twin `PowerOn`:

```text
[remotive_bridge] connected; subscribed signals=SCCM-DriverCan0:HazardLightButton.HazardLightButton,BCM-BodyCan0:TurnLightControl.LeftTurnLightRequest,BCM-BodyCan0:TurnLightControl.RightTurnLightRequest
```

The bridge then stays silent on success. It does not print each forwarded
hazard/left/right frame. Rejections print only on power-of-two counts.
Watch Gateway, not the bridge, for pytest.

Then Gateway should show `PowerOn` → `PreparingToStart` → `Idle`, then the
subscribe snapshot of Restbus OFF:

```text
HazardButtonChanged(false)  Idle → Idle  ...  sccm.hazard_button_on=false
TimerTick                   Idle → Idle  ...  bcm.left_turn_request_on=false
TimerTick                   Idle → Idle  ...  bcm.right_turn_request_on=false
```

Missing sequence numbers are filtered `UpdateRpm` rows.

If a previous pytest left Remotive Restbus at hazard/left/right ON, the
bridge subscribe snapshot delivers those values immediately. That looks like:

```text
TimerTick                  ...  bcm.left_turn_request_on=true
TimerTick                  ...  bcm.right_turn_request_on=true
HazardButtonChanged(true)  ...  sccm.hazard_button_on=true
```

A later pytest that sets the same `true` values is a Twin duplicate and must
not add rows. Remotive pytest still passes. To see pytest move the Twin from
OFF to ON, restart the Remotive compose project first so Restbus starts at 0.

### 3g. Run the unchanged Remotive pytest with the Twin attached

Keep Remotive from 3c, Gateway, and the bridge running. In a new terminal:

```bash
cd "$COMPOSE_DIR"

docker compose -p "$COMPOSE_PROJECT" \
  -f docker-compose.yml \
  -f "$COMPOSE_OVERRIDE" \
  --profile tester run --rm tester
```

Expected: `test_light_turns_on_when_hazard_button_is_pressed` passes.

In the filtered Gateway console, pytest should append exactly one change
burst. A live capture on this tree looked like:

```text
sccm.hazard change completed_duplicates=566 new=true
HazardButtonChanged(true)  ...  sccm.hazard_button_on=true  bcm.left_turn_request_on=false  bcm.right_turn_request_on=false
bcm.right change completed_duplicates=566 new=true
bcm.left change completed_duplicates=566 new=true
TimerTick                  ...  sccm.hazard_button_on=true  bcm.left_turn_request_on=false  bcm.right_turn_request_on=true
TimerTick                  ...  sccm.hazard_button_on=true  bcm.left_turn_request_on=true   bcm.right_turn_request_on=true
```

Left and right order can swap. The `completed_duplicates` counts are Restbus
repeats of OFF that were not ledgered. With the default `--rpm-clamp 1000`,
operational state stays `Idle`. There must be no `SetTurnLights` action.
Unfiltered `UpdateRpm` lines may later echo `hazard_button_on=true`; that is
context, not a new observation.

### 3h. Stop the bridge cleanly

In the bridge terminal:

```text
Ctrl+C
```

Expected: bridge writes `RPM=0`, then `PowerOff`, and exits 0. Gateway reaches
`Off`. Remotive containers stay up until section 5.

## 4. Optional TUI (incomplete at Stage 2)

Start TUI first, then Gateway with matching UDS. Bare socket names resolve
under `<repo>/tmp/`.

Terminal 1:

```bash
cd /home/nirmalya/Workspace-Rust/Eclipse-SDV/Self-handson-project/sdv_simulation_remotive
mkdir -p tmp
cargo run -p tui_dashboard -- --uds observation.sock
```

Terminal 2:

```bash
cd /home/nirmalya/Workspace-Rust/Eclipse-SDV/Self-handson-project/sdv_simulation_remotive
cargo run -p gateway -- --uds observation.sock --print-transitions-only
```

Then start the bridge as in 3f.

The TUI still shows the old driver/engineer panes. Do not treat it as Stage 2
acceptance. Use Gateway stdout.

## 5. Cleanup

Stop Twin processes first: Ctrl+C Gateway, bridge, TUI, and any `candump`.

Then stop only this run's Remotive compose project:

```bash
cd "$COMPOSE_DIR"
docker compose -p "$COMPOSE_PROJECT" \
  -f docker-compose.yml \
  -f "$COMPOSE_OVERRIDE" \
  --profile tester down --remove-orphans
```

Do not remove pre-existing `remotive_car_hello_world` networks.

Confirm leftovers:

```bash
pgrep -af 'gateway|remotive_bridge|tui_dashboard|candump' || true
docker ps --filter name=getting_started_local
docker network ls | grep -E 'getting_started_local|gs_local_ctl|hello_world' || true
```

## 6. What not to expect yet

- TUI hazard / left / right ON-OFF panes
- schema v5 event names in captured observation files
- Twin publication back to Remotive
- 3D RemotiveCar side-by-side demo
