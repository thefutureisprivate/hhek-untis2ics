use crate::{
    Result,
    untis::{HOST, Name, TimetableResult},
};
use jiff::{Timestamp, civil::DateTime};
use std::collections::BTreeMap;

pub fn line(out: &mut String, value: &str) {
    let mut width = 0;
    for ch in value.chars() {
        if width + ch.len_utf8() > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(ch);
        width += ch.len_utf8();
    }
    out.push_str("\r\n");
}

fn text(value: &str) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\t'))
        .collect::<String>()
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace(';', "\\;")
        .replace(',', "\\,")
}

fn property(out: &mut String, name: &str, value: &str) {
    line(out, &format!("{name}:{}", text(value)));
}

fn instant(value: &str) -> Result<Timestamp> {
    // The mobile API's literal Z suffix labels school-local wall time, not UTC.
    // Interpret it in Berlin, then emit actual UTC (including DST transitions).
    let local: DateTime = value
        .strip_suffix('Z')
        .ok_or("missing mobile timestamp suffix")?
        .parse()?;
    Ok(local.in_tz("Europe/Berlin")?.timestamp())
}

pub fn render(data: &TimetableResult) -> Result<BTreeMap<i64, String>> {
    let mut events = BTreeMap::new();
    for period in &data.timetable.periods {
        let names = |kind: &str, source: &[Name], original: bool| {
            period
                .elements
                .iter()
                .filter(|e| e.kind == kind)
                .map(|e| {
                    let id = if original { e.org_id } else { e.id };
                    source
                        .iter()
                        .find(|n| n.id == id)
                        .map_or_else(|| format!("#{id}"), |n| n.name.clone())
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        let master = &data.master_data;
        let subject = names("SUBJECT", &master.subjects, false);
        let room = names("ROOM", &master.rooms, false);
        let has = |state: &str| period.states.iter().any(|s| s == state);
        let cancelled = has("CANCELLED");
        let exam = has("EXAM") || period.exam.is_some();
        let changed = has("IRREGULAR") || period.elements.iter().any(|e| e.id != e.org_id);
        let mut title = if subject.is_empty() {
            "Lesson".to_owned()
        } else {
            subject
        };
        let mut description = vec![
            format!("Teachers: {}", names("TEACHER", &master.teachers, false)),
            format!("Classes: {}", names("CLASS", &master.klassen, false)),
        ];
        if changed {
            title = format!("[CHANGED] {title}");
            description.push(format!(
                "Original teachers: {}",
                names("TEACHER", &master.teachers, true)
            ));
            description.push(format!(
                "Original rooms: {}",
                names("ROOM", &master.rooms, true)
            ));
            description.push(format!(
                "Original subjects: {}",
                names("SUBJECT", &master.subjects, true)
            ));
        }
        if exam {
            if let Some(exam) = &period.exam {
                for value in [&exam.examtype, &exam.name, &exam.text]
                    .into_iter()
                    .flatten()
                {
                    if !value.is_empty() {
                        description.push(value.clone());
                    }
                }
                if let Some(name) = &exam.name
                    && !name.is_empty()
                    && name != &title
                {
                    title.push_str(&format!(" — {name}"));
                }
            }
            title = format!("[EXAM] {title}");
        }
        for value in [
            &period.text.lesson,
            &period.text.substitution,
            &period.text.info,
        ] {
            if !value.is_empty() {
                description.push(value.clone());
            }
        }
        if cancelled {
            title = format!("[CANCELLED] {title}");
        }
        let start = instant(&period.start_date_time)?;
        let end = instant(&period.end_date_time)?;
        if end <= start {
            return Err("lesson end must follow its start".into());
        }
        let mut out = String::new();
        line(&mut out, &format!("UID:period-{}@{HOST}", period.id));
        line(
            &mut out,
            &format!("DTSTART:{}", start.strftime("%Y%m%dT%H%M%SZ")),
        );
        line(
            &mut out,
            &format!("DTEND:{}", end.strftime("%Y%m%dT%H%M%SZ")),
        );
        property(&mut out, "SUMMARY", &title);
        property(&mut out, "LOCATION", &room);
        property(&mut out, "DESCRIPTION", &description.join("\n"));
        line(
            &mut out,
            if cancelled {
                "STATUS:CANCELLED"
            } else {
                "STATUS:CONFIRMED"
            },
        );
        line(
            &mut out,
            if cancelled {
                "TRANSP:TRANSPARENT"
            } else {
                "TRANSP:OPAQUE"
            },
        );
        if exam {
            line(&mut out, "CATEGORIES:EXAM");
            line(&mut out, "PRIORITY:1");
            line(&mut out, "COLOR:crimson");
        }
        if events.insert(period.id, out).is_some() {
            return Err("duplicate WebUntis period ID".into());
        }
    }
    Ok(events)
}

#[derive(Default)]
pub struct Calendar {
    entries: BTreeMap<i64, (String, Timestamp)>,
    pub body: String,
    pub refreshed: Option<std::time::Instant>,
}

impl Calendar {
    pub fn update(&mut self, events: BTreeMap<i64, String>, now: Timestamp) -> Result<()> {
        let entries: BTreeMap<_, _> = events
            .into_iter()
            .map(|(id, content)| {
                let modified = self
                    .entries
                    .get(&id)
                    .filter(|(old, _)| old == &content)
                    .map_or(now, |(_, time)| *time);
                (id, (content, modified))
            })
            .collect();
        let mut body = String::from(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//HHEK//Untis2ICS//EN\r\nCALSCALE:GREGORIAN\r\nNAME:Heinrich-Hertz-BK\r\nX-WR-CALNAME:Heinrich-Hertz-BK\r\nREFRESH-INTERVAL;VALUE=DURATION:PT15M\r\n",
        );
        for (content, modified) in entries.values() {
            // A timestamp-derived revision remains monotonic across restarts.
            let revision = i32::try_from(modified.as_second() - 1_577_836_800)?;
            line(&mut body, "BEGIN:VEVENT");
            body.push_str(content);
            line(
                &mut body,
                &format!("DTSTAMP:{}", modified.strftime("%Y%m%dT%H%M%SZ")),
            );
            line(
                &mut body,
                &format!("LAST-MODIFIED:{}", modified.strftime("%Y%m%dT%H%M%SZ")),
            );
            line(&mut body, &format!("SEQUENCE:{revision}"));
            line(&mut body, "END:VEVENT");
        }
        line(&mut body, "END:VCALENDAR");
        self.entries = entries;
        self.body = body;
        self.refreshed = Some(std::time::Instant::now());
        Ok(())
    }

    pub fn ready(&self) -> bool {
        self.refreshed
            .is_some_and(|time| time.elapsed().as_secs() < 3600)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture(states: Value) -> TimetableResult {
        serde_json::from_value(json!({
            "timetable": {"periods": [{"id": 42,
                "startDateTime": "2026-11-17T13:15Z", "endDateTime": "2026-11-17T14:00Z",
                "is": states, "exam": {"examtype": "Klausur", "name": "Math", "text": "Chapters 1,2"},
                "text": {"lesson": "", "substitution": "Bring calculator", "info": ""},
                "elements": [{"type": "SUBJECT", "id": 1, "orgId": 1},
                    {"type": "ROOM", "id": 3, "orgId": 2}]}]},
            "masterData": {"subjects": [{"id": 1, "name": "MATH"}],
                "rooms": [{"id": 2, "name": "Old"}, {"id": 3, "name": "New"}], "teachers": [], "klassen": []}
        })).unwrap()
    }
    use serde_json::Value;

    #[test]
    fn subscription_snapshots_preserve_status_without_scheduling() {
        let mut cal = Calendar::default();
        let now = "2026-11-17T10:00:00Z".parse().unwrap();
        for events in [
            BTreeMap::new(),
            render(&fixture(json!(["REGULAR"]))).unwrap(),
            render(&fixture(json!(["CANCELLED", "EXAM"]))).unwrap(),
        ] {
            cal.update(events, now).unwrap();
            assert!(!cal.body.lines().any(|line| line.starts_with("METHOD:")));
            assert!(cal.body.starts_with("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n"));
            assert!(cal.body.ends_with("END:VCALENDAR\r\n"));
        }
        assert!(cal.body.contains("STATUS:CANCELLED\r\n"));
        assert!(cal.body.contains("CATEGORIES:EXAM\r\n"));
        assert!(cal.body.contains("UID:period-42@"));
    }

    #[test]
    fn cancellation_exam_and_substitution_keep_identity() {
        let regular = render(&fixture(json!(["REGULAR"]))).unwrap();
        let cancelled = render(&fixture(json!(["CANCELLED", "EXAM"]))).unwrap();
        let event = &cancelled[&42];
        assert!(event.contains("STATUS:CANCELLED\r\nTRANSP:TRANSPARENT"));
        assert!(event.contains("SUMMARY:[CANCELLED] [EXAM] [CHANGED] MATH"));
        assert!(event.contains("CATEGORIES:EXAM\r\nPRIORITY:1"));
        assert!(event.contains("LOCATION:New"));
        assert!(event.contains("Original rooms: Old"));
        assert!(event.contains("Bring calculator"));
        assert_eq!(regular[&42].lines().next(), event.lines().next());
    }

    #[test]
    fn timestamps_are_school_local_and_observe_dst() {
        assert_eq!(
            instant("2026-11-17T13:15Z").unwrap().to_string(),
            "2026-11-17T12:15:00Z"
        );
        assert_eq!(
            instant("2026-06-17T13:15:00Z").unwrap().to_string(),
            "2026-06-17T11:15:00Z"
        );
        assert!(instant("bad").is_err());
    }

    #[test]
    fn escapes_injection_and_folds_utf8_by_octet() {
        let value = "ä,;\\\r\nBEGIN:VEVENT\0".repeat(20);
        let mut out = String::new();
        property(&mut out, "SUMMARY", &value);
        assert!(out.split("\r\n").all(|l| l.len() <= 75));
        assert_eq!(
            out.replace("\r\n ", ""),
            format!("SUMMARY:{}\r\n", text(&value))
        );
        assert!(!out.contains("\r\nBEGIN:VEVENT"));
        assert!(!out.contains('\0'));
    }

    #[test]
    fn unchanged_revisions_stay_stable_and_failures_are_atomic() {
        let mut cal = Calendar::default();
        assert!(!cal.ready());
        let first = "2026-11-17T10:00:00Z".parse().unwrap();
        let later = "2026-11-17T10:15:00Z".parse().unwrap();
        cal.update(render(&fixture(json!(["REGULAR"]))).unwrap(), first)
            .unwrap();
        let body = cal.body.clone();
        cal.update(render(&fixture(json!(["REGULAR"]))).unwrap(), later)
            .unwrap();
        assert_eq!(body, cal.body);
        cal.update(render(&fixture(json!(["CANCELLED"]))).unwrap(), later)
            .unwrap();
        assert_ne!(body, cal.body);
        assert!(cal.body.contains("LAST-MODIFIED:20261117T101500Z"));
        assert!(cal.ready());
        cal.refreshed = Some(std::time::Instant::now() - std::time::Duration::from_secs(3601));
        assert!(!cal.ready());
    }

    #[test]
    fn malformed_periods_reject_the_entire_refresh() {
        let mut data = fixture(json!(["REGULAR"]));
        data.timetable.periods[0].end_date_time = "2026-11-17T12:00Z".into();
        assert!(render(&data).is_err());
    }

    #[test]
    fn exam_flag_without_details_and_duplicate_ids() {
        let mut data = fixture(json!(["EXAM"]));
        data.timetable.periods[0].exam = None;
        assert!(render(&data).unwrap()[&42].contains("CATEGORIES:EXAM"));
        data.timetable.periods[0].states = vec!["REGULAR".into()];
        assert!(!render(&data).unwrap()[&42].contains("CATEGORIES:EXAM"));
        data.timetable
            .periods
            .extend(fixture(json!(["REGULAR"])).timetable.periods);
        assert!(render(&data).is_err());
    }
}
