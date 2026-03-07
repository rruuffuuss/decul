//! Configuration model and in tick sanitisation.
//!
//! Sanitisation is kept internal and deterministic so runtime code keep ticking safely if configuration input is partially invalid.
//!
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Config {
    pub preheat_ms: u32,
    pub ignition_window_ms: u32,
    pub flame_stabilise_ms: u32,
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
    pub has_flame_sensor: bool,
    pub has_overheat_cutoff: bool,
    pub has_fan_tach: bool,
    pub ignition_min_rise_c: f32,
    pub ignition_min_abs_c: f32,
    pub run_min_temp_c: f32,
    pub flame_loss_ms: u32,
    pub fan_stall_min_rpm: u16,
    pub fan_stall_ms: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            preheat_ms: 20_000,
            ignition_window_ms: 30_000,
            flame_stabilise_ms: 15_000,
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
            has_flame_sensor: false,
            has_overheat_cutoff: false,
            has_fan_tach: false,
            ignition_min_rise_c: 8.0,
            ignition_min_abs_c: 45.0,
            run_min_temp_c: 40.0,
            flame_loss_ms: 10_000,
            fan_stall_min_rpm: 300,
            fan_stall_ms: 3_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SanitisedConfig {
    pub(crate) cfg: Config,
    pub(crate) used_defaults: bool,
}

impl Config {
    pub(crate) fn sanitise(&self) -> SanitisedConfig {
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

        macro_rules! validate_u16 {
            ($field:ident, $min:expr, $max:expr) => {
                if !(($min..=$max).contains(&cfg.$field)) {
                    cfg.$field = d.$field;
                    used_defaults = true;
                }
            };
        }

        validate_u32!(preheat_ms, 50, 120_000);
        validate_u32!(ignition_window_ms, 50, 240_000);
        validate_u32!(flame_stabilise_ms, 50, 120_000);
        validate_u32!(retry_purge_ms, 50, 180_000);
        validate_u32!(cooldown_ms, 50, 600_000);
        validate_u32!(min_run_ms, 0, 3_600_000);
        validate_u32!(min_off_ms, 0, 3_600_000);
        validate_u32!(boost_duration_ms, 10_000, 86_400_000);
        validate_u32!(flame_loss_ms, 100, 600_000);
        validate_u32!(fan_stall_ms, 100, 60_000);

        validate_f32!(max_hx_temp_c, 120.0, 350.0);
        validate_f32!(min_supply_v, 6.0, 30.0);
        validate_f32!(setpoint_hysteresis_c, 0.1, 5.0);
        validate_f32!(stage_low_delta_c, 0.05, 10.0);
        validate_f32!(stage_medium_delta_c, 0.05, 15.0);
        validate_f32!(stage_high_delta_c, 0.05, 20.0);
        validate_f32!(ignition_min_rise_c, 0.1, 100.0);
        validate_f32!(ignition_min_abs_c, -20.0, 200.0);
        validate_f32!(run_min_temp_c, -20.0, 200.0);
        validate_u16!(fan_stall_min_rpm, 50, 20_000);

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

        if cfg.run_min_temp_c > cfg.ignition_min_abs_c {
            cfg.run_min_temp_c = d.run_min_temp_c;
            cfg.ignition_min_abs_c = d.ignition_min_abs_c;
            used_defaults = true;
        }

        if cfg.ignition_min_abs_c >= cfg.max_hx_temp_c {
            cfg.ignition_min_abs_c = d.ignition_min_abs_c;
            used_defaults = true;
        }

        SanitisedConfig { cfg, used_defaults }
    }
}
