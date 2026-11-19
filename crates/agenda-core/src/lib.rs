//! Local appointments and a bounded, one-way import of single iCalendar events.

mod ical;
pub use ical::{parse_calendar, Parsed, IMPORT_LIMIT};

use chrono::{Days, NaiveDate, NaiveTime};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub id: u64,
    pub title: String,
    pub date: NaiveDate,
    pub time: Option<NaiveTime>,
    source_uid: Option<String>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Agenda {
    events: Vec<Event>,
}

impl Agenda {
    pub fn between(&self, start: NaiveDate, days: u64) -> Vec<&Event> {
        let end = start
            .checked_add_days(Days::new(days.saturating_sub(1)))
            .unwrap_or(NaiveDate::MAX);
        let mut found: Vec<_> = self
            .events
            .iter()
            .filter(|event| (start..=end).contains(&event.date))
            .collect();
        found.sort_by_key(|event| (event.date, event.time, event.id));
        found
    }

    pub fn event(&self, id: u64) -> Option<&Event> {
        self.events.iter().find(|event| event.id == id)
    }

    pub fn put(
        &mut self,
        editing: Option<u64>,
        title: &str,
        date: NaiveDate,
        time: Option<NaiveTime>,
    ) -> Result<(), &'static str> {
        let title = title.trim();
        if !valid_title(title) {
            return Err("Enter a title of 1 to 100 characters.");
        }
        if let Some(id) = editing {
            let event = self
                .events
                .iter_mut()
                .find(|event| event.id == id)
                .ok_or("Event no longer exists.")?;
            event.title = title.into();
            event.date = date;
            event.time = time;
        } else {
            self.push(Event {
                id: 0,
                title: title.into(),
                date,
                time,
                source_uid: None,
            })?;
        }
        Ok(())
    }

    pub fn remove(&mut self, id: u64) {
        self.events.retain(|event| event.id != id);
    }

    pub fn preview_import(&self, mut parsed: Parsed) -> Parsed {
        let mut ready = Vec::<Event>::new();
        for event in parsed.events.drain(..) {
            if self
                .events
                .iter()
                .chain(ready.iter())
                .any(|known| duplicate(known, &event))
            {
                parsed.duplicates += 1;
            } else {
                ready.push(event);
            }
        }
        parsed.events = ready;
        parsed
    }

    pub fn import(&mut self, batch: Parsed) -> Result<usize, &'static str> {
        let events = self.preview_import(batch).events;
        if self.events.len() + events.len() > 2000 {
            return Err("The event limit has been reached.");
        }
        let count = events.len();
        let mut id = self
            .max_id()
            .checked_add(count as u64)
            .ok_or("The event limit has been reached.")?
            - count as u64;
        self.events.extend(events.into_iter().map(|mut event| {
            id += 1;
            event.id = id;
            event
        }));
        Ok(count)
    }

    fn max_id(&self) -> u64 {
        self.events.iter().map(|event| event.id).max().unwrap_or(0)
    }

    fn push(&mut self, mut event: Event) -> Result<(), &'static str> {
        if self.events.len() >= 2000 {
            return Err("The event limit has been reached.");
        }
        event.id = self
            .max_id()
            .checked_add(1)
            .ok_or("The event limit has been reached.")?;
        self.events.push(event);
        Ok(())
    }
}

fn valid_title(title: &str) -> bool {
    !title.is_empty() && title.chars().count() <= 100 && !title.chars().any(char::is_control)
}

fn duplicate(first: &Event, second: &Event) -> bool {
    first.source_uid.is_some() && first.source_uid == second.source_uid
        || first.date == second.date && first.time == second.time && first.title == second.title
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    #[test]
    fn the_seven_day_window_sorts_all_day_first_and_excludes_the_eighth_day() {
        let mut agenda = Agenda::default();
        let today = day("2026-11-20");
        agenda
            .put(None, "Afternoon", today, Some("13:45:00".parse().unwrap()))
            .unwrap();
        agenda.put(None, "Trip", today, None).unwrap();
        agenda.put(None, "Later", day("2026-11-26"), None).unwrap();
        agenda
            .put(None, "Next week", day("2026-11-27"), None)
            .unwrap();
        let titles: Vec<_> = agenda
            .between(today, 7)
            .iter()
            .map(|event| event.title.as_str())
            .collect();
        assert_eq!(titles, ["Trip", "Afternoon", "Later"]);
    }

    #[test]
    fn repeated_import_and_changed_uid_never_overwrite_an_existing_event() {
        let file = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:a\nSUMMARY:Study\nDTSTART:20261120T090000\nEND:VEVENT\nEND:VCALENDAR";
        let batch = parse_calendar(file, |_| Some(0)).unwrap();
        let mut agenda = Agenda::default();
        assert_eq!(agenda.import(batch.clone()), Ok(1));
        assert_eq!(agenda.preview_import(batch.clone()).duplicates, 1);
        assert_eq!(agenda.import(batch), Ok(0));
        let changed = parse_calendar(&file.replace("09", "10"), |_| Some(0)).unwrap();
        assert_eq!(agenda.import(changed), Ok(0));
        assert_eq!(
            agenda.event(1).unwrap().time.unwrap().to_string(),
            "09:00:00"
        );
    }
}
