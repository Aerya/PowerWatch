use powerwatch_core::model::{Component, SensorReading};

#[derive(Debug, PartialEq)]
pub struct ComponentSummary {
    pub component: Component,
    pub average_watts: f64,
    pub sample_count: usize,
}

pub fn aggregate_by_component(readings: &[SensorReading]) -> Vec<ComponentSummary> {
    let mut summaries: Vec<(Component, f64, usize)> = Vec::new();

    for reading in readings {
        match summaries
            .iter_mut()
            .find(|(c, _, _)| *c == reading.component)
        {
            Some((_, sum, count)) => {
                *sum += reading.watts;
                *count += 1;
            }
            None => summaries.push((reading.component.clone(), reading.watts, 1)),
        }
    }

    summaries
        .into_iter()
        .map(|(component, sum, count)| ComponentSummary {
            component,
            average_watts: sum / count as f64,
            sample_count: count,
        })
        .collect()
}

fn component_label(component: &Component) -> String {
    component.label()
}

pub fn format_history_table(summaries: &[ComponentSummary]) -> String {
    if summaries.is_empty() {
        return "no readings recorded in that period".to_string();
    }

    summaries
        .iter()
        .map(|s| {
            format!(
                "  {:<14} {:>7.1} W avg  ({} samples)",
                component_label(&s.component),
                s.average_watts,
                s.sample_count
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use powerwatch_core::model::Confidence;

    fn reading(component: Component, watts: f64) -> SensorReading {
        SensorReading {
            component,
            watts,
            confidence: Confidence::Measured,
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn averages_watts_for_a_single_component() {
        let readings = vec![reading(Component::Cpu, 10.0), reading(Component::Cpu, 20.0)];

        let summaries = aggregate_by_component(&readings);

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].component, Component::Cpu);
        assert_eq!(summaries[0].average_watts, 15.0);
        assert_eq!(summaries[0].sample_count, 2);
    }

    #[test]
    fn keeps_different_components_separate() {
        let readings = vec![
            reading(Component::Cpu, 10.0),
            reading(Component::Ram, 4.0),
            reading(Component::Cpu, 30.0),
        ];

        let summaries = aggregate_by_component(&readings);

        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].component, Component::Cpu);
        assert_eq!(summaries[0].average_watts, 20.0);
        assert_eq!(summaries[1].component, Component::Ram);
        assert_eq!(summaries[1].average_watts, 4.0);
    }

    #[test]
    fn treats_disks_with_different_names_as_different_components() {
        let readings = vec![
            reading(Component::Disk("sda".to_string()), 4.0),
            reading(Component::Disk("nvme0n1".to_string()), 6.0),
        ];

        let summaries = aggregate_by_component(&readings);

        assert_eq!(summaries.len(), 2);
    }

    #[test]
    fn table_includes_the_average_and_sample_count() {
        let summaries = vec![ComponentSummary {
            component: Component::Cpu,
            average_watts: 12.345,
            sample_count: 42,
        }];

        let table = format_history_table(&summaries);

        assert!(table.contains("cpu"));
        assert!(table.contains("12.3 W"));
        assert!(table.contains("42 samples"));
    }

    #[test]
    fn reports_when_theres_nothing_to_show() {
        let table = format_history_table(&[]);

        assert!(table.contains("no readings"));
    }
}
