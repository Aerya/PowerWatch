use powerwatch_core::alerts::{resolve_alert_rule, AlertRule};

pub fn resolve_alert_config(
    component: Option<&str>,
    above: Option<f64>,
    for_text: Option<&str>,
    run: Option<&str>,
    watch: bool,
) -> Result<Option<(AlertRule, Option<String>)>, String> {
    let resolved = resolve_alert_rule(component, above, for_text, run)?;

    if resolved.is_some() && !watch {
        return Err(
            "alerts require --watch (there's nothing to sustain in a single snapshot)".to_string(),
        );
    }

    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alert_above_without_watch_is_an_error() {
        let result = resolve_alert_config(None, Some(50.0), None, None, false);

        assert!(result.is_err());
    }

    #[test]
    fn alert_above_with_watch_is_fine() {
        let result = resolve_alert_config(None, Some(50.0), None, None, true);

        assert!(result.unwrap().is_some());
    }

    #[test]
    fn no_alert_flags_is_fine_regardless_of_watch() {
        assert!(resolve_alert_config(None, None, None, None, false)
            .unwrap()
            .is_none());
        assert!(resolve_alert_config(None, None, None, None, true)
            .unwrap()
            .is_none());
    }
}
