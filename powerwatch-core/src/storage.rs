use crate::model::{Component, Confidence, SensorReading};
use chrono::{DateTime, Utc};
use rusqlite::Connection;

#[derive(Debug)]
pub enum StorageError {
    OpenFailed(String),
    QueryFailed(String),
}

pub struct Storage {
    conn: Connection,
}

impl Storage {
    pub fn open(path: &std::path::Path) -> Result<Self, StorageError> {
        let conn = Connection::open(path).map_err(|e| StorageError::OpenFailed(e.to_string()))?;
        let storage = Self { conn };
        storage.create_schema()?;
        Ok(storage)
    }

    pub fn open_in_memory() -> Result<Self, StorageError> {
        let conn =
            Connection::open_in_memory().map_err(|e| StorageError::OpenFailed(e.to_string()))?;
        let storage = Self { conn };
        storage.create_schema()?;
        Ok(storage)
    }

    fn create_schema(&self) -> Result<(), StorageError> {
        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS readings (
                    component   TEXT NOT NULL,
                    watts       REAL NOT NULL,
                    confidence  TEXT NOT NULL,
                    ts          TEXT NOT NULL
                )",
                [],
            )
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?;
        Ok(())
    }

    pub fn insert_reading(&self, reading: &SensorReading) -> Result<(), StorageError> {
        let component_json = serde_json::to_string(&reading.component)
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?;
        let confidence = match reading.confidence {
            Confidence::Measured => "measured",
            Confidence::Estimated => "estimated",
        };

        self.conn
            .execute(
                "INSERT INTO readings (component, watts, confidence, ts) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    component_json,
                    reading.watts,
                    confidence,
                    reading.timestamp.to_rfc3339()
                ],
            )
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?;

        Ok(())
    }

    pub fn readings_since(&self, since: DateTime<Utc>) -> Result<Vec<SensorReading>, StorageError> {
        let mut stmt = self
            .conn
            .prepare("SELECT component, watts, confidence, ts FROM readings WHERE ts >= ?1 ORDER BY ts ASC")
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?;

        let rows = stmt
            .query_map([since.to_rfc3339()], |row| {
                let component_json: String = row.get(0)?;
                let watts: f64 = row.get(1)?;
                let confidence_text: String = row.get(2)?;
                let ts_text: String = row.get(3)?;
                Ok((component_json, watts, confidence_text, ts_text))
            })
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?;

        let mut readings = Vec::new();
        for row in rows {
            let (component_json, watts, confidence_text, ts_text) =
                row.map_err(|e| StorageError::QueryFailed(e.to_string()))?;

            let component: Component = serde_json::from_str(&component_json)
                .map_err(|e| StorageError::QueryFailed(e.to_string()))?;
            let confidence = if confidence_text == "measured" {
                Confidence::Measured
            } else {
                Confidence::Estimated
            };
            let timestamp = DateTime::parse_from_rfc3339(&ts_text)
                .map_err(|e| StorageError::QueryFailed(e.to_string()))?
                .with_timezone(&Utc);

            readings.push(SensorReading {
                component,
                watts,
                confidence,
                timestamp,
            });
        }

        Ok(readings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn reading(
        component: Component,
        watts: f64,
        confidence: Confidence,
        timestamp: DateTime<Utc>,
    ) -> SensorReading {
        SensorReading {
            component,
            watts,
            confidence,
            timestamp,
        }
    }

    #[test]
    fn a_stored_reading_can_be_read_back() {
        let storage = Storage::open_in_memory().unwrap();
        let now = Utc::now();
        storage
            .insert_reading(&reading(Component::Cpu, 12.5, Confidence::Measured, now))
            .unwrap();

        let readings = storage.readings_since(now - Duration::seconds(1)).unwrap();

        assert_eq!(readings.len(), 1);
        assert_eq!(readings[0].component, Component::Cpu);
        assert_eq!(readings[0].watts, 12.5);
        assert_eq!(readings[0].confidence, Confidence::Measured);
    }

    #[test]
    fn a_reading_with_data_carrying_component_round_trips_correctly() {
        let storage = Storage::open_in_memory().unwrap();
        let now = Utc::now();
        storage
            .insert_reading(&reading(
                Component::Disk("sda".to_string()),
                3.0,
                Confidence::Estimated,
                now,
            ))
            .unwrap();

        let readings = storage.readings_since(now - Duration::seconds(1)).unwrap();

        assert_eq!(readings[0].component, Component::Disk("sda".to_string()));
    }

    #[test]
    fn readings_before_the_cutoff_are_excluded() {
        let storage = Storage::open_in_memory().unwrap();
        let old = Utc::now() - Duration::hours(2);
        let recent = Utc::now();

        storage
            .insert_reading(&reading(Component::Cpu, 5.0, Confidence::Measured, old))
            .unwrap();
        storage
            .insert_reading(&reading(Component::Cpu, 8.0, Confidence::Measured, recent))
            .unwrap();

        let readings = storage
            .readings_since(recent - Duration::minutes(1))
            .unwrap();

        assert_eq!(readings.len(), 1);
        assert_eq!(readings[0].watts, 8.0);
    }

    #[test]
    fn readings_come_back_oldest_first() {
        let storage = Storage::open_in_memory().unwrap();
        let t0 = Utc::now() - Duration::seconds(10);
        let t1 = Utc::now();

        storage
            .insert_reading(&reading(Component::Cpu, 8.0, Confidence::Measured, t1))
            .unwrap();
        storage
            .insert_reading(&reading(Component::Cpu, 5.0, Confidence::Measured, t0))
            .unwrap();

        let readings = storage.readings_since(t0 - Duration::seconds(1)).unwrap();

        assert_eq!(readings[0].watts, 5.0);
        assert_eq!(readings[1].watts, 8.0);
    }
}
