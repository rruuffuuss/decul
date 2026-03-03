#![forbid(unsafe_code)]

pub type UtcTs = i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Idle,
    Precheck,
    Preheat,
    IgnitionTrial,
    FlameStabilize,
    Run,
    Shutdown,
    Cooldown,
    Lockout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectiveMode {
    Off,
    Manual,
    Scheduled,
    Boost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeRequest {
    Off,
    Manual,
    Scheduled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeatStage {
    Off,
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultCode {
    IgnitionFailed,
    FlameLost,
    OverTemp,
    UnderVoltage,
    SensorFault,
    InvalidState,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeInput {
    pub utc: Option<UtcTs>,
    pub utc_valid: bool,
    pub monotonic_ms: u64,
    pub delta_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SensorInput {
    pub room_temp_c: f32,
    pub hx_temp_c: f32,
    pub flame_present: bool,
    pub supply_v: f32,
    pub sensor_ok: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CommandInput {
    pub mode_request: Option<ModeRequest>,
    pub manual_setpoint_c: Option<f32>,
    pub boost_request: bool,
    pub boost_cancel: bool,
    pub fault_reset_request: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScheduleEvalInput {
    pub enabled_now: bool,
    pub target_temp_c: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TickInput {
    pub time: TimeInput,
    pub sensors: SensorInput,
    pub commands: CommandInput,
    pub schedule: ScheduleEvalInput,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActuatorIntent {
    pub fan_pct: u8,
    pub glow_on: bool,
    pub fuel_pump_hz: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Status {
    pub process_state: ProcessState,
    pub effective_mode: EffectiveMode,
    pub active_stage: HeatStage,
    pub fault: Option<FaultCode>,
    pub heat_demand: bool,
    pub schedule_inhibited_no_time: bool,
    pub config_defaulted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TickEvents {
    pub entered_state: Option<ProcessState>,
    pub fault_latched: Option<FaultCode>,
    pub fault_cleared: bool,
    pub ignition_retry_started: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TickOutput {
    pub actuators: ActuatorIntent,
    pub status: Status,
    pub events: TickEvents,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Config {
    pub preheat_ms: u32,
    pub ignition_window_ms: u32,
    pub flame_stabilize_ms: u32,
    pub retry_purge_ms: u32,
    pub cooldown_ms: u32,
    pub max_hx_temp_c: f32,
    pub min_supply_v: f32,
    pub min_run_ms: u32,
    pub min_off_ms: u32,
    pub setpoint_hysteresis_c: f32,
    pub stage_low_delta_c: f32,
    pub stage_medium_delta_c: f32,
    pub stage_high_delta_c: f32,
    pub boost_duration_ms: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            preheat_ms: 20_000,
            ignition_window_ms: 30_000,
            flame_stabilize_ms: 15_000,
            retry_purge_ms: 20_000,
            cooldown_ms: 90_000,
            max_hx_temp_c: 230.0,
            min_supply_v: 10.5,
            min_run_ms: 120_000,
            min_off_ms: 60_000,
            setpoint_hysteresis_c: 0.5,
            stage_low_delta_c: 0.3,
            stage_medium_delta_c: 1.0,
            stage_high_delta_c: 2.0,
            boost_duration_ms: 900_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SanitizedConfig {
    cfg: Config,
    used_defaults: bool,
}

impl Config {
    fn sanitize(&self) -> SanitizedConfig {
        let d = Self::default();
        let mut used_defaults = false;

        let mut cfg = *self;

        macro_rules! validate_u32 {
            ($field:ident, $min:expr, $max:expr) => {
                if !(($min..=$max).contains(&cfg.$field)) {
                    cfg.$field = d.$field;
                    used_defaults = true;
                }
            };
        }

        macro_rules! validate_f32 {
            ($field:ident, $min:expr, $max:expr) => {
                if !cfg.$field.is_finite() || cfg.$field < $min || cfg.$field > $max {
                    cfg.$field = d.$field;
                    used_defaults = true;
                }
            };
        }

        validate_u32!(preheat_ms, 50, 120_000);
        validate_u32!(ignition_window_ms, 50, 240_000);
        validate_u32!(flame_stabilize_ms, 50, 120_000);
        validate_u32!(retry_purge_ms, 50, 180_000);
        validate_u32!(cooldown_ms, 50, 600_000);
        validate_u32!(min_run_ms, 0, 3_600_000);
        validate_u32!(min_off_ms, 0, 3_600_000);
        validate_u32!(boost_duration_ms, 10_000, 86_400_000);

        validate_f32!(max_hx_temp_c, 120.0, 350.0);
        validate_f32!(min_supply_v, 6.0, 30.0);
        validate_f32!(setpoint_hysteresis_c, 0.1, 5.0);
        validate_f32!(stage_low_delta_c, 0.05, 10.0);
        validate_f32!(stage_medium_delta_c, 0.05, 15.0);
        validate_f32!(stage_high_delta_c, 0.05, 20.0);

        if !(cfg.stage_low_delta_c <= cfg.stage_medium_delta_c
            && cfg.stage_medium_delta_c <= cfg.stage_high_delta_c)
        {
            cfg.stage_low_delta_c = d.stage_low_delta_c;
            cfg.stage_medium_delta_c = d.stage_medium_delta_c;
            cfg.stage_high_delta_c = d.stage_high_delta_c;
            used_defaults = true;
        }

        if cfg.retry_purge_ms > cfg.cooldown_ms {
            cfg.retry_purge_ms = d.retry_purge_ms;
            used_defaults = true;
        }

        SanitizedConfig { cfg, used_defaults }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoreState {
    pub process_state: ProcessState,
    pub selected_mode: ModeRequest,
    pub effective_mode: EffectiveMode,
    pub latched_fault: Option<FaultCode>,
    pub ignition_attempt: u8,
    pub state_elapsed_ms: u64,
    pub run_elapsed_ms: u64,
    pub off_elapsed_ms: u64,
    pub boost_expires_at_ms: Option<u64>,
    pub last_heat_stage: HeatStage,
    pub pending_restart_after_cooldown: bool,
    pub manual_setpoint_c: f32,
}

impl Default for CoreState {
    fn default() -> Self {
        Self {
            process_state: ProcessState::Idle,
            selected_mode: ModeRequest::Off,
            effective_mode: EffectiveMode::Off,
            latched_fault: None,
            ignition_attempt: 0,
            state_elapsed_ms: 0,
            run_elapsed_ms: 0,
            off_elapsed_ms: u64::MAX,
            boost_expires_at_ms: None,
            last_heat_stage: HeatStage::Off,
            pending_restart_after_cooldown: false,
            manual_setpoint_c: 20.0,
        }
    }
}

fn is_finite_sensors(s: &SensorInput) -> bool {
    s.room_temp_c.is_finite() && s.hx_temp_c.is_finite() && s.supply_v.is_finite()
}

fn detect_safety_fault(input: &TickInput, cfg: &Config) -> Option<FaultCode> {
    if !is_finite_sensors(&input.sensors) {
        return Some(FaultCode::InvalidState);
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
            | ProcessState::FlameStabilize
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

fn stage_from_error(error_c: f32, cfg: &Config) -> HeatStage {
    if error_c >= cfg.stage_high_delta_c {
        HeatStage::High
    } else if error_c >= cfg.stage_medium_delta_c {
        HeatStage::Medium
    } else if error_c >= cfg.stage_low_delta_c {
        HeatStage::Low
    } else {
        HeatStage::Low
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

    let error_c = setpoint_c - input.sensors.room_temp_c;
    stage_from_error(error_c, cfg)
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
        ProcessState::FlameStabilize => ActuatorIntent {
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
pub fn tick(input: &TickInput, state: &mut CoreState, cfg: &Config) -> TickOutput {
    let mut events = TickEvents::default();
    let sanitized = cfg.sanitize();
    let cfg = sanitized.cfg;

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
        ProcessState::Idle => {
            if state.latched_fault.is_some() {
                transition(state, &mut events, ProcessState::Lockout);
            } else if demand.heat_demand && state.off_elapsed_ms >= u64::from(cfg.min_off_ms) {
                state.ignition_attempt = 0;
                state.run_elapsed_ms = 0;
                transition(state, &mut events, ProcessState::Precheck);
            }
        }
        ProcessState::Precheck => {
            if state.latched_fault.is_some() || !demand.heat_demand {
                transition(state, &mut events, ProcessState::Shutdown);
            } else {
                transition(state, &mut events, ProcessState::Preheat);
            }
        }
        ProcessState::Preheat => {
            if state.latched_fault.is_some() || !demand.heat_demand {
                transition(state, &mut events, ProcessState::Shutdown);
            } else if state.state_elapsed_ms >= u64::from(cfg.preheat_ms) {
                state.ignition_attempt = state.ignition_attempt.saturating_add(1).min(2);
                transition(state, &mut events, ProcessState::IgnitionTrial);
            }
        }
        ProcessState::IgnitionTrial => {
            if state.latched_fault.is_some() || !demand.heat_demand {
                transition(state, &mut events, ProcessState::Shutdown);
            } else if input.sensors.flame_present {
                transition(state, &mut events, ProcessState::FlameStabilize);
            } else if state.state_elapsed_ms >= u64::from(cfg.ignition_window_ms) {
                if state.ignition_attempt < 2 {
                    state.pending_restart_after_cooldown = true;
                    transition(state, &mut events, ProcessState::Shutdown);
                } else {
                    latch_fault(state, &mut events, FaultCode::IgnitionFailed);
                    transition(state, &mut events, ProcessState::Shutdown);
                }
            }
        }
        ProcessState::FlameStabilize => {
            if state.latched_fault.is_some() || !demand.heat_demand {
                transition(state, &mut events, ProcessState::Shutdown);
            } else if !input.sensors.flame_present {
                latch_fault(state, &mut events, FaultCode::FlameLost);
                transition(state, &mut events, ProcessState::Shutdown);
            } else if state.state_elapsed_ms >= u64::from(cfg.flame_stabilize_ms) {
                state.run_elapsed_ms = 0;
                transition(state, &mut events, ProcessState::Run);
            }
        }
        ProcessState::Run => {
            if state.latched_fault.is_some() {
                transition(state, &mut events, ProcessState::Shutdown);
            } else if !input.sensors.flame_present {
                latch_fault(state, &mut events, FaultCode::FlameLost);
                transition(state, &mut events, ProcessState::Shutdown);
            } else if manual_off {
                transition(state, &mut events, ProcessState::Shutdown);
            } else if !demand.heat_demand && state.run_elapsed_ms >= u64::from(cfg.min_run_ms) {
                transition(state, &mut events, ProcessState::Shutdown);
            }
        }
        ProcessState::Shutdown => {
            transition(state, &mut events, ProcessState::Cooldown);
        }
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
        ProcessState::IgnitionTrial | ProcessState::FlameStabilize | ProcessState::Run => {}
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
            config_defaulted: sanitized.used_defaults,
        },
        events,
    }
}
