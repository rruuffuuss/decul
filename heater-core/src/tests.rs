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
        has_flame_sensor: true,
        has_overheat_cutoff: true,
        ignition_min_rise_c: 8.0,
        ignition_min_abs_c: 45.0,
        run_min_temp_c: 40.0,
        flame_loss_ms: 200,
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
            flame_present: Some(false),
            overheat_cutoff_tripped: Some(false),
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
    i.sensors.flame_present = Some(true);
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
    i.sensors.flame_present = Some(true);
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
    i.sensors.flame_present = Some(false);
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
fn safety_fault_undervoltage_in_active_path_enters_cooldown_then_lockout() {
    let mut cfg = test_cfg();
    cfg.cooldown_ms = 100;
    let mut engine = HeaterEngine::new(cfg);

    let mut i = input(0, 100, Some(ModeRequest::Manual), false, None);
    let out0 = engine.step(&i);
    assert_eq!(out0.status.process_state, ProcessState::Precheck);

    i.time.monotonic_ms = 100;
    i.commands.mode_request = None;
    i.sensors.supply_v = 9.0;
    let out1 = engine.step(&i);
    assert_eq!(out1.status.fault, Some(FaultCode::UnderVoltage));
    assert_eq!(out1.status.process_state, ProcessState::Cooldown);
    assert_eq!(out1.actuators.fuel_pump_hz, 0.0);

    i.time.monotonic_ms = 200;
    let out2 = engine.step(&i);
    assert_eq!(out2.status.process_state, ProcessState::Lockout);
    assert_eq!(out2.status.fault, Some(FaultCode::UnderVoltage));
    assert_eq!(out2.actuators.fuel_pump_hz, 0.0);
    assert!(!out2.actuators.glow_on);
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
    i.sensors.flame_present = Some(true);
    let out_hold = engine.step(&i);
    assert_eq!(out_hold.status.process_state, ProcessState::Run);

    i.time.monotonic_ms = 2_300;
    i.time.delta_ms = 300;
    i.sensors.flame_present = Some(true);
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
            i.sensors.flame_present = Some(true);
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
        has_flame_sensor: true,
        has_overheat_cutoff: true,
        ignition_min_rise_c: 0.0,
        ignition_min_abs_c: 500.0,
        run_min_temp_c: 1_000.0,
        flame_loss_ms: 0,
    };

    let mut engine = HeaterEngine::new(cfg);
    let i = input(0, 100, Some(ModeRequest::Manual), false, None);
    let out = engine.step(&i);

    assert!(out.status.config_defaulted);
}

#[test]
fn configured_flame_sensor_requires_signal() {
    let cfg = test_cfg();
    let state = CoreState {
        process_state: ProcessState::Run,
        selected_mode: ModeRequest::Manual,
        effective_mode: EffectiveMode::Manual,
        manual_setpoint_c: 21.0,
        ..CoreState::default()
    };
    let mut engine = HeaterEngine::with_state(cfg, state);

    let mut i = input(0, 100, None, false, None);
    i.sensors.flame_present = None;

    let out = engine.step(&i);
    assert_eq!(out.status.fault, Some(FaultCode::SensorFault));
    assert_eq!(out.status.process_state, ProcessState::Shutdown);
}

#[test]
fn configured_cutoff_requires_signal() {
    let mut engine = HeaterEngine::new(test_cfg());

    let mut i = input(0, 100, Some(ModeRequest::Manual), false, None);
    i.sensors.overheat_cutoff_tripped = None;

    let out = engine.step(&i);
    assert_eq!(out.status.fault, Some(FaultCode::SensorFault));
    assert_eq!(out.status.process_state, ProcessState::Lockout);
}

#[test]
fn configured_cutoff_trip_latches_overtemp_immediately() {
    let mut engine = HeaterEngine::new(test_cfg());

    let mut i = input(0, 100, Some(ModeRequest::Manual), false, None);
    i.sensors.overheat_cutoff_tripped = Some(true);

    let out = engine.step(&i);
    assert_eq!(out.status.fault, Some(FaultCode::OverTemp));
    assert_eq!(out.status.process_state, ProcessState::Lockout);
    assert_eq!(out.actuators.fuel_pump_hz, 0.0);
    assert!(!out.actuators.glow_on);
}

#[test]
fn inferred_ignition_without_flame_sensor_reaches_run() {
    let mut cfg = test_cfg();
    cfg.has_flame_sensor = false;
    cfg.has_overheat_cutoff = false;
    cfg.ignition_min_rise_c = 2.0;
    cfg.ignition_min_abs_c = 25.0;
    cfg.run_min_temp_c = 20.0;
    cfg.flame_loss_ms = 300;
    let mut engine = HeaterEngine::new(cfg);

    let mut t = 0;
    let mut i = input(t, 100, Some(ModeRequest::Manual), false, None);
    i.sensors.hx_temp_c = 20.0;
    i.sensors.flame_present = None;
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
    i.sensors.hx_temp_c = 26.0;
    let out3 = engine.step(&i);
    assert_eq!(out3.status.process_state, ProcessState::FlameStabilise);

    t += 100;
    i.time.monotonic_ms = t;
    let out4 = engine.step(&i);
    assert_eq!(out4.status.process_state, ProcessState::Run);
}

#[test]
fn inferred_flame_loss_without_flame_sensor_latches_fault() {
    let mut cfg = test_cfg();
    cfg.has_flame_sensor = false;
    cfg.has_overheat_cutoff = false;
    cfg.run_min_temp_c = 40.0;
    cfg.flame_loss_ms = 200;

    let state = CoreState {
        process_state: ProcessState::Run,
        selected_mode: ModeRequest::Manual,
        effective_mode: EffectiveMode::Manual,
        manual_setpoint_c: 21.0,
        ..CoreState::default()
    };
    let mut engine = HeaterEngine::with_state(cfg, state);

    let mut i = input(0, 100, None, false, None);
    i.sensors.room_temp_c = 10.0;
    i.sensors.hx_temp_c = 30.0;
    i.sensors.flame_present = None;

    let out_hold = engine.step(&i);
    assert_eq!(out_hold.status.process_state, ProcessState::Run);
    assert_eq!(out_hold.status.fault, None);

    i.time.monotonic_ms = 100;
    let out_loss = engine.step(&i);
    assert_eq!(out_loss.status.process_state, ProcessState::Shutdown);
    assert_eq!(out_loss.status.fault, Some(FaultCode::FlameLost));
}

#[test]
fn flame_loss_timer_resets_when_hx_recovers() {
    let mut cfg = test_cfg();
    cfg.has_flame_sensor = false;
    cfg.has_overheat_cutoff = false;
    cfg.run_min_temp_c = 40.0;
    cfg.flame_loss_ms = 200;

    let state = CoreState {
        process_state: ProcessState::Run,
        selected_mode: ModeRequest::Manual,
        effective_mode: EffectiveMode::Manual,
        manual_setpoint_c: 21.0,
        ..CoreState::default()
    };
    let mut engine = HeaterEngine::with_state(cfg, state);

    let mut i = input(0, 100, None, false, None);
    i.sensors.room_temp_c = 10.0;
    i.sensors.flame_present = None;

    i.sensors.hx_temp_c = 30.0;
    let out_low_1 = engine.step(&i);
    assert_eq!(out_low_1.status.process_state, ProcessState::Run);
    assert_eq!(out_low_1.status.fault, None);

    i.time.monotonic_ms = 100;
    i.sensors.hx_temp_c = 45.0;
    let out_recover = engine.step(&i);
    assert_eq!(out_recover.status.process_state, ProcessState::Run);
    assert_eq!(out_recover.status.fault, None);

    i.time.monotonic_ms = 200;
    i.sensors.hx_temp_c = 30.0;
    let out_low_2 = engine.step(&i);
    assert_eq!(out_low_2.status.process_state, ProcessState::Run);
    assert_eq!(out_low_2.status.fault, None);

    i.time.monotonic_ms = 300;
    i.sensors.hx_temp_c = 30.0;
    let out_low_3 = engine.step(&i);
    assert_eq!(out_low_3.status.process_state, ProcessState::Shutdown);
    assert_eq!(out_low_3.status.fault, Some(FaultCode::FlameLost));
}

#[test]
fn ignition_baseline_captured_and_used_for_inference() {
    let mut cfg = test_cfg();
    cfg.has_flame_sensor = false;
    cfg.has_overheat_cutoff = false;
    cfg.ignition_window_ms = 500;
    cfg.ignition_min_rise_c = 4.0;
    cfg.ignition_min_abs_c = 45.0;

    let mut engine = HeaterEngine::new(cfg);
    let mut t = 0;
    let mut i = input(t, 100, Some(ModeRequest::Manual), false, None);
    i.sensors.flame_present = None;
    i.sensors.hx_temp_c = 50.0;

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
    assert_eq!(engine.state().ignition_baseline_temp_c, 50.0);

    t += 100;
    i.time.monotonic_ms = t;
    i.sensors.hx_temp_c = 53.0;
    let out3 = engine.step(&i);
    assert_eq!(out3.status.process_state, ProcessState::IgnitionTrial);

    t += 100;
    i.time.monotonic_ms = t;
    i.sensors.hx_temp_c = 54.0;
    let out4 = engine.step(&i);
    assert_eq!(out4.status.process_state, ProcessState::FlameStabilise);
}

#[test]
fn cutoff_trip_precedes_other_safety_faults() {
    let mut engine = HeaterEngine::new(test_cfg());
    let mut i = input(0, 100, Some(ModeRequest::Manual), false, None);
    i.sensors.overheat_cutoff_tripped = Some(true);
    i.sensors.sensor_ok = false;
    i.sensors.hx_temp_c = 300.0;
    i.sensors.supply_v = 2.0;

    let out = engine.step(&i);
    assert_eq!(out.status.fault, Some(FaultCode::OverTemp));
    assert_eq!(out.events.fault_latched, Some(FaultCode::OverTemp));
    assert_eq!(out.status.process_state, ProcessState::Lockout);
}

#[test]
fn manual_off_bypasses_min_run_but_uses_shutdown_and_cooldown() {
    let mut cfg = test_cfg();
    cfg.min_run_ms = 120_000;

    let state = CoreState {
        process_state: ProcessState::Run,
        selected_mode: ModeRequest::Manual,
        effective_mode: EffectiveMode::Manual,
        manual_setpoint_c: 21.0,
        run_elapsed_ms: 0,
        ..CoreState::default()
    };
    let mut engine = HeaterEngine::with_state(cfg, state);

    let mut i = input(0, 100, Some(ModeRequest::Off), false, None);
    i.sensors.flame_present = Some(true);
    i.sensors.room_temp_c = 10.0;
    let out_shutdown = engine.step(&i);
    assert_eq!(out_shutdown.status.process_state, ProcessState::Shutdown);
    assert_eq!(out_shutdown.actuators.fuel_pump_hz, 0.0);

    i.time.monotonic_ms = 100;
    i.commands.mode_request = None;
    let out_cooldown = engine.step(&i);
    assert_eq!(out_cooldown.status.process_state, ProcessState::Cooldown);
    assert_eq!(out_cooldown.actuators.fuel_pump_hz, 0.0);
}

#[test]
fn fault_reset_denied_while_cutoff_still_tripped() {
    let mut engine = HeaterEngine::new(test_cfg());
    let mut i = input(0, 100, Some(ModeRequest::Manual), false, None);
    i.sensors.overheat_cutoff_tripped = Some(true);

    let out_fault = engine.step(&i);
    assert_eq!(out_fault.status.process_state, ProcessState::Lockout);
    assert_eq!(out_fault.status.fault, Some(FaultCode::OverTemp));

    i.time.monotonic_ms = 100;
    i.commands.mode_request = None;
    i.commands.fault_reset_request = true;
    i.sensors.overheat_cutoff_tripped = Some(true);
    let out_reject = engine.step(&i);
    assert_eq!(out_reject.status.process_state, ProcessState::Lockout);
    assert!(!out_reject.events.fault_cleared);

    i.time.monotonic_ms = 200;
    i.sensors.overheat_cutoff_tripped = Some(false);
    let out_accept = engine.step(&i);
    assert_eq!(out_accept.status.process_state, ProcessState::Idle);
    assert!(out_accept.events.fault_cleared);
}

#[test]
fn deterministic_with_zero_and_large_deltas() {
    let cfg = test_cfg();
    let mut a = HeaterEngine::new(cfg);
    let mut b = HeaterEngine::new(cfg);

    let deltas_ms = [0_u32, 0, 100, 10_000, 50_000, 100, 0];
    let mut monotonic_ms = 0_u64;
    let mut stream = Vec::new();

    for (idx, delta_ms) in deltas_ms.into_iter().enumerate() {
        let mut i = input(
            monotonic_ms,
            delta_ms,
            if idx == 0 {
                Some(ModeRequest::Manual)
            } else {
                None
            },
            false,
            None,
        );
        i.sensors.room_temp_c = 10.0;
        i.sensors.hx_temp_c = 60.0;
        i.sensors.flame_present = Some(true);
        stream.push(i);
        monotonic_ms = monotonic_ms.saturating_add(u64::from(delta_ms));
    }

    let out_a: Vec<_> = stream.iter().map(|i| a.step(i)).collect();
    let out_b: Vec<_> = stream.iter().map(|i| b.step(i)).collect();
    assert_eq!(out_a, out_b);
    assert_eq!(a.state(), b.state());

    for out in out_a {
        if matches!(
            out.status.process_state,
            ProcessState::Idle
                | ProcessState::Precheck
                | ProcessState::Preheat
                | ProcessState::Shutdown
                | ProcessState::Cooldown
                | ProcessState::Lockout
        ) {
            assert_eq!(out.actuators.fuel_pump_hz, 0.0);
        }
        if out.status.process_state == ProcessState::Lockout {
            assert!(!out.actuators.glow_on);
            assert_eq!(out.actuators.fan_pct, 0);
        }
    }
}

#[test]
fn events_are_edge_triggered_not_level_triggered() {
    let mut cfg = test_cfg();
    cfg.preheat_ms = 300;
    let mut engine = HeaterEngine::new(cfg);

    let mut i = input(0, 100, Some(ModeRequest::Manual), false, None);
    let out0 = engine.step(&i);
    assert_eq!(out0.events.entered_state, Some(ProcessState::Precheck));

    i.time.monotonic_ms = 100;
    let out1 = engine.step(&i);
    assert_eq!(out1.events.entered_state, Some(ProcessState::Preheat));

    i.time.monotonic_ms = 200;
    let out2 = engine.step(&i);
    assert_eq!(out2.status.process_state, ProcessState::Preheat);
    assert_eq!(out2.events.entered_state, None);

    let state = CoreState {
        process_state: ProcessState::Run,
        selected_mode: ModeRequest::Manual,
        effective_mode: EffectiveMode::Manual,
        manual_setpoint_c: 21.0,
        ..CoreState::default()
    };
    let mut engine_fault = HeaterEngine::with_state(test_cfg(), state);
    let mut j = input(0, 100, None, false, None);
    j.sensors.flame_present = Some(false);

    let out_fault_1 = engine_fault.step(&j);
    assert_eq!(out_fault_1.events.fault_latched, Some(FaultCode::FlameLost));

    j.time.monotonic_ms = 100;
    let out_fault_2 = engine_fault.step(&j);
    assert_eq!(out_fault_2.events.fault_latched, None);

    let mut cfg_retry = test_cfg();
    cfg_retry.preheat_ms = 100;
    cfg_retry.ignition_window_ms = 100;
    cfg_retry.retry_purge_ms = 100;
    cfg_retry.cooldown_ms = 100;
    let mut engine_retry = HeaterEngine::new(cfg_retry);
    let mut k = input(0, 100, Some(ModeRequest::Manual), false, None);
    k.sensors.flame_present = Some(false);

    engine_retry.step(&k);
    k.time.monotonic_ms = 100;
    engine_retry.step(&k);
    k.time.monotonic_ms = 200;
    engine_retry.step(&k);
    k.time.monotonic_ms = 300;
    engine_retry.step(&k);
    k.time.monotonic_ms = 400;
    engine_retry.step(&k);
    k.time.monotonic_ms = 500;
    let out_retry = engine_retry.step(&k);
    assert!(out_retry.events.ignition_retry_started);

    k.time.monotonic_ms = 600;
    let out_after_retry = engine_retry.step(&k);
    assert!(!out_after_retry.events.ignition_retry_started);
}
