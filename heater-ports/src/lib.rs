//! Hardware and runtime abstraction contracts for heater control.
//!
#![forbid(unsafe_code)]

use heater_core::{
    ActuatorIntent, CommandInput, ScheduleEvalInput, SensorInput, TimeInput,
};

pub trait TimeSource {
    fn sample_time(&mut self) -> TimeInput;
}

pub trait SensorSource {
    fn sample_sensors(&mut self) -> SensorInput;
}

pub trait CommandSource {
    fn sample_commands(&mut self) -> CommandInput;
    fn sample_schedule(&mut self) -> ScheduleEvalInput;
}

pub trait ActuatorSink {
    fn apply(&mut self, intent: &ActuatorIntent);
}

#[cfg(test)]
mod tests {
    use super::*;
    use heater_core::{ModeRequest, UtcTs};

    #[derive(Clone, Copy)]
    struct MockTime;

    impl TimeSource for MockTime {
        fn sample_time(&mut self) -> TimeInput {
            TimeInput {
                utc: Some(1_700_000_000 as UtcTs),
                utc_valid: true,
                monotonic_ms: 1_000,
                delta_ms: 100,
            }
        }
    }

    #[derive(Clone, Copy)]
    struct MockSensors;

    impl SensorSource for MockSensors {
        fn sample_sensors(&mut self) -> SensorInput {
            SensorInput {
                room_temp_c: 19.5,
                hx_temp_c: 45.0,
                flame_present: Some(true),
                overheat_cutoff_tripped: Some(false),
                supply_v: 12.2,
                sensor_ok: true,
            }
        }
    }

    #[derive(Clone, Copy)]
    struct MockCommands;

    impl CommandSource for MockCommands {
        fn sample_commands(&mut self) -> CommandInput {
            CommandInput {
                mode_request: Some(ModeRequest::Manual),
                manual_setpoint_c: Some(21.0),
                boost_request: false,
                boost_cancel: false,
                fault_reset_request: false,
            }
        }

        fn sample_schedule(&mut self) -> ScheduleEvalInput {
            ScheduleEvalInput {
                enabled_now: false,
                target_temp_c: None,
            }
        }
    }

    #[derive(Default)]
    struct MockActuators {
        last: Option<ActuatorIntent>,
    }

    impl ActuatorSink for MockActuators {
        fn apply(&mut self, intent: &ActuatorIntent) {
            self.last = Some(*intent);
        }
    }

    fn sample_once<T, S, C, A>(time: &mut T, sensors: &mut S, commands: &mut C, actuators: &mut A)
    where
        T: TimeSource,
        S: SensorSource,
        C: CommandSource,
        A: ActuatorSink,
    {
        let _time = time.sample_time();
        let _sensors = sensors.sample_sensors();
        let _commands = commands.sample_commands();
        let _schedule = commands.sample_schedule();

        actuators.apply(&ActuatorIntent {
            fan_pct: 10,
            glow_on: false,
            fuel_pump_hz: 0.0,
        });
    }

    #[test]
    fn mock_ports_contract_cycle() {
        let mut time = MockTime;
        let mut sensors = MockSensors;
        let mut commands = MockCommands;
        let mut actuators = MockActuators::default();

        sample_once(&mut time, &mut sensors, &mut commands, &mut actuators);

        assert_eq!(
            actuators.last,
            Some(ActuatorIntent {
                fan_pct: 10,
                glow_on: false,
                fuel_pump_hz: 0.0,
            })
        );
    }
}
