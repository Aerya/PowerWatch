use powerwatch_core::model::{SensorError, SensorReading};
use powerwatch_core::sampler::Snapshot;
use serde::Serialize;

#[derive(Serialize)]
struct JsonSensorResult {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reading: Option<SensorReading>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
struct JsonSnapshot {
    timestamp: chrono::DateTime<chrono::Utc>,
    sensors: Vec<JsonSensorResult>,
    total: Option<SensorReading>,
}

fn error_message(error: &SensorError) -> String {
    match error {
        SensorError::PermissionDenied(msg) => format!("permission denied: {msg}"),
        SensorError::Unavailable(msg) => format!("unavailable: {msg}"),
        SensorError::ReadFailed(msg) => format!("read failed: {msg}"),
        SensorError::NotReady(msg) => format!("not ready yet: {msg}"),
    }
}

pub fn format_snapshot_json(snapshot: &Snapshot) -> String {
    let sensors = snapshot
        .results
        .iter()
        .map(|(name, result)| match result {
            Ok(reading) => JsonSensorResult {
                name: name.clone(),
                reading: Some(reading.clone()),
                error: None,
            },
            Err(error) => JsonSensorResult {
                name: name.clone(),
                reading: None,
                error: Some(error_message(error)),
            },
        })
        .collect();

    let json_snapshot = JsonSnapshot {
        timestamp: snapshot.timestamp,
        sensors,
        total: snapshot.total(),
    };

    serde_json::to_string_pretty(&json_snapshot).unwrap_or_else(|e| {
        format!("{{\"error\": \"failed to serialize snapshot: {e}\"}}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use powerwatch_core::model::{Component, Confidence};
    use chrono::Utc;
    use serde_json::Value;

    fn reading(component: Component, watts: f64, confidence: Confidence) -> SensorReading {
        SensorReading {
            component,
            watts,
            confidence,
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn output_is_valid_json() {
        let snapshot = Snapshot {
            timestamp: Utc::now(),
            results: vec![("cpu".to_string(), Ok(reading(Component::Cpu, 12.0, Confidence::Measured)))],
        };

        let json = format_snapshot_json(&snapshot);

        let _: Value = serde_json::from_str(&json).expect("output should be valid JSON");
    }

    #[test]
    fn includes_watts_and_confidence_for_a_successful_reading() {
        let snapshot = Snapshot {
            timestamp: Utc::now(),
            results: vec![("cpu".to_string(), Ok(reading(Component::Cpu, 12.0, Confidence::Measured)))],
        };

        let json = format_snapshot_json(&snapshot);
        let parsed: Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["sensors"][0]["name"], "cpu");
        assert_eq!(parsed["sensors"][0]["reading"]["watts"], 12.0);
        assert_eq!(parsed["sensors"][0]["reading"]["confidence"], "Measured");
    }

    #[test]
    fn includes_an_error_message_for_a_failed_sensor() {
        let snapshot = Snapshot {
            timestamp: Utc::now(),
            results: vec![(
                "gpu".to_string(),
                Err(SensorError::Unavailable("no NVIDIA driver".to_string())),
            )],
        };

        let json = format_snapshot_json(&snapshot);
        let parsed: Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["sensors"][0]["name"], "gpu");
        assert!(parsed["sensors"][0]["error"]
            .as_str()
            .unwrap()
            .contains("no NVIDIA driver"));
        assert!(parsed["sensors"][0]["reading"].is_null());
    }

    #[test]
    fn includes_the_total_when_available() {
        let snapshot = Snapshot {
            timestamp: Utc::now(),
            results: vec![("cpu".to_string(), Ok(reading(Component::Cpu, 10.0, Confidence::Measured)))],
        };

        let json = format_snapshot_json(&snapshot);
        let parsed: Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["total"]["watts"], 10.0);
    }

    #[test]
    fn total_is_null_when_nothing_reported_successfully() {
        let snapshot = Snapshot {
            timestamp: Utc::now(),
            results: vec![(
                "cpu".to_string(),
                Err(SensorError::Unavailable("no RAPL".to_string())),
            )],
        };

        let json = format_snapshot_json(&snapshot);
        let parsed: Value = serde_json::from_str(&json).unwrap();

        assert!(parsed["total"].is_null());
    }
}
