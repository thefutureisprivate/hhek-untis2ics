use jiff::Timestamp;
use std::time::Duration;

#[derive(Default)]
pub struct Metrics {
    successes: u64,
    failures: u64,
    last_refresh_success: bool,
    last_duration: Duration,
    last_success_timestamp: i64,
    events: usize,
}

impl Metrics {
    pub fn record(&mut self, count: Option<usize>, now: Timestamp, duration: Duration) {
        self.last_duration = duration;
        self.last_refresh_success = count.is_some();
        if let Some(count) = count {
            self.successes += 1;
            self.last_success_timestamp = now.as_second();
            self.events = count;
        } else {
            self.failures += 1;
        }
    }

    pub fn render(&self, ready: bool) -> String {
        // Fixed names and numeric values keep the exposition bounded and avoid
        // putting account details, feed tokens or timetable text in labels.
        format!(
            concat!(
                "# HELP hhek_untis2ics_ready Whether a calendar less than one hour old is available.\n",
                "# TYPE hhek_untis2ics_ready gauge\n",
                "hhek_untis2ics_ready {ready}\n",
                "# HELP hhek_untis2ics_refresh_successes_total Completed successful calendar refreshes.\n",
                "# TYPE hhek_untis2ics_refresh_successes_total counter\n",
                "hhek_untis2ics_refresh_successes_total {successes}\n",
                "# HELP hhek_untis2ics_refresh_failures_total Failed calendar refreshes.\n",
                "# TYPE hhek_untis2ics_refresh_failures_total counter\n",
                "hhek_untis2ics_refresh_failures_total {failures}\n",
                "# HELP hhek_untis2ics_last_refresh_success Whether the last completed refresh succeeded.\n",
                "# TYPE hhek_untis2ics_last_refresh_success gauge\n",
                "hhek_untis2ics_last_refresh_success {last_success}\n",
                "# HELP hhek_untis2ics_last_refresh_duration_seconds Duration of the last completed refresh.\n",
                "# TYPE hhek_untis2ics_last_refresh_duration_seconds gauge\n",
                "hhek_untis2ics_last_refresh_duration_seconds {duration}\n",
                "# HELP hhek_untis2ics_last_success_timestamp_seconds Unix time of the last successful refresh, or zero before success.\n",
                "# TYPE hhek_untis2ics_last_success_timestamp_seconds gauge\n",
                "hhek_untis2ics_last_success_timestamp_seconds {timestamp}\n",
                "# HELP hhek_untis2ics_calendar_events Events in the last successful calendar snapshot.\n",
                "# TYPE hhek_untis2ics_calendar_events gauge\n",
                "hhek_untis2ics_calendar_events {events}\n",
            ),
            ready = u8::from(ready),
            successes = self.successes,
            failures = self.failures,
            last_success = u8::from(self.last_refresh_success),
            duration = self.last_duration.as_secs_f64(),
            timestamp = self.last_success_timestamp,
            events = self.events,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_and_failed_refreshes_remain_scrapable() {
        let mut metrics = Metrics::default();
        let initial = metrics.render(false);
        assert!(initial.contains("hhek_untis2ics_ready 0\n"));
        assert!(initial.contains("hhek_untis2ics_last_success_timestamp_seconds 0\n"));
        metrics.record(None, Timestamp::now(), Duration::from_millis(250));
        let failed = metrics.render(false);
        assert!(failed.contains("hhek_untis2ics_refresh_failures_total 1\n"));
        assert!(failed.contains("hhek_untis2ics_last_refresh_duration_seconds 0.25\n"));
        assert!(failed.ends_with('\n'));
    }

    #[test]
    fn failure_preserves_success_timestamp_and_event_count() {
        let mut metrics = Metrics::default();
        let now = "2026-11-17T12:00:00Z".parse::<Timestamp>().unwrap();
        metrics.record(Some(170), now, Duration::from_millis(200));
        metrics.record(None, now, Duration::from_secs(30));
        let text = metrics.render(true);
        assert!(text.contains("hhek_untis2ics_refresh_successes_total 1\n"));
        assert!(text.contains("hhek_untis2ics_refresh_failures_total 1\n"));
        assert!(text.contains("hhek_untis2ics_last_refresh_success 0\n"));
        assert!(text.contains("hhek_untis2ics_ready 1\n"));
        assert!(text.contains("hhek_untis2ics_calendar_events 170\n"));
        assert!(text.contains(&format!(
            "hhek_untis2ics_last_success_timestamp_seconds {}\n",
            now.as_second()
        )));
        metrics.record(Some(0), now, Duration::ZERO);
        assert!(
            metrics
                .render(true)
                .contains("hhek_untis2ics_refresh_successes_total 2\n")
        );
        assert!(
            metrics
                .render(true)
                .contains("hhek_untis2ics_calendar_events 0\n")
        );
    }
}
