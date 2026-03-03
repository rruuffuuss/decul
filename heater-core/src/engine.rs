//! Public control API for stepping the heater state machine.
//!
//! `HeaterEngine` owns runtime state and configuration whilst the logic is handled by the FSM module.
//!
use crate::{Config, CoreState, TickInput, TickOutput, fsm};

#[derive(Debug, Clone)]
pub struct HeaterEngine {
    cfg: Config,
    state: CoreState,
}

impl HeaterEngine {
    pub fn new(cfg: Config) -> Self {
        Self {
            cfg,
            state: CoreState::default(),
        }
    }

    pub fn with_state(cfg: Config, state: CoreState) -> Self {
        Self { cfg, state }
    }

    pub fn step(&mut self, input: &TickInput) -> TickOutput {
        fsm::tick(input, &mut self.state, &self.cfg)
    }

    pub fn state(&self) -> &CoreState {
        &self.state
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    pub fn set_config(&mut self, cfg: Config) {
        self.cfg = cfg;
    }
}
