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

/// Single deterministic control step.
pub fn tick(input: &TickInput, state: &mut CoreState, cfg: &Config) -> TickOutput {
    let sanitized = cfg.sanitize();

    // v1 stub: always emits safe outputs while contract is being established.
    state.effective_mode = match state.selected_mode {
        ModeRequest::Off => EffectiveMode::Off,
        ModeRequest::Manual => EffectiveMode::Manual,
        ModeRequest::Scheduled => EffectiveMode::Scheduled,
    };

    let _ = input;

    TickOutput {
        actuators: ActuatorIntent {
            fan_pct: 0,
            glow_on: false,
            fuel_pump_hz: 0.0,
        },
        status: Status {
            process_state: state.process_state,
            effective_mode: state.effective_mode,
            active_stage: HeatStage::Off,
            fault: state.latched_fault,
            heat_demand: false,
            schedule_inhibited_no_time: false,
            config_defaulted: sanitized.used_defaults,
        },
        events: TickEvents::default(),
    }
}
