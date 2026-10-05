use powerwatch_core::disk_topology::component_display_label;
use powerwatch_core::model::{Component, Confidence, SensorError, SensorReading};
use powerwatch_core::sampler::Snapshot;

fn component_label(component: &Component) -> String {
    component_display_label(component)
}

fn confidence_label(confidence: Confidence) -> &'static str {
    match confidence {
        Confidence::Measured => "measured",
        Confidence::Estimated => "estimated",
    }
}

fn error_summary(error: &SensorError) -> String {
    match error {
        SensorError::PermissionDenied(msg) => format!("permission denied: {msg}"),
        SensorError::Unavailable(msg) => format!("unavailable: {msg}"),
        SensorError::ReadFailed(msg) => format!("read failed: {msg}"),
        SensorError::NotReady(msg) => format!("not ready yet: {msg}"),
    }
}

fn reading_row(reading: &SensorReading) -> String {
    format!(
        "  {:<14} {:>7.1} W  ({})",
        component_label(&reading.component),
        reading.watts,
        confidence_label(reading.confidence)
    )
}

fn failed_row(name: &str, error: &SensorError) -> String {
    format!("  {:<14} {}", name, error_summary(error))
}

pub fn format_snapshot_table(snapshot: &Snapshot) -> String {
    let mut rows: Vec<String> = snapshot
        .results
        .iter()
        .map(|(name, result)| match result {
            Ok(reading) => reading_row(reading),
            Err(error) => failed_row(name, error),
        })
        .collect();

    if let Some(total) = snapshot.total() {
        rows.push("  ------------------------------".to_string());
        rows.push(reading_row(&total));
    }

    rows.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn reading(component: Component, watts: f64, confidence: Confidence) -> SensorReading {
        SensorReading {
            component,
            watts,
            confidence,
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn shows_watts_and_confidence_for_a_measured_reading() {
        let snapshot = Snapshot {
            timestamp: Utc::now(),
            results: vec![(
                "cpu".to_string(),
                Ok(reading(Component::Cpu, 12.345, Confidence::Measured)),
            )],
        };

        let table = format_snapshot_table(&snapshot);

        assert!(table.contains("cpu"));
        assert!(table.contains("12.3 W"));
        assert!(table.contains("(measured)"));
    }

    #[test]
    fn marks_estimated_readings_as_such() {
        let snapshot = Snapshot {
            timestamp: Utc::now(),
            results: vec![(
                "ram".to_string(),
                Ok(reading(Component::Ram, 4.0, Confidence::Estimated)),
            )],
        };

        let table = format_snapshot_table(&snapshot);

        assert!(table.contains("(estimated)"));
    }

    #[test]
    fn shows_why_a_sensor_failed_instead_of_a_wattage() {
        let snapshot = Snapshot {
            timestamp: Utc::now(),
            results: vec![(
                "gpu".to_string(),
                Err(SensorError::Unavailable("no NVIDIA driver".to_string())),
            )],
        };

        let table = format_snapshot_table(&snapshot);

        assert!(table.contains("gpu"));
        assert!(table.contains("unavailable"));
        assert!(table.contains("no NVIDIA driver"));
    }

    #[test]
    fn adds_a_total_row_when_something_reported_successfully() {
        let snapshot = Snapshot {
            timestamp: Utc::now(),
            results: vec![(
                "cpu".to_string(),
                Ok(reading(Component::Cpu, 10.0, Confidence::Measured)),
            )],
        };

        let table = format_snapshot_table(&snapshot);

        assert!(table.contains("total"));
        assert!(table.contains("10.0 W"));
    }

    #[test]
    fn skips_the_total_row_when_nothing_reported_successfully() {
        let snapshot = Snapshot {
            timestamp: Utc::now(),
            results: vec![(
                "cpu".to_string(),
                Err(SensorError::Unavailable("no RAPL".to_string())),
            )],
        };

        let table = format_snapshot_table(&snapshot);

        assert!(!table.contains("total"));
    }
}
