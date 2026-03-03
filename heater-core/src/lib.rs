#![forbid(unsafe_code)]

mod config;
mod fsm;
mod types;

pub use config::Config;
pub use fsm::tick;
pub use types::{
    ActuatorIntent, CommandInput, CoreState, EffectiveMode, FaultCode, HeatStage, ModeRequest,
    ProcessState, ScheduleEvalInput, SensorInput, Status, TickEvents, TickInput, TickOutput,
    TimeInput, UtcTs,
};

#[cfg(test)]
mod tests {
    use super::*;

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

    fn run_to_run_state(state: &mut CoreState, cfg: &Config) {
        // Start command.
        let mut t = 0;
        let mut i = input(t, 100, Some(ModeRequest::Manual), false, None);
        tick(&i, state, cfg);

        // Precheck -> Preheat.
        t += 100;
        i.time.monotonic_ms = t;
        tick(&i, state, cfg);

        // Preheat elapsed.
        t += 100;
        i.time.monotonic_ms = t;
        tick(&i, state, cfg);

        // Ignition with flame present.
        t += 100;
        i.time.monotonic_ms = t;
        i.sensors.flame_present = true;
        tick(&i, state, cfg);

        // Stabilise to Run.
        t += 100;
        i.time.monotonic_ms = t;
        tick(&i, state, cfg);

        assert_eq!(state.process_state, ProcessState::Run);
    }

    #[test]
    fn progression_reaches_run() {
        let cfg = test_cfg();
        let mut state = CoreState::default();
        let mut t = 0;

        let mut i = input(t, 100, Some(ModeRequest::Manual), false, None);
        let out0 = tick(&i, &mut state, &cfg);
        assert_eq!(out0.status.process_state, ProcessState::Precheck);

        t += 100;
        i.time.monotonic_ms = t;
        let out1 = tick(&i, &mut state, &cfg);
        assert_eq!(out1.status.process_state, ProcessState::Preheat);

        t += 100;
        i.time.monotonic_ms = t;
        let out2 = tick(&i, &mut state, &cfg);
        assert_eq!(out2.status.process_state, ProcessState::IgnitionTrial);

        t += 100;
        i.time.monotonic_ms = t;
        i.sensors.flame_present = true;
        let out3 = tick(&i, &mut state, &cfg);
        assert_eq!(out3.status.process_state, ProcessState::FlameStabilise);

        t += 100;
        i.time.monotonic_ms = t;
        let out4 = tick(&i, &mut state, &cfg);
        assert_eq!(out4.status.process_state, ProcessState::Run);
        assert_eq!(out4.actuators.fuel_pump_hz > 0.0, true);
    }

    #[test]
    fn ignition_two_tries_then_lockout() {
        let cfg = test_cfg();
        let mut state = CoreState::default();

        let mut t = 0;
        let mut i = input(t, 100, Some(ModeRequest::Manual), false, None);

        // Walk through first failed ignition.
        tick(&i, &mut state, &cfg); // precheck
        t += 100;
        i.time.monotonic_ms = t;
        tick(&i, &mut state, &cfg); // preheat
        t += 100;
        i.time.monotonic_ms = t;
        tick(&i, &mut state, &cfg); // ignition trial attempt 1
        t += 100;
        i.time.monotonic_ms = t;
        let out_fail_1 = tick(&i, &mut state, &cfg); // shutdown pending retry
        assert_eq!(out_fail_1.status.process_state, ProcessState::Shutdown);

        t += 100;
        i.time.monotonic_ms = t;
        tick(&i, &mut state, &cfg); // cooldown
        t += 100;
        i.time.monotonic_ms = t;
        let out_retry = tick(&i, &mut state, &cfg); // preheat retry
        assert_eq!(out_retry.events.ignition_retry_started, true);
        assert_eq!(out_retry.status.process_state, ProcessState::Preheat);

        t += 100;
        i.time.monotonic_ms = t;
        tick(&i, &mut state, &cfg); // ignition trial attempt 2
        t += 100;
        i.time.monotonic_ms = t;
        let out_fail_2 = tick(&i, &mut state, &cfg); // shutdown + latched fault
        assert_eq!(out_fail_2.status.fault, Some(FaultCode::IgnitionFailed));

        t += 100;
        i.time.monotonic_ms = t;
        tick(&i, &mut state, &cfg); // cooldown
        t += 100;
        i.time.monotonic_ms = t;
        let out_lockout = tick(&i, &mut state, &cfg); // lockout
        assert_eq!(out_lockout.status.process_state, ProcessState::Lockout);
        assert_eq!(out_lockout.actuators.fuel_pump_hz, 0.0);
        assert_eq!(out_lockout.actuators.glow_on, false);
    }

    #[test]
    fn flame_loss_in_run_latches_fault() {
        let cfg = test_cfg();
        let mut state = CoreState::default();
        run_to_run_state(&mut state, &cfg);

        let mut i = input(1_000, 100, None, false, None);
        i.sensors.flame_present = false;
        let out = tick(&i, &mut state, &cfg);

        assert_eq!(out.status.process_state, ProcessState::Shutdown);
        assert_eq!(out.status.fault, Some(FaultCode::FlameLost));
    }

    #[test]
    fn safety_fault_overtemp_forces_safe_outputs() {
        let cfg = test_cfg();
        let mut state = CoreState::default();
        let mut i = input(0, 100, Some(ModeRequest::Manual), false, None);
        i.sensors.hx_temp_c = 300.0;

        let out = tick(&i, &mut state, &cfg);
        assert_eq!(out.status.fault, Some(FaultCode::OverTemp));
        assert_eq!(out.status.process_state, ProcessState::Lockout);
        assert_eq!(out.actuators.fuel_pump_hz, 0.0);
        assert_eq!(out.actuators.glow_on, false);
    }

    #[test]
    fn schedule_inhibited_without_valid_time_but_manual_works() {
        let cfg = test_cfg();
        let mut state = CoreState::default();

        let mut i = input(0, 100, Some(ModeRequest::Scheduled), true, Some(22.0));
        i.time.utc_valid = false;
        let out_schedule = tick(&i, &mut state, &cfg);
        assert_eq!(out_schedule.status.schedule_inhibited_no_time, true);
        assert_eq!(out_schedule.status.process_state, ProcessState::Idle);

        i.commands.mode_request = Some(ModeRequest::Manual);
        i.time.utc_valid = false;
        let out_manual = tick(&i, &mut state, &cfg);
        assert_eq!(out_manual.status.process_state, ProcessState::Precheck);
    }

    #[test]
    fn mode_priority_manual_off_overrides_boost_and_schedule() {
        let cfg = test_cfg();
        let mut state = CoreState::default();

        let mut i = input(0, 100, Some(ModeRequest::Scheduled), true, Some(22.0));
        i.commands.boost_request = true;
        tick(&i, &mut state, &cfg);
        assert_eq!(state.effective_mode, EffectiveMode::Boost);

        i.time.monotonic_ms = 100;
        i.commands.mode_request = Some(ModeRequest::Off);
        i.commands.boost_request = false;
        let out = tick(&i, &mut state, &cfg);
        assert_eq!(out.status.effective_mode, EffectiveMode::Off);
        assert_eq!(out.status.heat_demand, false);
    }

    #[test]
    fn boost_overrides_schedule() {
        let cfg = test_cfg();
        let mut state = CoreState::default();

        let mut i = input(0, 100, Some(ModeRequest::Scheduled), true, Some(18.0));
        i.sensors.room_temp_c = 17.8;
        tick(&i, &mut state, &cfg);
        assert_eq!(state.effective_mode, EffectiveMode::Scheduled);

        i.time.monotonic_ms = 100;
        i.commands.mode_request = None;
        i.commands.boost_request = true;
        let out = tick(&i, &mut state, &cfg);
        assert_eq!(out.status.effective_mode, EffectiveMode::Boost);
        assert_eq!(out.status.heat_demand, true);
    }

    #[test]
    fn anti_short_cycle_min_run_and_min_off() {
        let cfg = test_cfg();
        let mut state = CoreState::default();
        run_to_run_state(&mut state, &cfg);

        // Demand drops but min_run should hold run state.
        let mut i = input(2_000, 100, None, false, None);
        i.sensors.room_temp_c = 30.0;
        i.sensors.flame_present = true;
        let out_hold = tick(&i, &mut state, &cfg);
        assert_eq!(out_hold.status.process_state, ProcessState::Run);

        // Advance beyond min_run and demand remains false.
        i.time.monotonic_ms = 2_300;
        i.time.delta_ms = 300;
        i.sensors.flame_present = true;
        let out_stop = tick(&i, &mut state, &cfg);
        assert_eq!(out_stop.status.process_state, ProcessState::Shutdown);

        // Finish cooldown to Idle.
        i.time.monotonic_ms = 2_400;
        i.time.delta_ms = 100;
        tick(&i, &mut state, &cfg);
        i.time.monotonic_ms = 2_500;
        tick(&i, &mut state, &cfg);
        assert_eq!(state.process_state, ProcessState::Idle);

        // Immediate restart blocked by min_off.
        i.time.monotonic_ms = 2_600;
        i.time.delta_ms = 100;
        i.commands.mode_request = Some(ModeRequest::Manual);
        i.sensors.room_temp_c = 10.0;
        let out_blocked = tick(&i, &mut state, &cfg);
        assert_eq!(out_blocked.status.process_state, ProcessState::Idle);

        // After min_off elapsed, restart permitted.
        i.time.monotonic_ms = 2_900;
        i.time.delta_ms = 300;
        let out_start = tick(&i, &mut state, &cfg);
        assert_eq!(out_start.status.process_state, ProcessState::Precheck);
    }

    #[test]
    fn fault_reset_requires_clear_conditions() {
        let cfg = test_cfg();
        let mut state = CoreState::default();

        let mut i = input(0, 100, Some(ModeRequest::Manual), false, None);
        i.sensors.hx_temp_c = 400.0;
        tick(&i, &mut state, &cfg);
        assert_eq!(state.process_state, ProcessState::Lockout);

        i.time.monotonic_ms = 100;
        i.commands.fault_reset_request = true;
        let out_reject = tick(&i, &mut state, &cfg);
        assert_eq!(out_reject.status.process_state, ProcessState::Lockout);

        i.time.monotonic_ms = 200;
        i.sensors.hx_temp_c = 40.0;
        let out_accept = tick(&i, &mut state, &cfg);
        assert_eq!(out_accept.events.fault_cleared, true);
        assert_eq!(out_accept.status.process_state, ProcessState::Idle);
    }

    #[test]
    fn deterministic_for_identical_input_stream() {
        let cfg = test_cfg();
        let mut a = CoreState::default();
        let mut b = CoreState::default();

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

        let out_a: Vec<_> = stream.iter().map(|i| tick(i, &mut a, &cfg)).collect();
        let out_b: Vec<_> = stream.iter().map(|i| tick(i, &mut b, &cfg)).collect();

        assert_eq!(out_a, out_b);
        assert_eq!(a, b);
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

        let mut state = CoreState::default();
        let i = input(0, 100, Some(ModeRequest::Manual), false, None);
        let out = tick(&i, &mut state, &cfg);

        assert_eq!(out.status.config_defaulted, true);
    }
}
