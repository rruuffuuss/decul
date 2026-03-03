use crate::*;

fn test_cfg() -> Config {
    Config {
        preheat_ms: 100,
        ignition_window_ms: 100,
        flame_stabilise_ms: 100,
        retry_purge_ms: 100,
        cooldown_ms: 100,
        max_hx_temp_c: 230.0,
        min_supply_v: 10.0,
        min_run_ms: 200,
        min_off_ms: 200,
        setpoint_hysteresis_c: 0.5,
        stage_low_delta_c: 0.2,
        stage_medium_delta_c: 1.0,
        stage_high_delta_c: 2.0,
        boost_duration_ms: 600,
    }
}

fn input(
    monotonic_ms: u64,
    delta_ms: u32,
    mode_request: Option<ModeRequest>,
    schedule_enabled: bool,
    schedule_target: Option<f32>,
) -> TickInput {
    TickInput {
        time: TimeInput {
            utc: Some(0),
            utc_valid: true,
            monotonic_ms,
            delta_ms,
        },
        sensors: SensorInput {
            room_temp_c: 15.0,
            hx_temp_c: 30.0,
            flame_present: false,
            supply_v: 12.0,
            sensor_ok: true,
        },
        commands: CommandInput {
            mode_request,
            manual_setpoint_c: Some(21.0),
            boost_request: false,
            boost_cancel: false,
            fault_reset_request: false,
        },
        schedule: ScheduleEvalInput {
            enabled_now: schedule_enabled,
            target_temp_c: schedule_target,
        },
    }
}

fn run_to_run_state(engine: &mut HeaterEngine) {
    let mut t = 0;
    let mut i = input(t, 100, Some(ModeRequest::Manual), false, None);
    engine.step(&i);

    t += 100;
    i.time.monotonic_ms = t;
    engine.step(&i);

    t += 100;
    i.time.monotonic_ms = t;
    engine.step(&i);

    t += 100;
    i.time.monotonic_ms = t;
    i.sensors.flame_present = true;
    engine.step(&i);

    t += 100;
    i.time.monotonic_ms = t;
    engine.step(&i);

    assert_eq!(engine.state().process_state, ProcessState::Run);
}

#[test]
fn progression_reaches_run() {
    let mut engine = HeaterEngine::new(test_cfg());
    let mut t = 0;

    let mut i = input(t, 100, Some(ModeRequest::Manual), false, None);
    let out0 = engine.step(&i);
    assert_eq!(out0.status.process_state, ProcessState::Precheck);

    t += 100;
    i.time.monotonic_ms = t;
    let out1 = engine.step(&i);
    assert_eq!(out1.status.process_state, ProcessState::Preheat);

    t += 100;
    i.time.monotonic_ms = t;
    let out2 = engine.step(&i);
    assert_eq!(out2.status.process_state, ProcessState::IgnitionTrial);

    t += 100;
    i.time.monotonic_ms = t;
    i.sensors.flame_present = true;
    let out3 = engine.step(&i);
    assert_eq!(out3.status.process_state, ProcessState::FlameStabilise);

    t += 100;
    i.time.monotonic_ms = t;
    let out4 = engine.step(&i);
    assert_eq!(out4.status.process_state, ProcessState::Run);
    assert!(out4.actuators.fuel_pump_hz > 0.0);
}

#[test]
fn ignition_two_tries_then_lockout() {
    let mut engine = HeaterEngine::new(test_cfg());

    let mut t = 0;
    let mut i = input(t, 100, Some(ModeRequest::Manual), false, None);

    engine.step(&i);
    t += 100;
    i.time.monotonic_ms = t;
    engine.step(&i);
    t += 100;
    i.time.monotonic_ms = t;
    engine.step(&i);
    t += 100;
    i.time.monotonic_ms = t;
    let out_fail_1 = engine.step(&i);
    assert_eq!(out_fail_1.status.process_state, ProcessState::Shutdown);

    t += 100;
    i.time.monotonic_ms = t;
    engine.step(&i);
    t += 100;
    i.time.monotonic_ms = t;
    let out_retry = engine.step(&i);
    assert!(out_retry.events.ignition_retry_started);
    assert_eq!(out_retry.status.process_state, ProcessState::Preheat);

    t += 100;
    i.time.monotonic_ms = t;
    engine.step(&i);
    t += 100;
    i.time.monotonic_ms = t;
    let out_fail_2 = engine.step(&i);
    assert_eq!(out_fail_2.status.fault, Some(FaultCode::IgnitionFailed));

    t += 100;
    i.time.monotonic_ms = t;
    engine.step(&i);
    t += 100;
    i.time.monotonic_ms = t;
    let out_lockout = engine.step(&i);
    assert_eq!(out_lockout.status.process_state, ProcessState::Lockout);
    assert_eq!(out_lockout.actuators.fuel_pump_hz, 0.0);
    assert!(!out_lockout.actuators.glow_on);
}

#[test]
fn flame_loss_in_run_latches_fault() {
    let mut engine = HeaterEngine::new(test_cfg());
    run_to_run_state(&mut engine);

    let mut i = input(1_000, 100, None, false, None);
    i.sensors.flame_present = false;
    let out = engine.step(&i);

    assert_eq!(out.status.process_state, ProcessState::Shutdown);
    assert_eq!(out.status.fault, Some(FaultCode::FlameLost));
}

#[test]
fn safety_fault_overtemp_forces_safe_outputs() {
    let mut engine = HeaterEngine::new(test_cfg());
    let mut i = input(0, 100, Some(ModeRequest::Manual), false, None);
    i.sensors.hx_temp_c = 300.0;

    let out = engine.step(&i);
    assert_eq!(out.status.fault, Some(FaultCode::OverTemp));
    assert_eq!(out.status.process_state, ProcessState::Lockout);
    assert_eq!(out.actuators.fuel_pump_hz, 0.0);
    assert!(!out.actuators.glow_on);
}

#[test]
fn schedule_inhibited_without_valid_time_but_manual_works() {
    let mut engine = HeaterEngine::new(test_cfg());

    let mut i = input(0, 100, Some(ModeRequest::Scheduled), true, Some(22.0));
    i.time.utc_valid = false;
    let out_schedule = engine.step(&i);
    assert!(out_schedule.status.schedule_inhibited_no_time);
    assert_eq!(out_schedule.status.process_state, ProcessState::Idle);

    i.commands.mode_request = Some(ModeRequest::Manual);
    i.time.utc_valid = false;
    let out_manual = engine.step(&i);
    assert_eq!(out_manual.status.process_state, ProcessState::Precheck);
}

#[test]
fn mode_priority_manual_off_overrides_boost_and_schedule() {
    let mut engine = HeaterEngine::new(test_cfg());

    let mut i = input(0, 100, Some(ModeRequest::Scheduled), true, Some(22.0));
    i.commands.boost_request = true;
    engine.step(&i);
    assert_eq!(engine.state().effective_mode, EffectiveMode::Boost);

    i.time.monotonic_ms = 100;
    i.commands.mode_request = Some(ModeRequest::Off);
    i.commands.boost_request = false;
    let out = engine.step(&i);
    assert_eq!(out.status.effective_mode, EffectiveMode::Off);
    assert!(!out.status.heat_demand);
}

#[test]
fn boost_overrides_schedule() {
    let mut engine = HeaterEngine::new(test_cfg());

    let mut i = input(0, 100, Some(ModeRequest::Scheduled), true, Some(18.0));
    i.sensors.room_temp_c = 17.8;
    engine.step(&i);
    assert_eq!(engine.state().effective_mode, EffectiveMode::Scheduled);

    i.time.monotonic_ms = 100;
    i.commands.mode_request = None;
    i.commands.boost_request = true;
    let out = engine.step(&i);
    assert_eq!(out.status.effective_mode, EffectiveMode::Boost);
    assert!(out.status.heat_demand);
}

#[test]
fn anti_short_cycle_min_run_and_min_off() {
    let mut engine = HeaterEngine::new(test_cfg());
    run_to_run_state(&mut engine);

    let mut i = input(2_000, 100, None, false, None);
    i.sensors.room_temp_c = 30.0;
    i.sensors.flame_present = true;
    let out_hold = engine.step(&i);
    assert_eq!(out_hold.status.process_state, ProcessState::Run);

    i.time.monotonic_ms = 2_300;
    i.time.delta_ms = 300;
    i.sensors.flame_present = true;
    let out_stop = engine.step(&i);
    assert_eq!(out_stop.status.process_state, ProcessState::Shutdown);

    i.time.monotonic_ms = 2_400;
    i.time.delta_ms = 100;
    engine.step(&i);
    i.time.monotonic_ms = 2_500;
    engine.step(&i);
    assert_eq!(engine.state().process_state, ProcessState::Idle);

    i.time.monotonic_ms = 2_600;
    i.time.delta_ms = 100;
    i.commands.mode_request = Some(ModeRequest::Manual);
    i.sensors.room_temp_c = 10.0;
    let out_blocked = engine.step(&i);
    assert_eq!(out_blocked.status.process_state, ProcessState::Idle);

    i.time.monotonic_ms = 2_900;
    i.time.delta_ms = 300;
    let out_start = engine.step(&i);
    assert_eq!(out_start.status.process_state, ProcessState::Precheck);
}

#[test]
fn fault_reset_requires_clear_conditions() {
    let mut engine = HeaterEngine::new(test_cfg());

    let mut i = input(0, 100, Some(ModeRequest::Manual), false, None);
    i.sensors.hx_temp_c = 400.0;
    engine.step(&i);
    assert_eq!(engine.state().process_state, ProcessState::Lockout);

    i.time.monotonic_ms = 100;
    i.commands.fault_reset_request = true;
    let out_reject = engine.step(&i);
    assert_eq!(out_reject.status.process_state, ProcessState::Lockout);

    i.time.monotonic_ms = 200;
    i.sensors.hx_temp_c = 40.0;
    let out_accept = engine.step(&i);
    assert!(out_accept.events.fault_cleared);
    assert_eq!(out_accept.status.process_state, ProcessState::Idle);
}

#[test]
fn deterministic_for_identical_input_stream() {
    let cfg = test_cfg();
    let mut a = HeaterEngine::new(cfg);
    let mut b = HeaterEngine::new(cfg);

    let mut stream = Vec::new();
    let mut t = 0;
    for step in 0..12 {
        let mut i = input(
            t,
            100,
            if step == 0 {
                Some(ModeRequest::Manual)
            } else {
                None
            },
            false,
            None,
        );
        if step >= 3 {
            i.sensors.flame_present = true;
        }
        if step >= 8 {
            i.sensors.room_temp_c = 30.0;
        }
        stream.push(i);
        t += 100;
    }

    let out_a: Vec<_> = stream.iter().map(|i| a.step(i)).collect();
    let out_b: Vec<_> = stream.iter().map(|i| b.step(i)).collect();

    assert_eq!(out_a, out_b);
    assert_eq!(a.state(), b.state());
}

#[test]
fn invalid_config_falls_back_to_defaults_flagged() {
    let cfg = Config {
        preheat_ms: 0,
        ignition_window_ms: 0,
        flame_stabilise_ms: 0,
        retry_purge_ms: 999_999,
        cooldown_ms: 1,
        max_hx_temp_c: f32::NAN,
        min_supply_v: -3.0,
        min_run_ms: 0,
        min_off_ms: 0,
        setpoint_hysteresis_c: 0.0,
        stage_low_delta_c: 3.0,
        stage_medium_delta_c: 1.0,
        stage_high_delta_c: 0.5,
        boost_duration_ms: 0,
    };

    let mut engine = HeaterEngine::new(cfg);
    let i = input(0, 100, Some(ModeRequest::Manual), false, None);
    let out = engine.step(&i);

    assert!(out.status.config_defaulted);
}
