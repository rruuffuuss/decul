//! Public domain types for heater control.
//!
//! These types are shared by simulation and embedded runtimes.
//!
pub type UtcTs = i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Idle,
    Precheck,
    Preheat,
    IgnitionTrial,
    FlameStabilise,
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
    FanStall,
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
    pub flame_present: Option<bool>,
    pub overheat_cutoff_tripped: Option<bool>,
    pub fan_rpm: Option<u16>,
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
    pub ignition_baseline_temp_c: f32,
    pub low_temp_elapsed_ms: u64,
    pub fan_stall_elapsed_ms: u64,
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
            ignition_baseline_temp_c: 0.0,
            low_temp_elapsed_ms: 0,
            fan_stall_elapsed_ms: 0,
        }
    }
}
