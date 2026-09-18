use anyhow::Result;
use emulator::cli::{ProbabilityOverride, apply_probability_override, parse_args};
use emulator::models::PhysicalWorldModelConfig;
use emulator::runner::{SessionConfig, run_session};
use emulator::sink::SocketCanSink;
use emulator::source::LivePhysicsSource;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{env, thread};

/// Override for the per-tick probability of *entering* a tunnel (low lux → headlamp ON).
///
/// A "tick" is one publish loop (default 100 ms; see `--tick-ms`), so with the default period
/// probability `p` ≈ one tunnel every `1/(p·10)` seconds while not already in one. Must be a
/// float in `0.0..=1.0`; unset → the profile default `0.01` (≈ a tunnel every ~10 s at 100 ms
/// ticks — frequent, good for demos). For **infrequent** tunnels try
/// `EMULATOR_TUNNEL_PROB=0.002` or `0.001`. Wall-clock spacing scales with `--tick-ms`.
const ENV_TUNNEL_PROB: &str = "EMULATOR_TUNNEL_PROB";

/// Override for the per-tick probability of *entering* rain when dry.
///
/// Same units as [`ENV_TUNNEL_PROB`]: probability **per tick**, not per wall-clock second.
/// Default profile `0.008` ≈ rain every ~12 s at 100 ms ticks. Try `EMULATOR_RAIN_PROB=0.002`
/// or `0.0` to disable. Wall-clock spacing scales with `--tick-ms`.
const ENV_RAIN_PROB: &str = "EMULATOR_RAIN_PROB";

fn main() -> Result<()> {
    let args = parse_args(env::args().skip(1))?;
    let mut cfg = PhysicalWorldModelConfig::daytime_tunnel_profile();

    apply_env_probability_override(
        ENV_TUNNEL_PROB,
        &mut cfg.ambient_road_light.tunnel_event_probability_per_tick,
        "tunnel entry probability per tick",
    );
    apply_env_probability_override(
        ENV_RAIN_PROB,
        &mut cfg.rain.rain_event_probability_per_tick,
        "rain entry probability per tick",
    );

    if args.tick.as_millis() != u128::from(emulator::cli::DEFAULT_TICK_MS) {
        println!(
            "[emulator] --tick-ms={} (default {})",
            args.tick.as_millis(),
            emulator::cli::DEFAULT_TICK_MS
        );
    }

    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_handler = Arc::clone(&stop);
    ctrlc::set_handler(move || {
        stop_for_handler.store(true, Ordering::SeqCst);
    })?;

    let mut sink = SocketCanSink::open(vehicle_device_bus::DEFAULT_CAN_INTERFACE)?;
    let mut source = LivePhysicsSource::from_config(cfg);
    run_session(
        &mut sink,
        &mut source,
        SessionConfig {
            max_readings: args.readings,
            tick: args.tick,
        },
        thread::sleep,
        || stop.load(Ordering::SeqCst),
    )
}

fn apply_env_probability_override(name: &str, target: &mut f32, success_text: &str) {
    let raw = env::var(name).ok();
    match apply_probability_override(raw.as_deref(), target) {
        ProbabilityOverride::Applied(value) => {
            println!("[emulator] {name}={value} — {success_text}");
        }
        ProbabilityOverride::Missing => {}
        ProbabilityOverride::Invalid => {
            if let Some(raw) = raw {
                eprintln!("[emulator] ignoring {name}={raw:?} — expected a float in 0.0..=1.0");
            }
        }
    }
}
