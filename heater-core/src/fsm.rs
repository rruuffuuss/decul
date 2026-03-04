//! Internal finite state machine 
//!
//! This module contains deterministic transition logic only.
//!

use crate::{
    ActuatorIntent, Config, CoreState, EffectiveMode, FaultCode, HeatStage, ModeRequest,
    ProcessState, SensorInput, Status, TickEvents, TickInput, TickOutput,
};

fn is_finite_sensors(s: &SensorInput) -> bool {
    s.room_temp_c.is_finite() && s.hx_temp_c.is_finite() && s.supply_v.is_finite()
}

fn detect_safety_fault(input: &TickInput, cfg: &Config) -> Option<FaultCode> {
    if !is_finite_sensors(&input.sensors) {
        return Some(FaultCode::InvalidState);
    }
    if cfg.has_overheat_cutoff {
        match input.sensors.overheat_cutoff_tripped {
            Some(true) => return Some(FaultCode::OverTemp),
            Some(false) => {}
            None => return Some(FaultCode::SensorFault),
        }
    }
    if !input.sensors.sensor_ok {
        return Some(FaultCode::SensorFault);
    }
    if input.sensors.hx_temp_c >= cfg.max_hx_temp_c {
        return Some(FaultCode::OverTemp);
    }
    if input.sensors.supply_v < cfg.min_supply_v {
        return Some(FaultCode::UnderVoltage);
    }
    None
}

fn transition(state: &mut CoreState, events: &mut TickEvents, next: ProcessState) {
    if state.process_state != next {
        state.process_state = next;
        state.state_elapsed_ms = 0;
        events.entered_state = Some(next);
    }
}

fn latch_fault(state: &mut CoreState, events: &mut TickEvents, fault: FaultCode) {
    if state.latched_fault.is_none() {
        state.latched_fault = Some(fault);
        events.fault_latched = Some(fault);
    }
}

fn force_safe_shutdown_path(state: &mut CoreState, events: &mut TickEvents) {
    match state.process_state {
        ProcessState::Idle => transition(state, events, ProcessState::Lockout),
        ProcessState::Lockout | ProcessState::Shutdown | ProcessState::Cooldown => {}
        _ => transition(state, events, ProcessState::Shutdown),
    }
}

fn combustion_session_active(process_state: ProcessState) -> bool {
    matches!(
        process_state,
        ProcessState::Precheck
            | ProcessState::Preheat
            | ProcessState::IgnitionTrial
            | ProcessState::FlameStabilise
            | ProcessState::Run
            | ProcessState::Shutdown
            | ProcessState::Cooldown
    )
}

fn thermostat_demand(
    room_temp_c: f32,
    setpoint_c: f32,
    hysteresis_c: f32,
    currently_heating: bool,
) -> bool {
    if room_temp_c <= setpoint_c - hysteresis_c {
        true
    } else if room_temp_c >= setpoint_c + hysteresis_c {
        false
    } else {
        currently_heating
    }
}

fn stage_from_temp_difference(temp_difference_c: f32, cfg: &Config) -> HeatStage {
    if temp_difference_c >= cfg.stage_high_delta_c {
        HeatStage::High
    } else if temp_difference_c >= cfg.stage_medium_delta_c {
        HeatStage::Medium
    } else if temp_difference_c >= cfg.stage_low_delta_c {
        HeatStage::Low
    } else {
        HeatStage::Low
    }
}

///prefer flame signal otherwise use inferred value from heat exchanger temp change
fn resolve_flame_signal(input: &TickInput, cfg: &Config, inferred: bool) -> Result<bool, FaultCode> {
    if cfg.has_flame_sensor {
        input.sensors.flame_present.ok_or(FaultCode::SensorFault)
    } else {
        Ok(inferred)
    }
}

fn infer_ignition_flame(input: &TickInput, state: &CoreState, cfg: &Config) -> bool {
    let rise_c = input.sensors.hx_temp_c - state.ignition_baseline_temp_c;
    rise_c >= cfg.ignition_min_rise_c && input.sensors.hx_temp_c >= cfg.ignition_min_abs_c
}

fn update_low_temp_timer(state: &mut CoreState, input: &TickInput, cfg: &Config, delta_ms: u64) {
    if input.sensors.hx_temp_c < cfg.run_min_temp_c {
        state.low_temp_elapsed_ms = state.low_temp_elapsed_ms.saturating_add(delta_ms);
    } else {
        state.low_temp_elapsed_ms = 0;
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct DemandResult {
    heat_demand: bool,
    forced_high: bool,
    schedule_inhibited_no_time: bool,
}

fn resolve_mode_and_demand(input: &TickInput, state: &mut CoreState, cfg: &Config) -> DemandResult {
    if let Some(setpoint) = input.commands.manual_setpoint_c {
        if setpoint.is_finite() && (5.0..=35.0).contains(&setpoint) {
            state.manual_setpoint_c = setpoint;
        }
    }

    if let Some(request) = input.commands.mode_request {
        state.selected_mode = request;
        if request == ModeRequest::Off {
            state.boost_expires_at_ms = None;
        }
    }

    if input.commands.boost_cancel {
        state.boost_expires_at_ms = None;
    }

    if input.commands.boost_request && state.selected_mode != ModeRequest::Off {
        state.boost_expires_at_ms = Some(
            input
                .time
                .monotonic_ms
                .saturating_add(u64::from(cfg.boost_duration_ms)),
        );
    }

    let boost_active = state
        .boost_expires_at_ms
        .is_some_and(|expires| input.time.monotonic_ms < expires);
    if !boost_active {
        state.boost_expires_at_ms = None;
    }

    state.effective_mode = match state.selected_mode {
        ModeRequest::Off => EffectiveMode::Off,
        ModeRequest::Manual => EffectiveMode::Manual,
        ModeRequest::Scheduled if boost_active => EffectiveMode::Boost,
        ModeRequest::Scheduled => EffectiveMode::Scheduled,
    };

    let currently_heating = combustion_session_active(state.process_state);
    let mut schedule_inhibited_no_time = false;

    match state.effective_mode {
        EffectiveMode::Off => DemandResult {
            heat_demand: false,
            forced_high: false,
            schedule_inhibited_no_time: false,
        },
        EffectiveMode::Manual => DemandResult {
            heat_demand: thermostat_demand(
                input.sensors.room_temp_c,
                state.manual_setpoint_c,
                cfg.setpoint_hysteresis_c,
                currently_heating,
            ),
            forced_high: false,
            schedule_inhibited_no_time: false,
        },
        EffectiveMode::Boost => DemandResult {
            heat_demand: true,
            forced_high: true,
            schedule_inhibited_no_time: false,
        },
        EffectiveMode::Scheduled => {
            if !input.time.utc_valid {
                schedule_inhibited_no_time = true;
                DemandResult {
                    heat_demand: false,
                    forced_high: false,
                    schedule_inhibited_no_time,
                }
            } else if !input.schedule.enabled_now {
                DemandResult {
                    heat_demand: false,
                    forced_high: false,
                    schedule_inhibited_no_time,
                }
            } else if let Some(target_temp_c) = input.schedule.target_temp_c {
                DemandResult {
                    heat_demand: thermostat_demand(
                        input.sensors.room_temp_c,
                        target_temp_c,
                        cfg.setpoint_hysteresis_c,
                        currently_heating,
                    ),
                    forced_high: false,
                    schedule_inhibited_no_time,
                }
            } else {
                DemandResult {
                    heat_demand: false,
                    forced_high: false,
                    schedule_inhibited_no_time,
                }
            }
        }
    }
}

fn stage_for_run(
    input: &TickInput,
    state: &CoreState,
    cfg: &Config,
    forced_high: bool,
) -> HeatStage {
    if forced_high {
        return HeatStage::High;
    }

    let maybe_setpoint = match state.effective_mode {
        EffectiveMode::Manual => Some(state.manual_setpoint_c),
        EffectiveMode::Scheduled => input.schedule.target_temp_c,
        EffectiveMode::Boost => Some(100.0),
        EffectiveMode::Off => None,
    };

    let Some(setpoint_c) = maybe_setpoint else {
        return HeatStage::Off;
    };

    //
    let temp_difference_c = setpoint_c - input.sensors.room_temp_c;
    stage_from_temp_difference(temp_difference_c, cfg)
}

fn default_actuators_for_state(
    process_state: ProcessState,
    run_stage: HeatStage,
) -> ActuatorIntent {
    match process_state {
        ProcessState::Idle | ProcessState::Precheck | ProcessState::Lockout => ActuatorIntent {
            fan_pct: 0,
            glow_on: false,
            fuel_pump_hz: 0.0,
        },
        ProcessState::Preheat => ActuatorIntent {
            fan_pct: 25,
            glow_on: true,
            fuel_pump_hz: 0.0,
        },
        ProcessState::IgnitionTrial => ActuatorIntent {
            fan_pct: 35,
            glow_on: true,
            fuel_pump_hz: 1.6,
        },
        ProcessState::FlameStabilise => ActuatorIntent {
            fan_pct: 42,
            glow_on: true,
            fuel_pump_hz: 1.8,
        },
        ProcessState::Run => match run_stage {
            HeatStage::Off | HeatStage::Low => ActuatorIntent {
                fan_pct: 50,
                glow_on: false,
                fuel_pump_hz: 1.7,
            },
            HeatStage::Medium => ActuatorIntent {
                fan_pct: 68,
                glow_on: false,
                fuel_pump_hz: 2.6,
            },
            HeatStage::High => ActuatorIntent {
                fan_pct: 92,
                glow_on: false,
                fuel_pump_hz: 3.6,
            },
        },
        ProcessState::Shutdown => ActuatorIntent {
            fan_pct: 65,
            glow_on: false,
            fuel_pump_hz: 0.0,
        },
        ProcessState::Cooldown => ActuatorIntent {
            fan_pct: 80,
            glow_on: false,
            fuel_pump_hz: 0.0,
        },
    }
}

/// Single deterministic control step.
pub(crate) fn tick(input: &TickInput, state: &mut CoreState, cfg: &Config) -> TickOutput {
    let mut events = TickEvents::default();
    let sanitised = cfg.sanitise();
    let cfg = sanitised.cfg;

    let delta_ms = u64::from(input.time.delta_ms);
    state.state_elapsed_ms = state.state_elapsed_ms.saturating_add(delta_ms);

    if state.process_state == ProcessState::Run {
        state.run_elapsed_ms = state.run_elapsed_ms.saturating_add(delta_ms);
    }
    if state.process_state == ProcessState::Idle {
        state.off_elapsed_ms = state.off_elapsed_ms.saturating_add(delta_ms);
    }

    let demand = resolve_mode_and_demand(input, state, &cfg);
    let manual_off = state.selected_mode == ModeRequest::Off;

    if let Some(safety_fault) = detect_safety_fault(input, &cfg) {
        latch_fault(state, &mut events, safety_fault);
        force_safe_shutdown_path(state, &mut events);
    }

    match state.process_state {

        //idle
        //latched fault -> lockout
        //heat demand && off longer than minimum off time -> Precheck
        ProcessState::Idle => {
            if state.latched_fault.is_some() {
                transition(state, &mut events, ProcessState::Lockout);
            } else if demand.heat_demand && state.off_elapsed_ms >= u64::from(cfg.min_off_ms) {
                state.ignition_attempt = 0;
                state.run_elapsed_ms = 0;
                transition(state, &mut events, ProcessState::Precheck);
            }
        }

        //precheck
        //confirmation state before starting
        //latched fault || no heat demand -> shutdown
        //heat demand -> preheat
        ProcessState::Precheck => {
            if state.latched_fault.is_some() || !demand.heat_demand {
                transition(state, &mut events, ProcessState::Shutdown);
            } else {
                transition(state, &mut events, ProcessState::Preheat);
            }
        }

        //preheat
        //state for warming glow plug prior to fuel injection
        //glow actuator on
        //latched fault || no heat demand -> shutdown
        //pre heating has occured for long enough (glow plug is warm) -> ignitiontrial
        ProcessState::Preheat => {
            if state.latched_fault.is_some() || !demand.heat_demand {
                transition(state, &mut events, ProcessState::Shutdown);
            } else if state.state_elapsed_ms >= u64::from(cfg.preheat_ms) {
                state.ignition_attempt = state.ignition_attempt.saturating_add(1).min(2);
                state.ignition_baseline_temp_c = input.sensors.hx_temp_c;
                state.low_temp_elapsed_ms = 0;
                transition(state, &mut events, ProcessState::IgnitionTrial);
            }
        }

        //ignitiontrial
        //state for attempting & confirming ignition
        //latched fault || no heat demand -> shutdown
        //ignition detected within ignition window-> flamestabilise
        //first failed ignition attempt -> shutdown and restart
        //second failed ignition attept -> shutdown with error
        ProcessState::IgnitionTrial => {
            if state.latched_fault.is_some() || !demand.heat_demand {
                transition(state, &mut events, ProcessState::Shutdown);
            } else {
                let inferred = infer_ignition_flame(input, state, &cfg);
                match resolve_flame_signal(input, &cfg, inferred) {
                    Ok(true) => {
                        state.low_temp_elapsed_ms = 0;
                        transition(state, &mut events, ProcessState::FlameStabilise);
                    }
                    Ok(false) => {
                        if state.state_elapsed_ms >= u64::from(cfg.ignition_window_ms) {
                            if state.ignition_attempt < 2 {
                                state.pending_restart_after_cooldown = true;
                                transition(state, &mut events, ProcessState::Shutdown);
                            } else {
                                latch_fault(state, &mut events, FaultCode::IgnitionFailed);
                                transition(state, &mut events, ProcessState::Shutdown);
                            }
                        }
                    }
                    Err(fault) => {
                        latch_fault(state, &mut events, fault);
                        transition(state, &mut events, ProcessState::Shutdown);
                    }
                }
            }
        }

        //FlameStabilise
        //temporary state to set actuator values for warmup/flame stabilisation
        ProcessState::FlameStabilise => {
            if state.latched_fault.is_some() || !demand.heat_demand {
                transition(state, &mut events, ProcessState::Shutdown);
            } else {
                if !cfg.has_flame_sensor {
                    update_low_temp_timer(state, input, &cfg, delta_ms);
                } else {
                    state.low_temp_elapsed_ms = 0;
                }

                let inferred_flame = state.low_temp_elapsed_ms < u64::from(cfg.flame_loss_ms);
                match resolve_flame_signal(input, &cfg, inferred_flame) {
                    Ok(true) => {
                        if state.state_elapsed_ms >= u64::from(cfg.flame_stabilise_ms) {
                            state.run_elapsed_ms = 0;
                            state.low_temp_elapsed_ms = 0;
                            transition(state, &mut events, ProcessState::Run);
                        }
                    }
                    Ok(false) => {
                        latch_fault(state, &mut events, FaultCode::FlameLost);
                        transition(state, &mut events, ProcessState::Shutdown);
                    }
                    Err(fault) => {
                        latch_fault(state, &mut events, fault);
                        transition(state, &mut events, ProcessState::Shutdown);
                    }
                }
            }
        }

        //Run
        //main state for heating
        //actuators vary depending on mode/setpoint
        //latched fault || turned off manually -> shutdown
        //flame sensor doesn't detect flame -> FlameLost latch fault then shutdown
        //no heat demand after minimum runtime -> shutdown
        ProcessState::Run => {
            if state.latched_fault.is_some() {
                transition(state, &mut events, ProcessState::Shutdown);
            } else {
                if !cfg.has_flame_sensor {
                    update_low_temp_timer(state, input, &cfg, delta_ms);
                } else {
                    state.low_temp_elapsed_ms = 0;
                }

                let inferred_flame = state.low_temp_elapsed_ms < u64::from(cfg.flame_loss_ms);
                match resolve_flame_signal(input, &cfg, inferred_flame) {
                    Ok(false) => {
                        latch_fault(state, &mut events, FaultCode::FlameLost);
                        transition(state, &mut events, ProcessState::Shutdown);
                    }
                    Err(fault) => {
                        latch_fault(state, &mut events, fault);
                        transition(state, &mut events, ProcessState::Shutdown);
                    }
                    Ok(true) if manual_off => {
                        transition(state, &mut events, ProcessState::Shutdown);
                    }
                    Ok(true) if !demand.heat_demand && state.run_elapsed_ms >= u64::from(cfg.min_run_ms) => {
                        transition(state, &mut events, ProcessState::Shutdown);
                    }
                    Ok(true) => {}
                }
            }
        }

        //shutdown
        //confirmation state before cooldown
        ProcessState::Shutdown => {
            transition(state, &mut events, ProcessState::Cooldown);
        }

        //shutdown
        //keeps fan running with no fuel injection (this is critical to prevent damage in a webasto style heater)
        ProcessState::Cooldown => {
            let cooldown_target = if state.pending_restart_after_cooldown {
                cfg.retry_purge_ms
            } else {
                cfg.cooldown_ms
            };

            if state.state_elapsed_ms >= u64::from(cooldown_target) {
                if state.pending_restart_after_cooldown {
                    state.pending_restart_after_cooldown = false;
                    events.ignition_retry_started = true;
                    transition(state, &mut events, ProcessState::Preheat);
                } else if state.latched_fault.is_some() {
                    transition(state, &mut events, ProcessState::Lockout);
                } else {
                    state.ignition_attempt = 0;
                    state.run_elapsed_ms = 0;
                    state.last_heat_stage = HeatStage::Off;
                    state.off_elapsed_ms = 0;
                    transition(state, &mut events, ProcessState::Idle);
                }
            }
        }

        //Lockout
        //safety fault state
        //requires manual reset with no faults to return to idle
        ProcessState::Lockout => {
            if input.commands.fault_reset_request && detect_safety_fault(input, &cfg).is_none() {
                state.latched_fault = None;
                state.ignition_attempt = 0;
                state.pending_restart_after_cooldown = false;
                state.last_heat_stage = HeatStage::Off;
                state.off_elapsed_ms = 0;
                events.fault_cleared = true;
                transition(state, &mut events, ProcessState::Idle);
            }
        }
    }

    if state.process_state == ProcessState::Run {
        state.last_heat_stage = stage_for_run(input, state, &cfg, demand.forced_high);
    } else {
        state.last_heat_stage = HeatStage::Off;
    }

    let mut actuators = default_actuators_for_state(state.process_state, state.last_heat_stage);

    // Hard safety invariants on actuator intent.
    match state.process_state {
        ProcessState::Idle
        | ProcessState::Precheck
        | ProcessState::Preheat
        | ProcessState::Shutdown
        | ProcessState::Cooldown
        | ProcessState::Lockout => {
            actuators.fuel_pump_hz = 0.0;
        }
        ProcessState::IgnitionTrial | ProcessState::FlameStabilise | ProcessState::Run => {}
    }
    if state.process_state == ProcessState::Lockout {
        actuators.glow_on = false;
        actuators.fan_pct = 0;
    }

    TickOutput {
        actuators,
        status: Status {
            process_state: state.process_state,
            effective_mode: state.effective_mode,
            active_stage: state.last_heat_stage,
            fault: state.latched_fault,
            heat_demand: demand.heat_demand,
            schedule_inhibited_no_time: demand.schedule_inhibited_no_time,
            config_defaulted: sanitised.used_defaults,
        },
        events,
    }
}
