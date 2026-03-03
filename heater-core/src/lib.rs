//! Platform independent heater control core.
//!
//! This crate is deterministic by design:
//! - no direct hardware I/O
//! - no blocking calls or sleeps
//! - same input stream produces the same output stream
//!
//! Public interfaces intentionally use core primitives (as opposed to platform dependent STD) to keep
//! embedded portability straightforward as the project evolves.
//!
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
