#![forbid(unsafe_code)]

mod config;
mod engine;
mod fsm;
#[cfg(test)]
mod tests;
mod types;

pub use config::Config;
pub use engine::HeaterEngine;
pub use types::{
    ActuatorIntent, CommandInput, CoreState, EffectiveMode, FaultCode, HeatStage, ModeRequest,
    ProcessState, ScheduleEvalInput, SensorInput, Status, TickEvents, TickInput, TickOutput,
    TimeInput, UtcTs,
};
