use crate::model::{Component, Confidence, PowerSensor, SensorError, SensorReading};

pub fn parse_power_key(plist_text: &str, key: &str) -> Result<f64, SensorError> {
    let key_tag = format!("<key>{key}</key>");
    let key_pos = plist_text.find(&key_tag).ok_or_else(|| {
        SensorError::ReadFailed(format!("key '{key}' not found in powermetrics output"))
    })?;

    let after_key = &plist_text[key_pos + key_tag.len()..];

    let real_start = after_key.find("<real>");
    let integer_start = after_key.find("<integer>");

    let (tag_start, open_tag, close_tag) = match (real_start, integer_start) {
        (Some(r), Some(i)) if r < i => (r, "<real>", "</real>"),
        (Some(r), None) => (r, "<real>", "</real>"),
        (_, Some(i)) => (i, "<integer>", "</integer>"),
        _ => {
            return Err(SensorError::ReadFailed(format!(
                "no value found for key '{key}'"
            )))
        }
    };

    let value_start = tag_start + open_tag.len();
    let value_end = after_key[value_start..]
        .find(close_tag)
        .ok_or_else(|| SensorError::ReadFailed(format!("malformed value for key '{key}'")))?;

    after_key[value_start..value_start + value_end]
        .trim()
        .parse::<f64>()
        .map_err(|e| SensorError::ReadFailed(format!("invalid numeric value for key '{key}': {e}")))
}

pub fn milliwatts_to_watts(mw: f64) -> f64 {
    mw / 1000.0
}

pub trait PowerMetricsRunner: Send {
    fn sample_plist(&mut self, sampler: &str) -> Result<String, SensorError>;
}

pub struct MacPowerMetricsSensor<R: PowerMetricsRunner> {
    runner: R,
    sampler_name: &'static str,
    plist_key: &'static str,
    component: Component,
}

impl<R: PowerMetricsRunner> MacPowerMetricsSensor<R> {
    pub fn new(
        runner: R,
        sampler_name: &'static str,
        plist_key: &'static str,
        component: Component,
    ) -> Self {
        Self {
            runner,
            sampler_name,
            plist_key,
            component,
        }
    }
}

impl<R: PowerMetricsRunner> PowerSensor for MacPowerMetricsSensor<R> {
    fn sample(&mut self) -> Result<SensorReading, SensorError> {
        let plist = self.runner.sample_plist(self.sampler_name)?;
        let milliwatts = parse_power_key(&plist, self.plist_key)?;

        Ok(SensorReading {
            component: self.component.clone(),
            watts: milliwatts_to_watts(milliwatts),
            confidence: Confidence::Measured,
            timestamp: chrono::Utc::now(),
        })
    }
}

#[cfg(target_os = "macos")]
mod macos_impl {
    use super::PowerMetricsRunner;
    use crate::model::SensorError;
    use std::process::Command;

    pub struct RealPowerMetricsRunner;

    impl PowerMetricsRunner for RealPowerMetricsRunner {
        fn sample_plist(&mut self, sampler: &str) -> Result<String, SensorError> {
            let output = Command::new("powermetrics")
                .args(["-n1", "-i", "1000", "-f", "plist", "--samplers", sampler])
                .output()
                .map_err(|e| SensorError::ReadFailed(format!("failed to run powermetrics: {e}")))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
                if stderr.contains("root") || stderr.contains("permission") {
                    return Err(SensorError::PermissionDenied(
                        "powermetrics needs root - run powerwatch with sudo".to_string(),
                    ));
                }
                return Err(SensorError::ReadFailed(format!(
                    "powermetrics exited with an error: {stderr}"
                )));
            }

            String::from_utf8(output.stdout).map_err(|e| {
                SensorError::ReadFailed(format!("powermetrics output wasn't valid UTF-8: {e}"))
            })
        }
    }
}

#[cfg(target_os = "macos")]
pub use macos_impl::RealPowerMetricsRunner;

#[cfg(test)]
mod tests {
    use super::*;

    // a trimmed, realistic slice of what `powermetrics -n1 -f plist
    // --samplers cpu_power,gpu_power` actually outputs
    const SAMPLE_PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
	<key>cpu_energy</key>
	<integer>1923</integer>
	<key>cpu_power</key>
	<real>383.434</real>
	<key>gpu_energy</key>
	<integer>5807</integer>
	<key>gpu_power</key>
	<real>1157.88</real>
	<key>ane_energy</key>
	<integer>0</integer>
	<key>ane_power</key>
	<real>0</real>
	<key>combined_power</key>
	<real>1541.31</real>
</dict>
</plist>
"#;

    #[test]
    fn extracts_a_real_valued_key() {
        let value = parse_power_key(SAMPLE_PLIST, "cpu_power").unwrap();

        assert_eq!(value, 383.434);
    }

    #[test]
    fn extracts_a_different_key_from_the_same_document() {
        let value = parse_power_key(SAMPLE_PLIST, "gpu_power").unwrap();

        assert_eq!(value, 1157.88);
    }

    #[test]
    fn extracts_an_integer_valued_key_too() {
        let value = parse_power_key(SAMPLE_PLIST, "cpu_energy").unwrap();

        assert_eq!(value, 1923.0);
    }

    #[test]
    fn reports_a_clear_error_for_a_missing_key() {
        let result = parse_power_key(SAMPLE_PLIST, "does_not_exist");

        assert!(matches!(result.unwrap_err(), SensorError::ReadFailed(_)));
    }

    #[test]
    fn converts_milliwatts_to_watts() {
        assert!((milliwatts_to_watts(1157.88) - 1.15788).abs() < 0.000001);
    }

    struct FakeRunner {
        plist: String,
    }

    impl PowerMetricsRunner for FakeRunner {
        fn sample_plist(&mut self, _sampler: &str) -> Result<String, SensorError> {
            Ok(self.plist.clone())
        }
    }

    #[test]
    fn sensor_reports_a_measured_reading_from_the_configured_key() {
        let runner = FakeRunner {
            plist: SAMPLE_PLIST.to_string(),
        };
        let mut sensor =
            MacPowerMetricsSensor::new(runner, "cpu_power", "cpu_power", Component::Cpu);

        let reading = sensor.sample().unwrap();

        assert_eq!(reading.component, Component::Cpu);
        assert_eq!(reading.confidence, Confidence::Measured);
        assert!((reading.watts - 0.383434).abs() < 0.000001);
    }

    #[test]
    fn gpu_sensor_reads_the_gpu_key_from_the_same_kind_of_document() {
        let runner = FakeRunner {
            plist: SAMPLE_PLIST.to_string(),
        };
        let mut sensor = MacPowerMetricsSensor::new(
            runner,
            "gpu_power",
            "gpu_power",
            Component::Gpu(crate::model::GpuVendor::Apple),
        );

        let reading = sensor.sample().unwrap();

        assert_eq!(
            reading.component,
            Component::Gpu(crate::model::GpuVendor::Apple)
        );
        assert!((reading.watts - 1.15788).abs() < 0.000001);
    }

    struct FailingRunner;

    impl PowerMetricsRunner for FailingRunner {
        fn sample_plist(&mut self, _sampler: &str) -> Result<String, SensorError> {
            Err(SensorError::PermissionDenied("needs sudo".to_string()))
        }
    }

    #[test]
    fn propagates_runner_errors_without_panicking() {
        let mut sensor =
            MacPowerMetricsSensor::new(FailingRunner, "cpu_power", "cpu_power", Component::Cpu);

        let result = sensor.sample();

        assert!(matches!(
            result.unwrap_err(),
            SensorError::PermissionDenied(_)
        ));
    }
}
