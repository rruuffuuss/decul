use heater_core::{
    CommandInput, Config, HeaterEngine, ModeRequest, ScheduleEvalInput, SensorInput, TickInput,
    TimeInput,
};

#[derive(Debug, Clone, Copy)]
struct SimWorld {
    room_temp_c: f32,
    hx_temp_c: f32,
    supply_v: f32,
    flame_present: bool,
}

impl Default for SimWorld {
    fn default() -> Self {
        Self {
            room_temp_c: 15.0,
            hx_temp_c: 20.0,
            supply_v: 12.0,
            flame_present: false,
        }
    }
}

fn default_sim_config() -> Config {
    Config {
        preheat_ms: 8_000,
        ignition_window_ms: 12_000,
        flame_stabilise_ms: 5_000,
        retry_purge_ms: 6_000,
        cooldown_ms: 20_000,
        max_hx_temp_c: 230.0,
        min_supply_v: 10.5,
        min_run_ms: 30_000,
        min_off_ms: 10_000,
        setpoint_hysteresis_c: 0.5,
        stage_low_delta_c: 0.2,
        stage_medium_delta_c: 1.0,
        stage_high_delta_c: 2.0,
        boost_duration_ms: 120_000,
        has_flame_sensor: true,
        has_overheat_cutoff: true,
        has_fan_tach: false,
        ignition_min_rise_c: 8.0,
        ignition_min_abs_c: 45.0,
        run_min_temp_c: 40.0,
        flame_loss_ms: 10_000,
        fan_stall_min_rpm: 300,
        fan_stall_ms: 3_000,
    }
}

fn run_trace(total_steps: usize, dt_ms: u32) -> Vec<String> {
    let cfg = default_sim_config();
    let mut engine = HeaterEngine::new(cfg);
    let mut world = SimWorld::default();

    let mut lines = Vec::with_capacity(total_steps.saturating_add(1));
    lines.push(
        "t_s,state,mode,stage,room_c,hx_c,flame,fan_pct,glow,fuel_hz,fault,events".to_string(),
    );

    for step in 0..total_steps {
        let monotonic_ms = (step as u64) * u64::from(dt_ms);

        let commands = scripted_commands(step);
        let schedule = scripted_schedule(step);

        let input = TickInput {
            time: TimeInput {
                utc: Some(1_700_000_000 + monotonic_ms as i64 / 1_000),
                utc_valid: true,
                monotonic_ms,
                delta_ms: dt_ms,
            },
            sensors: SensorInput {
                room_temp_c: world.room_temp_c,
                hx_temp_c: world.hx_temp_c,
                flame_present: Some(world.flame_present),
                overheat_cutoff_tripped: Some(false),
                fan_rpm: None,
                supply_v: world.supply_v,
                sensor_ok: true,
            },
            commands,
            schedule,
        };

        let output = engine.step(&input);

        update_world(
            &mut world,
            output.actuators.fuel_pump_hz,
            output.actuators.fan_pct,
        );

        let event_summary = format!(
            "enter={:?}|fault_latched={:?}|fault_cleared={}|retry={}",
            output.events.entered_state,
            output.events.fault_latched,
            output.events.fault_cleared,
            output.events.ignition_retry_started
        );

        lines.push(format!(
            "{},{:?},{:?},{:?},{:.2},{:.2},{},{},{},{:.2},{:?},{}",
            monotonic_ms / 1_000,
            output.status.process_state,
            output.status.effective_mode,
            output.status.active_stage,
            world.room_temp_c,
            world.hx_temp_c,
            world.flame_present,
            output.actuators.fan_pct,
            output.actuators.glow_on,
            output.actuators.fuel_pump_hz,
            output.status.fault,
            event_summary,
        ));
    }

    lines
}

fn main() {
    for line in run_trace(180, 1_000) {
        println!("{line}");
    }
}

fn scripted_commands(step: usize) -> CommandInput {
    // Scenario:
    // 0-39s manual heat to 21C
    // 40-79s scheduled mode with 20C target
    // 80s boost request
    // 120s explicit off
    // 140s manual back on
    let mode_request = match step {
        0 => Some(ModeRequest::Manual),
        40 => Some(ModeRequest::Scheduled),
        120 => Some(ModeRequest::Off),
        140 => Some(ModeRequest::Manual),
        _ => None,
    };

    CommandInput {
        mode_request,
        manual_setpoint_c: Some(21.0),
        boost_request: step == 80,
        boost_cancel: step == 110,
        fault_reset_request: false,
    }
}

fn scripted_schedule(step: usize) -> ScheduleEvalInput {
    if (40..120).contains(&step) {
        ScheduleEvalInput {
            enabled_now: true,
            target_temp_c: Some(20.0),
        }
    } else {
        ScheduleEvalInput {
            enabled_now: false,
            target_temp_c: None,
        }
    }
}

fn update_world(world: &mut SimWorld, fuel_hz: f32, fan_pct: u8) {
    // Very simple thermal model for deterministic behavior regression.
    let combustion_heat = fuel_hz * 0.22;
    let cooling = (fan_pct as f32 / 100.0) * 0.09;

    world.hx_temp_c += combustion_heat - cooling;
    world.hx_temp_c = world.hx_temp_c.clamp(10.0, 260.0);

    let room_gain = (world.hx_temp_c - world.room_temp_c) * 0.012;
    world.room_temp_c += room_gain - 0.01; // ambient loss
    world.room_temp_c = world.room_temp_c.clamp(-20.0, 45.0);

    // Flame signal is derived from enough fuel delivery and exchanger temperature support.
    world.flame_present = fuel_hz > 1.0 && world.hx_temp_c > 35.0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_trace_regression() {
        let actual = run_trace(11, 1_000);
        let expected = vec![
            "t_s,state,mode,stage,room_c,hx_c,flame,fan_pct,glow,fuel_hz,fault,events",
            "0,Precheck,Manual,Off,15.05,20.00,false,0,false,0.00,None,enter=Some(Precheck)|fault_latched=None|fault_cleared=false|retry=false",
            "1,Preheat,Manual,Off,15.10,19.98,false,25,true,0.00,None,enter=Some(Preheat)|fault_latched=None|fault_cleared=false|retry=false",
            "2,Preheat,Manual,Off,15.15,19.96,false,25,true,0.00,None,enter=None|fault_latched=None|fault_cleared=false|retry=false",
            "3,Preheat,Manual,Off,15.19,19.93,false,25,true,0.00,None,enter=None|fault_latched=None|fault_cleared=false|retry=false",
            "4,Preheat,Manual,Off,15.24,19.91,false,25,true,0.00,None,enter=None|fault_latched=None|fault_cleared=false|retry=false",
            "5,Preheat,Manual,Off,15.29,19.89,false,25,true,0.00,None,enter=None|fault_latched=None|fault_cleared=false|retry=false",
            "6,Preheat,Manual,Off,15.33,19.87,false,25,true,0.00,None,enter=None|fault_latched=None|fault_cleared=false|retry=false",
            "7,Preheat,Manual,Off,15.38,19.84,false,25,true,0.00,None,enter=None|fault_latched=None|fault_cleared=false|retry=false",
            "8,Preheat,Manual,Off,15.42,19.82,false,25,true,0.00,None,enter=None|fault_latched=None|fault_cleared=false|retry=false",
            "9,IgnitionTrial,Manual,Off,15.47,20.14,false,35,true,1.60,None,enter=Some(IgnitionTrial)|fault_latched=None|fault_cleared=false|retry=false",
            "10,IgnitionTrial,Manual,Off,15.52,20.46,false,35,true,1.60,None,enter=None|fault_latched=None|fault_cleared=false|retry=false",
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();

        assert_eq!(actual, expected);
    }
}
