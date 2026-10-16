//! Independent, monotonic Hub MQTT energy meter (not the selectable Hub historical total).
//!
//! On initial install, it imports the recorded lifetime energy of the nodes in
//! the current total ONCE. Afterwards, it only adds newly observed intervals.
//! Offline gaps are not interpolated.
//! The existing node history and its selectable energy overview are untouched.
use rusqlite::{params, Connection, OptionalExtension, Result};
use std::collections::HashSet;

const MAX_SAMPLE_GAP_SECONDS: i64 = 180;

pub fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS ha_hub_energy_total (
            id INTEGER PRIMARY KEY CHECK(id=1), wh REAL NOT NULL CHECK(wh>=0)
        );
        INSERT OR IGNORE INTO ha_hub_energy_total(id,wh) VALUES(1,0);
        CREATE TABLE IF NOT EXISTS ha_hub_energy_last (
            node_id TEXT PRIMARY KEY, epoch INTEGER NOT NULL, watts REAL NOT NULL
        );
        CREATE TABLE IF NOT EXISTS ha_hub_energy_meta (
            key TEXT PRIMARY KEY, value TEXT NOT NULL
        );",
    )
}

/// Initialize a new Home Assistant meter from existing Hub lifetime history,
/// honoring the selection at upgrade time. This runs exactly once, before the
/// polling/history tasks start, so the old Wh cannot be added a second time.
///
/// If a previous MQTT meter has already accumulated energy, preserve it rather
/// than importing history on top of it (important for development upgrades).
/// Subsequent inclusion/exclusion changes affect only future energy intervals.
pub fn seed_from_history(
    conn: &mut Connection,
    included_nodes: &HashSet<String>,
    now_epoch: i64,
) -> Result<()> {
    let initialized: Option<String> = conn.query_row(
        "SELECT value FROM ha_hub_energy_meta WHERE key='initialized'",
        [], |row| row.get(0),
    ).optional()?;
    if initialized.is_some() { return Ok(()); }

    let existing_wh: f64 = conn.query_row(
        "SELECT wh FROM ha_hub_energy_total WHERE id=1", [], |row| row.get(0),
    )?;
    let has_cursor: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM ha_hub_energy_last)", [], |row| row.get(0),
    )?;
    let seed_wh = if existing_wh > 0.0 || has_cursor {
        // Already-running experimental MQTT meter: do NOT add previous history.
        existing_wh
    } else {
        let mut historical_wh = 0.0;
        for node in included_nodes {
            let stats = crate::energy::stats(conn, node, None, now_epoch)?;
            let wh = stats.energy_kwh * 1000.0;
            if wh.is_finite() && wh >= 0.0 { historical_wh += wh; }
        }
        if historical_wh.is_finite() && historical_wh >= 0.0 { historical_wh } else { 0.0 }
    };
    let tx = conn.transaction()?;
    tx.execute("UPDATE ha_hub_energy_total SET wh=?1 WHERE id=1", params![seed_wh])?;
    tx.execute(
        "INSERT INTO ha_hub_energy_meta(key,value) VALUES('initialized','1')", [],
    )?;
    tx.commit()
}

/// Called within the Hub history write transaction, on *every* tick, even when
/// no node is online. Removing stale cursors prevents overcounting downtime or
/// intervals during which nodes have been deliberately excluded from the total.
pub fn record(
    conn: &Connection,
    epoch: i64,
    active_totals: &[(String, f64)],
    included_nodes: &HashSet<String>,
) -> Result<()> {
    let current: Vec<(&str, f64)> = active_totals.iter()
        .filter(|(id, watts)| included_nodes.contains(id) && watts.is_finite() && *watts >= 0.0)
        .map(|(id, watts)| (id.as_str(), *watts))
        .collect();
    let active: HashSet<&str> = current.iter().map(|(id, _)| *id).collect();

    let old_ids = {
        let mut stmt = conn.prepare("SELECT node_id FROM ha_hub_energy_last")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>>>()?
    };
    for id in old_ids {
        if !active.contains(id.as_str()) {
            conn.execute("DELETE FROM ha_hub_energy_last WHERE node_id=?1", [&id])?;
        }
    }

    for (id, watts) in current {
        let previous: Option<(i64, f64)> = conn.query_row(
            "SELECT epoch,watts FROM ha_hub_energy_last WHERE node_id=?1", [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        if let Some((last_epoch, last_watts)) = previous {
            let dt = epoch - last_epoch;
            if dt > 0 && dt <= MAX_SAMPLE_GAP_SECONDS && last_watts.is_finite() && last_watts >= 0.0 {
                let added_wh = (watts + last_watts) / 2.0 * dt as f64 / 3600.0;
                if added_wh.is_finite() && added_wh >= 0.0 {
                    conn.execute("UPDATE ha_hub_energy_total SET wh=wh+?1 WHERE id=1",params![added_wh])?;
                }
            }
        }
        conn.execute(
            "INSERT INTO ha_hub_energy_last(node_id,epoch,watts) VALUES(?1,?2,?3)
             ON CONFLICT(node_id) DO UPDATE SET epoch=excluded.epoch,watts=excluded.watts
             WHERE excluded.epoch>ha_hub_energy_last.epoch",
            params![id, epoch, watts],
        )?;
    }
    Ok(())
}

pub fn total_kwh(conn: &Connection) -> Result<f64> {
    let wh: f64 = conn.query_row("SELECT wh FROM ha_hub_energy_total WHERE id=1",[],|r|r.get(0))?;
    Ok(wh / 1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn nodes(names: &[&str]) -> HashSet<String> { names.iter().map(|s|s.to_string()).collect() }
    fn watts(values: &[(&str,f64)]) -> Vec<(String,f64)> {
        values.iter().map(|(id,w)|(id.to_string(),*w)).collect()
    }
    #[test]
    fn excludes_without_retroactive_subtraction_and_reincludes_without_backfilling() {
        let c=Connection::open_in_memory().unwrap();init(&c).unwrap();
        let both=nodes(&["garuda","lab"]);
        let one=nodes(&["garuda"]);
        record(&c,100,&watts(&[("garuda",60.0),("lab",120.0)]),&both).unwrap();
        record(&c,160,&watts(&[("garuda",60.0),("lab",120.0)]),&both).unwrap();
        let first=total_kwh(&c).unwrap();
        assert!((first-3.0/1000.0).abs()<1e-10);
        record(&c,220,&watts(&[("garuda",60.0),("lab",120.0)]),&one).unwrap();
        let second=total_kwh(&c).unwrap();
        assert!((second-first-1.0/1000.0).abs()<1e-10);
        record(&c,280,&watts(&[("garuda",60.0),("lab",120.0)]),&both).unwrap();
        // Reintroduced node has a new baseline and no retroactive consumption.
        assert!((total_kwh(&c).unwrap()-second-1.0/1000.0).abs()<1e-10);
    }
    #[test]
    fn offline_restart_empty_tick_no_phantom_energy() {
        let c=Connection::open_in_memory().unwrap();init(&c).unwrap();let all=nodes(&["lab"]);
        record(&c,100,&watts(&[("lab",60.0)]),&all).unwrap();
        record(&c,160,&watts(&[("lab",60.0)]),&all).unwrap();
        record(&c,220,&[],&all).unwrap();
        record(&c,280,&watts(&[("lab",60.0)]),&all).unwrap();
        assert!((total_kwh(&c).unwrap()-1.0/1000.0).abs()<1e-10);
        record(&c,340,&watts(&[("lab",60.0)]),&all).unwrap();
        assert!((total_kwh(&c).unwrap()-2.0/1000.0).abs()<1e-10);
    }
    #[test]
    fn duplicate_and_long_gap_never_overcount() {
        let c=Connection::open_in_memory().unwrap();init(&c).unwrap();let all=nodes(&["a"]);
        record(&c,10,&watts(&[("a",120.0)]),&all).unwrap();
        record(&c,70,&watts(&[("a",120.0)]),&all).unwrap();
        let first=total_kwh(&c).unwrap();
        record(&c,70,&watts(&[("a",120.0)]),&all).unwrap();
        record(&c,1000,&watts(&[("a",120.0)]),&all).unwrap();
        assert!((total_kwh(&c).unwrap()-first).abs()<1e-10);
    }
    #[test]
    fn initial_seed_imports_selected_historical_kwh_once() {
        let mut c=Connection::open_in_memory().unwrap();
        crate::energy::init(&c).unwrap();
        init(&c).unwrap();
        crate::energy::record(&mut c,"garuda",100,60.0,true,180).unwrap();
        crate::energy::record(&mut c,"garuda",160,60.0,true,180).unwrap();
        crate::energy::record(&mut c,"lab",100,120.0,true,180).unwrap();
        crate::energy::record(&mut c,"lab",160,120.0,true,180).unwrap();
        seed_from_history(&mut c,&nodes(&["garuda","lab"]),200).unwrap();
        assert!((total_kwh(&c).unwrap()-0.003).abs()<1e-10);
        // A restart cannot import those three Wh again, regardless of selection.
        seed_from_history(&mut c,&nodes(&["garuda"]),200).unwrap();
        assert!((total_kwh(&c).unwrap()-0.003).abs()<1e-10);
        // Removing one machine changes only the new increments.
        record(&c,200,&watts(&[("garuda",60.0)]),&nodes(&["garuda"])).unwrap();
        record(&c,260,&watts(&[("garuda",60.0)]),&nodes(&["garuda"])).unwrap();
        assert!((total_kwh(&c).unwrap()-0.004).abs()<1e-10);
    }
    #[test]
    fn an_existing_mqtt_meter_is_not_seeded_twice() {
        let mut c=Connection::open_in_memory().unwrap();
        crate::energy::init(&c).unwrap();
        init(&c).unwrap();
        crate::energy::record(&mut c,"a",100,60.0,true,180).unwrap();
        crate::energy::record(&mut c,"a",160,60.0,true,180).unwrap();
        record(&c,100,&watts(&[("a",60.0)]),&nodes(&["a"])).unwrap();
        record(&c,160,&watts(&[("a",60.0)]),&nodes(&["a"])).unwrap();
        assert!((total_kwh(&c).unwrap()-0.001).abs()<1e-10);
        seed_from_history(&mut c,&nodes(&["a"]),200).unwrap();
        assert!((total_kwh(&c).unwrap()-0.001).abs()<1e-10);
    }
    #[test]
    fn meter_survives_reopen() {
        let temp=tempfile::NamedTempFile::new().unwrap();
        let first={let c=Connection::open(temp.path()).unwrap();init(&c).unwrap();
            let all=nodes(&["a"]);record(&c,100,&watts(&[("a",60.0)]),&all).unwrap();
            record(&c,160,&watts(&[("a",60.0)]),&all).unwrap();total_kwh(&c).unwrap()};
        let c=Connection::open(temp.path()).unwrap();init(&c).unwrap();
        assert!((total_kwh(&c).unwrap()-first).abs()<1e-10);
    }
}
