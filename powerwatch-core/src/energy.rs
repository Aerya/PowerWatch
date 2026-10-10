//! Durable energy accounting for PowerWatch and its federated Hub.
//!
//! Power readings are Watts (instantaneous). Energy is integrated between successive
//! readings using trapezoids and stored in Wh. Long gaps are deliberately NOT imputed.
//! Recent intervals allow accurate rolling/custom ranges; older ones are compacted
//! into hourly buckets without losing all-time counters.
use rusqlite::{params, Connection, OptionalExtension, Result};
use serde::Serialize;

const COMPACT_AFTER_DAYS: i64 = 45;
const HOUR: i64 = 3600;

#[derive(Debug, Clone, Serialize, Default)]
pub struct EnergyStats {
    pub energy_kwh: f64,
    pub estimated_kwh: f64,
    pub coverage_seconds: f64,
    pub first_seen_epoch: Option<i64>,
    pub from_epoch: Option<i64>,
    pub to_epoch: i64,
}

pub fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS energy_last (
            source TEXT PRIMARY KEY, epoch INTEGER NOT NULL, watts REAL NOT NULL,
            estimated INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS energy_segments (
            source TEXT NOT NULL, start_epoch INTEGER NOT NULL, end_epoch INTEGER NOT NULL,
            wh REAL NOT NULL, estimated_wh REAL NOT NULL,
            PRIMARY KEY(source, end_epoch)
        );
        CREATE INDEX IF NOT EXISTS idx_energy_segments_range
            ON energy_segments(source,start_epoch,end_epoch);
        CREATE TABLE IF NOT EXISTS energy_hourly (
            source TEXT NOT NULL, hour_epoch INTEGER NOT NULL,
            wh REAL NOT NULL, estimated_wh REAL NOT NULL, covered_seconds REAL NOT NULL,
            PRIMARY KEY(source,hour_epoch)
        );
        CREATE TABLE IF NOT EXISTS energy_archive_cursor (
            source TEXT PRIMARY KEY, end_epoch INTEGER NOT NULL
        );",
    )
}

fn valid_watts(value: f64) -> bool { value.is_finite() && value >= 0.0 }

fn insert_segment(
    conn: &Connection, source: &str, start: i64, end: i64,
    left: f64, right: f64, estimated: bool, max_gap: i64,
) -> Result<()> {
    let delta = end - start;
    if delta <= 0 || delta > max_gap || !valid_watts(left) || !valid_watts(right) { return Ok(()); }
    let wh = (left + right) / 2.0 * delta as f64 / 3600.0;
    if !wh.is_finite() { return Ok(()); }
    conn.execute(
        "INSERT OR IGNORE INTO energy_segments(source,start_epoch,end_epoch,wh,estimated_wh)
         VALUES (?1,?2,?3,?4,?5)",
        params![source, start, end, wh, if estimated {wh} else {0.0}],
    )?;
    Ok(())
}

/// Idempotent historical import. Does not overwrite a newer live cursor.
/// Use only on timestamps that correspond to actual observations; interpolation
/// across large gaps or aggregated history is deliberately excluded.
pub fn import_samples(
    conn: &mut Connection, source: &str, samples: &[(i64, f64, bool)], max_gap: i64,
) -> Result<()> {
    let mut sorted = samples.to_vec();
    sorted.sort_by_key(|p| p.0);
    let tx = conn.transaction()?;
    // Re-importing old history after hourly compaction must not recreate
    // intervals which already contributed to the permanent Wh ledger.
    let archived_until: Option<i64> = tx.query_row(
        "SELECT end_epoch FROM energy_archive_cursor WHERE source=?1",
        [source], |r| r.get(0),
    ).optional()?;
    for pair in sorted.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if archived_until.is_some_and(|until| b.0 <= until) { continue; }
        insert_segment(&tx, source, a.0, b.0, a.1, b.1, a.2 || b.2, max_gap)?;
    }
    if let Some(&(epoch, watts, estimated)) = sorted.last() {
        if valid_watts(watts) {
            tx.execute(
                "INSERT INTO energy_last(source,epoch,watts,estimated) VALUES (?1,?2,?3,?4)
                 ON CONFLICT(source) DO UPDATE SET epoch=excluded.epoch,
                 watts=excluded.watts,estimated=excluded.estimated
                 WHERE excluded.epoch > energy_last.epoch",
                params![source, epoch, watts, i32::from(estimated)],
            )?;
        }
    }
    tx.commit()
}

/// A sample is recorded once, atomically with the previous cursor. Missing
/// samples, non-monotonic time, process downtime and invalid power are excluded.
pub fn record(
    conn: &mut Connection, source: &str, epoch: i64, watts: f64,
    estimated: bool, max_gap: i64,
) -> Result<()> {
    if !valid_watts(watts) { return Ok(()); }
    let tx = conn.transaction()?;
    let previous = tx.query_row(
        "SELECT epoch,watts,estimated FROM energy_last WHERE source=?1",
        [source], |row| Ok((row.get::<_,i64>(0)?,row.get::<_,f64>(1)?,row.get::<_,i32>(2)? != 0)),
    );
    match previous {
        Ok((last_epoch,last_watts,last_estimated)) if epoch > last_epoch => {
            insert_segment(&tx,source,last_epoch,epoch,last_watts,watts,last_estimated||estimated,max_gap)?;
        },
        Ok((last_epoch,_,_)) if epoch <= last_epoch => return tx.commit(),
        Err(rusqlite::Error::QueryReturnedNoRows) => {},
        Err(error) => return Err(error),
        _ => {},
    }
    tx.execute(
        "INSERT INTO energy_last(source,epoch,watts,estimated) VALUES (?1,?2,?3,?4)
         ON CONFLICT(source) DO UPDATE SET epoch=excluded.epoch,
         watts=excluded.watts,estimated=excluded.estimated",
        params![source,epoch,watts,i32::from(estimated)],
    )?;
    tx.commit()
}

pub fn has_samples(conn: &Connection, source: &str) -> Result<bool> {
    Ok(conn.query_row("SELECT COUNT(*) FROM energy_last WHERE source=?1", [source], |r| r.get::<_,i64>(0))? > 0)
}

/// Keep detailed intervals for 45 days; permanently retain older hourly Wh
/// and observed coverage. Re-running compaction is safe (transactional delete).
pub fn compact(conn: &mut Connection, now_epoch: i64) -> Result<()> {
    let cutoff = now_epoch - COMPACT_AFTER_DAYS * 86400;
    let tx = conn.transaction()?;
    let mut samples = Vec::new();
    {
        let mut stmt = tx.prepare(
            "SELECT source,start_epoch,end_epoch,wh,estimated_wh
             FROM energy_segments WHERE end_epoch<=?1 ORDER BY source,start_epoch",
        )?;
        let rows = stmt.query_map([cutoff], |r| Ok((
            r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,
            r.get::<_,f64>(3)?,r.get::<_,f64>(4)?,
        )))?;
        for row in rows { samples.push(row?); }
    }
    let mut archived_ends = std::collections::BTreeMap::<String, i64>::new();
    for (source,start,end,wh,estimated_wh) in samples {
        let seconds = (end-start) as f64;
        if seconds > 0.0 {
            archived_ends.entry(source.clone())
                .and_modify(|last| *last = (*last).max(end))
                .or_insert(end);
        }
        if seconds<=0.0 { continue; }
        let mut cursor = start;
        while cursor < end {
            let hour = cursor.div_euclid(HOUR)*HOUR;
            let piece_end = end.min(hour+HOUR);
            let duration = (piece_end-cursor) as f64;
            let factor = duration/seconds;
            tx.execute(
                "INSERT INTO energy_hourly(source,hour_epoch,wh,estimated_wh,covered_seconds)
                 VALUES (?1,?2,?3,?4,?5)
                 ON CONFLICT(source,hour_epoch) DO UPDATE SET
                 wh=energy_hourly.wh+excluded.wh,
                 estimated_wh=energy_hourly.estimated_wh+excluded.estimated_wh,
                 covered_seconds=energy_hourly.covered_seconds+excluded.covered_seconds",
                params![source,hour,wh*factor,estimated_wh*factor,duration],
            )?;
            cursor=piece_end;
        }
    }
    for (source, end_epoch) in archived_ends {
        tx.execute(
            "INSERT INTO energy_archive_cursor(source,end_epoch) VALUES (?1,?2)
             ON CONFLICT(source) DO UPDATE SET
             end_epoch=MAX(energy_archive_cursor.end_epoch,excluded.end_epoch)",
            params![source,end_epoch],
        )?;
    }
    tx.execute("DELETE FROM energy_segments WHERE end_epoch<=?1",[cutoff])?;
    tx.commit()
}

/// Sum energy within [from, to]. A missing 'from' means the first retained
/// reading. Hourly archives are proportional on partial boundary hours.
pub fn stats(conn: &Connection, source: &str, from: Option<i64>, to: i64) -> Result<EnergyStats> {
    let mut result=EnergyStats{from_epoch:from,to_epoch:to,..Default::default()};
    let floor=from.unwrap_or(i64::MIN/2);
    {
        let mut stmt=conn.prepare(
            "SELECT start_epoch,end_epoch,wh,estimated_wh FROM energy_segments
             WHERE source=?1 AND end_epoch>?2 AND start_epoch<?3")?;
        let rows=stmt.query_map(params![source,floor,to],|r| Ok((
            r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,f64>(2)?,r.get::<_,f64>(3)?,
        )))?;
        for row in rows {
            let (start,end,wh,estimated)=row?;
            if end<=start {continue;}
            let overlap=(end.min(to)-start.max(floor)).max(0) as f64;
            let frac=overlap/(end-start) as f64;
            result.energy_kwh+=wh*frac/1000.0;
            result.estimated_kwh+=estimated*frac/1000.0;
            result.coverage_seconds+=overlap;
            result.first_seen_epoch=Some(result.first_seen_epoch.map_or(start,|v|v.min(start)));
        }
    }
    {
        let mut stmt=conn.prepare(
            "SELECT hour_epoch,wh,estimated_wh,covered_seconds FROM energy_hourly
             WHERE source=?1 AND hour_epoch+3600>?2 AND hour_epoch<?3")?;
        let rows=stmt.query_map(params![source,floor,to],|r| Ok((
            r.get::<_,i64>(0)?,r.get::<_,f64>(1)?,r.get::<_,f64>(2)?,r.get::<_,f64>(3)?,
        )))?;
        for row in rows {
            let (hour,wh,estimated,coverage)=row?;
            let overlap=((hour+HOUR).min(to)-hour.max(floor)).max(0) as f64;
            let frac=overlap/HOUR as f64;
            result.energy_kwh+=wh*frac/1000.0;
            result.estimated_kwh+=estimated*frac/1000.0;
            result.coverage_seconds+=coverage*frac;
            result.first_seen_epoch=Some(result.first_seen_epoch.map_or(hour,|v|v.min(hour)));
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup()->Connection {let c=Connection::open_in_memory().unwrap();init(&c).unwrap();c}
    #[test]
    fn integrates_samples_and_retains_lifetime() {
        let mut c=setup();
        record(&mut c,"a",3600,10.0,true,180).unwrap();
        record(&mut c,"a",3660,30.0,true,180).unwrap();
        let s=stats(&c,"a",None,4000).unwrap();
        assert!((s.energy_kwh-20.0/60000.0).abs()<1e-10);
        assert_eq!(s.coverage_seconds,60.0);
        assert!((stats(&c,"a",Some(3630),4000).unwrap().energy_kwh-s.energy_kwh/2.0).abs()<1e-10);
        record(&mut c,"a",7200,30.0,true,180).unwrap();
        assert_eq!(stats(&c,"a",None,8000).unwrap().coverage_seconds,60.0);
    }
    #[test]
    fn rejects_duplicate_samples_and_compacts_without_changing_energy() {
        let mut c=setup();
        record(&mut c,"a",100,20.0,false,180).unwrap();
        record(&mut c,"a",160,20.0,false,180).unwrap();
        record(&mut c,"a",160,100.0,true,180).unwrap();
        let before=stats(&c,"a",None,3600).unwrap();
        compact(&mut c,46*86400+1000).unwrap();
        let after=stats(&c,"a",None,3600).unwrap();
        assert!((before.energy_kwh-after.energy_kwh).abs()<1e-10);
        assert_eq!(after.coverage_seconds,60.0);
        assert_eq!(after.estimated_kwh,0.0);
    }
    #[test]
    fn reimport_after_hourly_archiving_does_not_double_count() {
        let mut c = setup();
        let points = [(100, 20.0, true), (160, 20.0, true)];
        import_samples(&mut c, "host", &points, 180).unwrap();
        let before = stats(&c, "host", None, 3600).unwrap();
        compact(&mut c, 46 * 86400 + 1000).unwrap();
        import_samples(&mut c, "host", &points, 180).unwrap();
        let after = stats(&c, "host", None, 3600).unwrap();
        assert!((before.energy_kwh - after.energy_kwh).abs() < 1e-10);
        assert_eq!(before.coverage_seconds, after.coverage_seconds);
    }

    #[test]
    fn imports_are_idempotent() {
        let mut c=setup();
        let points=[(100,10.0,true),(160,10.0,true)];
        import_samples(&mut c,"host",&points,180).unwrap();
        import_samples(&mut c,"host",&points,180).unwrap();
        assert!((stats(&c,"host",None,1000).unwrap().energy_kwh-10.0/60000.0).abs()<1e-10);
    }
}
