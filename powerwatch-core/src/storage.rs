use crate::model::{Component, Confidence, SensorReading};
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde::Serialize;

#[derive(Debug)]
pub enum StorageError {
    OpenFailed(String),
    QueryFailed(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct AggregatedReading {
    pub component: Component,
    pub confidence: Confidence,
    pub timestamp: DateTime<Utc>,
    pub avg_watts: f64,
    pub min_watts: f64,
    pub max_watts: f64,
    pub samples: u64,
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
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS readings (
                    component   TEXT NOT NULL,
                    watts       REAL NOT NULL,
                    confidence  TEXT NOT NULL,
                    ts          TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_readings_ts ON readings(ts);
                CREATE INDEX IF NOT EXISTS idx_readings_component_ts ON readings(component, ts);
                CREATE TABLE IF NOT EXISTS history_rollups (
                    component TEXT NOT NULL,
                    confidence TEXT NOT NULL,
                    bucket_seconds INTEGER NOT NULL,
                    bucket_epoch INTEGER NOT NULL,
                    avg_watts REAL NOT NULL,
                    min_watts REAL NOT NULL,
                    max_watts REAL NOT NULL,
                    samples INTEGER NOT NULL,
                    PRIMARY KEY (component, confidence, bucket_seconds, bucket_epoch)
                );
                CREATE INDEX IF NOT EXISTS idx_history_rollups_epoch ON history_rollups(bucket_epoch);",
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
            .prepare(
                "SELECT component, watts, confidence, ts
                 FROM readings
                 WHERE ts >= ?1
                 ORDER BY ts ASC",
            )
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

    pub fn aggregated_since(
        &self,
        since: DateTime<Utc>,
        bucket_seconds: u64,
    ) -> Result<Vec<AggregatedReading>, StorageError> {
        let bucket_seconds = bucket_seconds.max(1) as i64;
        let since_epoch = since.timestamp();
        let mut stmt = self.conn.prepare(
            "WITH source AS (
                SELECT component, confidence, CAST(strftime('%s', ts) AS INTEGER) AS sample_epoch,
                       watts AS avg_watts, watts AS min_watts, watts AS max_watts, 1 AS samples
                FROM readings WHERE ts >= ?2
                UNION ALL
                SELECT component, confidence, bucket_epoch, avg_watts, min_watts, max_watts, samples
                FROM history_rollups WHERE bucket_epoch >= ?3
             )
             SELECT component, confidence, (sample_epoch / ?1) * ?1,
                    SUM(avg_watts * samples) / SUM(samples), MIN(min_watts), MAX(max_watts), SUM(samples)
             FROM source
             GROUP BY component, confidence, (sample_epoch / ?1) * ?1
             ORDER BY (sample_epoch / ?1) * ?1 ASC, component ASC"
        ).map_err(|e| StorageError::QueryFailed(e.to_string()))?;
        let rows = stmt.query_map(rusqlite::params![bucket_seconds, since.to_rfc3339(), since_epoch], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, f64>(3)?, row.get::<_, f64>(4)?, row.get::<_, f64>(5)?, row.get::<_, i64>(6)?))
        }).map_err(|e| StorageError::QueryFailed(e.to_string()))?;
        let mut readings = Vec::new();
        for row in rows {
            let (component_json, confidence_text, bucket_epoch, avg_watts, min_watts, max_watts, samples) = row.map_err(|e| StorageError::QueryFailed(e.to_string()))?;
            let component: Component = serde_json::from_str(&component_json).map_err(|e| StorageError::QueryFailed(e.to_string()))?;
            let confidence = if confidence_text == "measured" { Confidence::Measured } else { Confidence::Estimated };
            let timestamp = DateTime::<Utc>::from_timestamp(bucket_epoch, 0).ok_or_else(|| StorageError::QueryFailed(format!("invalid bucket timestamp {bucket_epoch}")))?;
            readings.push(AggregatedReading { component, confidence, timestamp, avg_watts, min_watts, max_watts, samples: samples.max(0) as u64 });
        }
        Ok(readings)
    }

    pub fn compact_history(&mut self, now: DateTime<Utc>) -> Result<(), StorageError> {
        const Q15: i64 = 900;
        const H1: i64 = 3600;
        let raw_cutoff = (now - chrono::Duration::days(30)).to_rfc3339();
        let hourly_cutoff = (now - chrono::Duration::days(365)).timestamp();
        let tx = self.conn.transaction().map_err(|e| StorageError::QueryFailed(e.to_string()))?;
        tx.execute("INSERT OR REPLACE INTO history_rollups (component, confidence, bucket_seconds, bucket_epoch, avg_watts, min_watts, max_watts, samples)
                    SELECT component, confidence, ?1, (CAST(strftime('%s', ts) AS INTEGER) / ?1) * ?1, AVG(watts), MIN(watts), MAX(watts), COUNT(*)
                    FROM readings WHERE ts < ?2 GROUP BY component, confidence, (CAST(strftime('%s', ts) AS INTEGER) / ?1) * ?1",
                   rusqlite::params![Q15, raw_cutoff]).map_err(|e| StorageError::QueryFailed(e.to_string()))?;
        tx.execute("DELETE FROM readings WHERE ts < ?1", [&raw_cutoff]).map_err(|e| StorageError::QueryFailed(e.to_string()))?;
        tx.execute("INSERT OR REPLACE INTO history_rollups (component, confidence, bucket_seconds, bucket_epoch, avg_watts, min_watts, max_watts, samples)
                    SELECT component, confidence, ?1, (bucket_epoch / ?1) * ?1, SUM(avg_watts * samples) / SUM(samples), MIN(min_watts), MAX(max_watts), SUM(samples)
                    FROM history_rollups WHERE bucket_seconds = ?2 AND bucket_epoch < ?3 GROUP BY component, confidence, (bucket_epoch / ?1) * ?1",
                   rusqlite::params![H1, Q15, hourly_cutoff]).map_err(|e| StorageError::QueryFailed(e.to_string()))?;
        tx.execute("DELETE FROM history_rollups WHERE bucket_seconds = ?1 AND bucket_epoch < ?2", rusqlite::params![Q15, hourly_cutoff]).map_err(|e| StorageError::QueryFailed(e.to_string()))?;
        tx.commit().map_err(|e| StorageError::QueryFailed(e.to_string()))?;
        Ok(())
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

    #[test]
    fn aggregated_history_returns_average_min_max_and_count() {
        let storage = Storage::open_in_memory().unwrap();
        let base = Utc::now() - Duration::minutes(2);

        storage
            .insert_reading(&reading(
                Component::Total,
                10.0,
                Confidence::Estimated,
                base,
            ))
            .unwrap();
        storage
            .insert_reading(&reading(
                Component::Total,
                20.0,
                Confidence::Estimated,
                base + Duration::seconds(10),
            ))
            .unwrap();

        let points = storage
            .aggregated_since(base - Duration::seconds(1), 60)
            .unwrap();

        let total = points
            .iter()
            .find(|point| point.component == Component::Total)
            .unwrap();

        assert_eq!(total.samples, 2);
        assert_eq!(total.avg_watts, 15.0);
        assert_eq!(total.min_watts, 10.0);
        assert_eq!(total.max_watts, 20.0);
    }
}
