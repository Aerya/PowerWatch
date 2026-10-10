//! Board-level power estimate from CPU load.
//!
//! Most ARM single-board computers (Raspberry Pi 2/3/4, Odroid, ...) expose no
//! power telemetry at all: no RAPL, no hwmon power counter. For those machines
//! PowerWatch can estimate the power drawn by the whole board from its CPU
//! utilisation, using two coefficients measured once with a plug-in power
//! meter:
//!
//! ```text
//! watts = idle_watts + (max_watts - idle_watts) * utilisation
//! ```
//!
//! The model is only enabled when both coefficients are configured
//! (`POWERWATCH_BOARD_IDLE_W` and `POWERWATCH_BOARD_MAX_W`); PowerWatch never
//! guesses them. Readings are always reported as [`Confidence::Estimated`].
//!
//! Because the coefficients are measured at the wall, the estimate already
//! includes RAM, storage and power-supply losses. The separate RAM and disk
//! estimates must therefore not be added on top of it (see
//! `runtime::CpuSource::is_board_level`).

use crate::model::{Component, Confidence, PowerSensor, SensorError, SensorReading};
use std::path::{Path, PathBuf};

pub const IDLE_WATTS_ENV: &str = "POWERWATCH_BOARD_IDLE_W";
pub const MAX_WATTS_ENV: &str = "POWERWATCH_BOARD_MAX_W";

/// Aggregate CPU time counters read from the first `cpu` line of
/// `/proc/stat`, in clock ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuJiffies {
    /// `idle + iowait`.
    pub idle: u64,
    /// `user + nice + system + idle + iowait + irq + softirq + steal`.
    ///
    /// `guest` and `guest_nice` are left out on purpose: the kernel already
    /// accounts them inside `user` and `nice`.
    pub total: u64,
}

pub fn parse_proc_stat(contents: &str) -> Result<CpuJiffies, SensorError> {
    let line = contents
        .lines()
        .find(|line| line.split_whitespace().next() == Some("cpu"))
        .ok_or_else(|| {
            SensorError::ReadFailed("no aggregate \"cpu\" line in /proc/stat".to_string())
        })?;

    let fields = line
        .split_whitespace()
        .skip(1)
        .map(|field| field.parse::<u64>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| SensorError::ReadFailed(format!("invalid /proc/stat value: {e}")))?;

    if fields.len() < 4 {
        return Err(SensorError::ReadFailed(format!(
            "expected at least 4 fields on the /proc/stat cpu line, found {}",
            fields.len()
        )));
    }

    let field = |index: usize| fields.get(index).copied().unwrap_or(0);
    let idle = field(3).saturating_add(field(4));
    let total = (0..8).map(field).fold(0u64, u64::saturating_add);

    Ok(CpuJiffies { idle, total })
}

/// Share of the elapsed CPU time that was not idle, between 0.0 and 1.0.
///
/// Returns 0.0 when no time elapsed or when the counters went backwards
/// (CPU hot-unplug), instead of reporting a nonsensical value.
pub fn compute_utilization(before: &CpuJiffies, after: &CpuJiffies) -> f64 {
    if after.total <= before.total {
        return 0.0;
    }

    let total_delta = after.total - before.total;
    let idle_delta = after.idle.saturating_sub(before.idle).min(total_delta);
    let busy_delta = total_delta - idle_delta;

    (busy_delta as f64 / total_delta as f64).clamp(0.0, 1.0)
}

/// Linear board power model: `idle_watts` at 0 % CPU, `max_watts` at 100 %.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoardPowerModel {
    idle_watts: f64,
    max_watts: f64,
}

impl BoardPowerModel {
    pub fn new(idle_watts: f64, max_watts: f64) -> Result<Self, String> {
        if !idle_watts.is_finite() || !max_watts.is_finite() {
            return Err("board power coefficients must be finite numbers".to_string());
        }
        if idle_watts < 0.0 {
            return Err(format!(
                "{IDLE_WATTS_ENV} must not be negative (got {idle_watts})"
            ));
        }
        if max_watts < idle_watts {
            return Err(format!(
                "{MAX_WATTS_ENV} ({max_watts}) must be greater than or equal to \
                 {IDLE_WATTS_ENV} ({idle_watts})"
            ));
        }

        Ok(Self {
            idle_watts,
            max_watts,
        })
    }

    pub fn idle_watts(&self) -> f64 {
        self.idle_watts
    }

    pub fn max_watts(&self) -> f64 {
        self.max_watts
    }

    pub fn watts_at(&self, utilization: f64) -> f64 {
        let utilization = utilization.clamp(0.0, 1.0);
        self.idle_watts + (self.max_watts - self.idle_watts) * utilization
    }
}

/// Builds the board model from the raw values of the two environment
/// variables.
///
/// * both unset (or empty, as Docker Compose produces for `${VAR:-}`) →
///   `Ok(None)`: the model is simply not used;
/// * only one set → configuration error;
/// * both set → parsed and validated. A decimal comma (`2,7`) is accepted as
///   well as a decimal point.
pub fn board_model_from_values(
    idle: Option<&str>,
    max: Option<&str>,
) -> Result<Option<BoardPowerModel>, String> {
    let idle = idle.map(str::trim).filter(|value| !value.is_empty());
    let max = max.map(str::trim).filter(|value| !value.is_empty());

    match (idle, max) {
        (None, None) => Ok(None),
        (Some(_), None) | (None, Some(_)) => Err(format!(
            "{IDLE_WATTS_ENV} and {MAX_WATTS_ENV} must both be set to enable the \
             CPU-load board model"
        )),
        (Some(idle), Some(max)) => {
            let idle_watts = parse_watts(IDLE_WATTS_ENV, idle)?;
            let max_watts = parse_watts(MAX_WATTS_ENV, max)?;
            BoardPowerModel::new(idle_watts, max_watts).map(Some)
        }
    }
}

/// Reads the two environment variables and builds the board model.
pub fn board_model_from_env() -> Result<Option<BoardPowerModel>, String> {
    let idle = std::env::var(IDLE_WATTS_ENV).ok();
    let max = std::env::var(MAX_WATTS_ENV).ok();
    board_model_from_values(idle.as_deref(), max.as_deref())
}

fn parse_watts(name: &str, value: &str) -> Result<f64, String> {
    value
        .replace(',', ".")
        .parse::<f64>()
        .map_err(|_| format!("{name} is not a number: {value:?}"))
}

pub trait CpuJiffiesSource: Send {
    fn read(&mut self) -> Result<CpuJiffies, SensorError>;
}

pub struct ProcStatSource {
    path: PathBuf,
}

impl ProcStatSource {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl CpuJiffiesSource for ProcStatSource {
    fn read(&mut self) -> Result<CpuJiffies, SensorError> {
        let contents = read_file(&self.path)?;
        parse_proc_stat(&contents)
    }
}

fn read_file(path: &Path) -> Result<String, SensorError> {
    std::fs::read_to_string(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => {
            SensorError::Unavailable(format!("{} does not exist", path.display()))
        }
        std::io::ErrorKind::PermissionDenied => {
            SensorError::PermissionDenied(format!("no read access to {}", path.display()))
        }
        _ => SensorError::ReadFailed(e.to_string()),
    })
}

/// Estimated whole-board power, reported on the `cpu` sensor so that the Hub,
/// alerts and history keep working unchanged.
pub struct CpuLoadModelSensor<S: CpuJiffiesSource> {
    source: S,
    model: BoardPowerModel,
    last: Option<CpuJiffies>,
}

impl<S: CpuJiffiesSource> CpuLoadModelSensor<S> {
    pub fn new(source: S, model: BoardPowerModel) -> Self {
        Self {
            source,
            model,
            last: None,
        }
    }
}

impl<S: CpuJiffiesSource> PowerSensor for CpuLoadModelSensor<S> {
    fn sample(&mut self) -> Result<SensorReading, SensorError> {
        let jiffies = self.source.read()?;

        let Some(last) = self.last.replace(jiffies) else {
            return Err(SensorError::NotReady(
                "need a second sample to compute CPU utilisation".to_string(),
            ));
        };

        let utilization = compute_utilization(&last, &jiffies);

        Ok(SensorReading {
            component: Component::Cpu,
            watts: self.model.watts_at(utilization),
            confidence: Confidence::Estimated,
            timestamp: chrono::Utc::now(),
        })
    }
}

/// Registered instead of the model when its configuration is invalid, so the
/// problem shows up in the CLI, TUI and Web UI rather than being ignored.
pub struct InvalidBoardModelSensor {
    message: String,
}

impl InvalidBoardModelSensor {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl PowerSensor for InvalidBoardModelSensor {
    fn sample(&mut self) -> Result<SensorReading, SensorError> {
        Err(SensorError::Unavailable(format!(
            "invalid board power model configuration: {}",
            self.message
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Aggregate line followed by per-core lines, as on a 4-core board.
    const PROC_STAT: &str = "\
cpu  1000 50 400 8000 200 30 20 0 100 10
cpu0 250 10 100 2000 50 10 5 0 25 5
cpu1 250 10 100 2000 50 10 5 0 25 5
cpu2 250 15 100 2000 50 5 5 0 25 0
cpu3 250 15 100 2000 50 5 5 0 25 0
intr 123456 0 0
ctxt 987654
";

    fn jiffies(idle: u64, total: u64) -> CpuJiffies {
        CpuJiffies { idle, total }
    }

    #[test]
    fn parses_the_aggregate_cpu_line() {
        let parsed = parse_proc_stat(PROC_STAT).unwrap();

        assert_eq!(parsed.idle, 8000 + 200);
        assert_eq!(parsed.total, 1000 + 50 + 400 + 8000 + 200 + 30 + 20);
    }

    #[test]
    fn guest_time_is_not_counted_twice() {
        let without_guest = parse_proc_stat("cpu  10 0 10 80 0 0 0 0\n").unwrap();
        let with_guest = parse_proc_stat("cpu  10 0 10 80 0 0 0 0 7 3\n").unwrap();

        assert_eq!(with_guest.total, without_guest.total);
    }

    #[test]
    fn accepts_old_kernels_with_only_four_fields() {
        let parsed = parse_proc_stat("cpu  10 0 10 80\n").unwrap();

        assert_eq!(parsed, jiffies(80, 100));
    }

    #[test]
    fn reports_a_missing_aggregate_line() {
        let error = parse_proc_stat("cpu0 1 2 3 4\nintr 5\n").unwrap_err();

        assert!(matches!(error, SensorError::ReadFailed(_)));
    }

    #[test]
    fn reports_a_non_numeric_field() {
        let error = parse_proc_stat("cpu  1 2 x 4\n").unwrap_err();

        assert!(matches!(error, SensorError::ReadFailed(_)));
    }

    #[test]
    fn reports_a_truncated_line() {
        let error = parse_proc_stat("cpu  1 2 3\n").unwrap_err();

        assert!(matches!(error, SensorError::ReadFailed(_)));
    }

    #[test]
    fn fully_idle_interval_is_zero_utilization() {
        assert_eq!(
            compute_utilization(&jiffies(100, 200), &jiffies(200, 300)),
            0.0
        );
    }

    #[test]
    fn fully_busy_interval_is_full_utilization() {
        assert_eq!(
            compute_utilization(&jiffies(100, 200), &jiffies(100, 300)),
            1.0
        );
    }

    #[test]
    fn half_busy_interval_is_half_utilization() {
        assert_eq!(
            compute_utilization(&jiffies(100, 200), &jiffies(150, 300)),
            0.5
        );
    }

    #[test]
    fn no_elapsed_time_does_not_divide_by_zero() {
        assert_eq!(
            compute_utilization(&jiffies(100, 200), &jiffies(100, 200)),
            0.0
        );
    }

    #[test]
    fn counters_going_backwards_do_not_panic() {
        assert_eq!(
            compute_utilization(&jiffies(500, 900), &jiffies(100, 200)),
            0.0
        );
    }

    #[test]
    fn idle_growing_faster_than_total_is_clamped() {
        assert_eq!(
            compute_utilization(&jiffies(100, 200), &jiffies(400, 300)),
            0.0
        );
    }

    #[test]
    fn model_interpolates_between_idle_and_max() {
        let model = BoardPowerModel::new(2.7, 6.4).unwrap();

        assert_eq!(model.watts_at(0.0), 2.7);
        assert_eq!(model.watts_at(1.0), 6.4);
        assert!((model.watts_at(0.5) - 4.55).abs() < 1e-9);
    }

    #[test]
    fn model_clamps_out_of_range_utilization() {
        let model = BoardPowerModel::new(2.0, 6.0).unwrap();

        assert_eq!(model.watts_at(-0.5), 2.0);
        assert_eq!(model.watts_at(1.5), 6.0);
    }

    #[test]
    fn model_accepts_equal_idle_and_max() {
        assert!(BoardPowerModel::new(3.0, 3.0).is_ok());
    }

    #[test]
    fn model_rejects_negative_idle() {
        assert!(BoardPowerModel::new(-1.0, 3.0).is_err());
    }

    #[test]
    fn model_rejects_max_below_idle() {
        let error = BoardPowerModel::new(5.0, 3.0).unwrap_err();

        assert!(error.contains(MAX_WATTS_ENV));
    }

    #[test]
    fn model_rejects_non_finite_values() {
        assert!(BoardPowerModel::new(f64::NAN, 3.0).is_err());
        assert!(BoardPowerModel::new(1.0, f64::INFINITY).is_err());
    }

    #[test]
    fn no_configuration_means_no_model() {
        assert_eq!(board_model_from_values(None, None), Ok(None));
    }

    #[test]
    fn empty_values_from_compose_mean_no_model() {
        assert_eq!(board_model_from_values(Some(""), Some("  ")), Ok(None));
    }

    #[test]
    fn a_single_coefficient_is_a_configuration_error() {
        let error = board_model_from_values(Some("2.7"), None).unwrap_err();

        assert!(error.contains(IDLE_WATTS_ENV) && error.contains(MAX_WATTS_ENV));
        assert!(board_model_from_values(None, Some("6.4")).is_err());
    }

    #[test]
    fn coefficients_are_parsed_and_trimmed() {
        let model = board_model_from_values(Some(" 2.7 "), Some("6.4"))
            .unwrap()
            .unwrap();

        assert_eq!(model.idle_watts(), 2.7);
        assert_eq!(model.max_watts(), 6.4);
    }

    #[test]
    fn a_decimal_comma_is_accepted() {
        let model = board_model_from_values(Some("2,7"), Some("6,4"))
            .unwrap()
            .unwrap();

        assert_eq!(model, BoardPowerModel::new(2.7, 6.4).unwrap());
    }

    #[test]
    fn a_non_numeric_coefficient_names_the_variable() {
        let error = board_model_from_values(Some("2.7"), Some("six")).unwrap_err();

        assert!(error.contains(MAX_WATTS_ENV));
    }

    #[test]
    fn invalid_coefficients_are_rejected() {
        assert!(board_model_from_values(Some("6"), Some("2")).is_err());
    }

    struct FakeSource {
        readings: Vec<Result<CpuJiffies, SensorError>>,
    }

    impl CpuJiffiesSource for FakeSource {
        fn read(&mut self) -> Result<CpuJiffies, SensorError> {
            self.readings.remove(0)
        }
    }

    fn sensor_with(
        readings: Vec<Result<CpuJiffies, SensorError>>,
    ) -> CpuLoadModelSensor<FakeSource> {
        CpuLoadModelSensor::new(
            FakeSource { readings },
            BoardPowerModel::new(2.0, 6.0).unwrap(),
        )
    }

    #[test]
    fn first_sample_is_not_ready() {
        let mut sensor = sensor_with(vec![Ok(jiffies(100, 200))]);

        assert!(matches!(sensor.sample(), Err(SensorError::NotReady(_))));
    }

    #[test]
    fn second_sample_is_an_estimated_cpu_reading() {
        let mut sensor = sensor_with(vec![Ok(jiffies(100, 200)), Ok(jiffies(150, 300))]);

        let _ = sensor.sample();
        let reading = sensor.sample().unwrap();

        assert_eq!(reading.component, Component::Cpu);
        assert_eq!(reading.confidence, Confidence::Estimated);
        assert_eq!(reading.watts, 4.0);
    }

    #[test]
    fn source_errors_are_propagated() {
        let mut sensor = sensor_with(vec![Err(SensorError::PermissionDenied(
            "/proc/stat".to_string(),
        ))]);

        assert!(matches!(
            sensor.sample(),
            Err(SensorError::PermissionDenied(_))
        ));
    }

    #[test]
    fn reads_proc_stat_from_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stat");
        std::fs::write(&path, PROC_STAT).unwrap();

        let parsed = ProcStatSource::new(&path).read().unwrap();

        assert_eq!(parsed, parse_proc_stat(PROC_STAT).unwrap());
    }

    #[test]
    fn a_missing_file_is_reported_as_unavailable() {
        let dir = tempfile::tempdir().unwrap();

        let error = ProcStatSource::new(dir.path().join("absent"))
            .read()
            .unwrap_err();

        assert!(matches!(error, SensorError::Unavailable(_)));
    }

    #[test]
    fn invalid_configuration_is_visible_as_a_sensor_error() {
        let mut sensor = InvalidBoardModelSensor::new("POWERWATCH_BOARD_MAX_W is not a number");

        let error = sensor.sample().unwrap_err();

        assert!(matches!(error, SensorError::Unavailable(message)
            if message.contains("POWERWATCH_BOARD_MAX_W")));
    }
}
