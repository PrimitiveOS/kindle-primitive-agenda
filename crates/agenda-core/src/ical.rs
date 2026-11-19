use chrono::{NaiveDate, NaiveDateTime, TimeDelta, Timelike};

use crate::{valid_title, Event};

pub const IMPORT_LIMIT: usize = 256 * 1024;
const MAX_EVENTS: usize = 128;

#[derive(Clone, Debug, Default)]
pub struct Parsed {
    pub events: Vec<Event>,
    pub duplicates: usize,
    pub unsupported: usize,
    pub invalid: usize,
}

#[derive(Default)]
struct Entry {
    title: Option<String>,
    uid: Option<String>,
    date: Option<NaiveDateTime>,
    all_day: bool,
    unsupported: bool,
    invalid: bool,
}

impl Entry {
    fn field(&mut self, key: &str, value: &str, offset: &impl Fn(i64) -> Option<i32>) {
        let (name, parameter) = key.split_once(';').unwrap_or((key, ""));
        match name.to_ascii_uppercase().as_str() {
            "RRULE" | "RDATE" | "EXDATE" | "EXRULE" | "RECURRENCE-ID" => self.unsupported = true,
            "STATUS" if value.eq_ignore_ascii_case("CANCELLED") => self.unsupported = true,
            "UID" if value.len() <= 256 && !value.is_empty() => self.uid = Some(value.to_owned()),
            "SUMMARY" => self.title = Some(text(value)),
            "DTSTART" => {
                if self.date.is_some() || !value.is_ascii() {
                    self.invalid = true;
                    return;
                }
                if !(parameter.is_empty()
                    || parameter.eq_ignore_ascii_case("VALUE=DATE")
                    || parameter.eq_ignore_ascii_case("VALUE=DATE-TIME"))
                {
                    self.unsupported = true;
                    return;
                }
                self.all_day = value.len() == 8;
                if parameter.eq_ignore_ascii_case("VALUE=DATE") != self.all_day
                    && !parameter.is_empty()
                {
                    self.invalid = true;
                    return;
                }
                self.date = if self.all_day {
                    NaiveDate::parse_from_str(value, "%Y%m%d")
                        .ok()
                        .and_then(|date| date.and_hms_opt(0, 0, 0))
                } else if let Some(utc) = value.strip_suffix('Z') {
                    NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S")
                        .ok()
                        .and_then(|time| {
                            let Some(zone_offset) = offset(time.and_utc().timestamp()) else {
                                self.unsupported = true;
                                return None;
                            };
                            time.checked_add_signed(TimeDelta::seconds(i64::from(zone_offset)))
                        })
                } else {
                    NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok()
                };
            }
            _ => {}
        }
    }

    fn finish(self, results: &mut Parsed) {
        if self.unsupported {
            results.unsupported += 1;
            return;
        }
        if self.invalid {
            results.invalid += 1;
            return;
        }
        let (Some(title), Some(start)) = (self.title, self.date) else {
            results.invalid += 1;
            return;
        };
        if !valid_title(&title) {
            results.invalid += 1;
            return;
        }
        results.events.push(Event {
            id: 0,
            title,
            date: start.date(),
            time: if self.all_day {
                None
            } else {
                start.time().with_second(0)
            },
            source_uid: self.uid,
        });
    }
}

/// Import uses the event start. Supports all-day, local and UTC values. Recurrence and named time
/// zones are counted as unsupported so their appointments cannot silently land on the wrong day.
pub fn parse_calendar(
    input: &str,
    offset: impl Fn(i64) -> Option<i32>,
) -> Result<Parsed, &'static str> {
    if input.len() > IMPORT_LIMIT {
        return Err("Calendar files can be at most 256 KB.");
    }
    let mut lines = Vec::<String>::new();
    for line in input.trim_start_matches('\u{feff}').lines() {
        if let Some(suffix) = line.strip_prefix([' ', '\t']) {
            let previous = lines.last_mut().ok_or("Invalid calendar file.")?;
            previous.push_str(suffix);
        } else if !line.is_empty() {
            lines.push(line.to_owned());
        }
    }
    let mut results = Parsed::default();
    let mut stack = Vec::<String>::new();
    let mut entry = None;
    let mut finished = false;
    let mut count = 0;
    for line in &lines {
        let (key, value) = line.split_once(':').ok_or("Invalid calendar file.")?;
        let value = value.trim();
        if finished {
            return Err("Choose a file containing one calendar.");
        }
        if key.eq_ignore_ascii_case("BEGIN") {
            let kind = value.to_ascii_uppercase();
            if stack.is_empty() && kind != "VCALENDAR" {
                return Err("Invalid calendar file.");
            }
            if kind == "VEVENT" {
                if stack.len() != 1 || stack[0] != "VCALENDAR" {
                    return Err("Invalid calendar event.");
                }
                count += 1;
                if count > MAX_EVENTS {
                    return Err("The calendar file contains more than 128 events.");
                }
                entry = Some(Entry::default());
            }
            if stack.len() >= 8 {
                return Err("The calendar file is nested too deeply.");
            }
            stack.push(kind);
        } else if key.eq_ignore_ascii_case("END") {
            let kind = stack.pop().ok_or("Invalid calendar file.")?;
            if !kind.eq_ignore_ascii_case(value) {
                return Err("Invalid calendar file.");
            }
            if kind == "VEVENT" {
                entry
                    .take()
                    .ok_or("Invalid calendar event.")?
                    .finish(&mut results);
            }
            finished = stack.is_empty();
        } else if stack.is_empty() {
            return Err("Invalid calendar file.");
        } else if stack.len() == 2 && stack[1] == "VEVENT" {
            if let Some(entry) = &mut entry {
                entry.field(key.trim(), value, &offset);
            }
        } else if stack.len() == 1
            && key.eq_ignore_ascii_case("METHOD")
            && value.eq_ignore_ascii_case("CANCEL")
        {
            return Err("Calendar cancellation files are not supported.");
        }
    }
    if !finished {
        return Err("The calendar file is incomplete.");
    }
    Ok(results)
}

fn text(input: &str) -> String {
    let mut output = String::new();
    let mut chars = input.chars();
    while let Some(character) = chars.next() {
        let character = if character == '\\' {
            match chars.next() {
                Some('n' | 'N') => ' ',
                Some(next) => next,
                None => '\\',
            }
        } else {
            character
        };
        output.push(character);
    }
    output.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folded_text_and_utc_events_keep_the_correct_local_day() {
        let source = concat!(
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nDTSTART:20261119T213000Z\r\n",
            "SUMMARY:Read\\, take\r\n notes\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
        );
        let report = parse_calendar(source, |_| Some(7 * 3600)).unwrap();
        assert_eq!(report.events[0].title, "Read, takenotes");
        assert_eq!(report.events[0].date.to_string(), "2026-11-20");
        assert_eq!(report.events[0].time.unwrap().to_string(), "04:30:00");
        assert_eq!(parse_calendar(source, |_| None).unwrap().unsupported, 1);
    }

    #[test]
    fn exclusions_are_visible_and_the_valid_event_can_still_be_imported() {
        let source = concat!(
            "BEGIN:VCALENDAR\n",
            "BEGIN:VEVENT\nDTSTART;TZID=America/Toronto:20261120T121500\nSUMMARY:Zoned\nEND:VEVENT\n",
            "BEGIN:VEVENT\nDTSTART;VALUE=DATE:20261120\nSUMMARY:Day\nEND:VEVENT\n",
            "BEGIN:VEVENT\nDTSTART:20261120T121500\nRRULE:FREQ=WEEKLY\nSUMMARY:Routine\nEND:VEVENT\n",
            "BEGIN:VEVENT\nDTSTART:invalid\nSUMMARY:Bad\nEND:VEVENT\nEND:VCALENDAR"
        );
        let report = parse_calendar(source, |_| Some(0)).unwrap();
        assert_eq!(report.events.len(), 1);
        assert_eq!(report.events[0].time, None);
        assert_eq!(report.invalid, 1);
        assert_eq!(report.unsupported, 2);
        assert!(parse_calendar("BEGIN:VCALENDAR", |_| Some(0)).is_err());
    }
}
