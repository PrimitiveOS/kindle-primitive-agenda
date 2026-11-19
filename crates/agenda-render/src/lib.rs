mod common;
mod date_picker;

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use agenda_core::{parse_calendar, Agenda, Event, Parsed, IMPORT_LIMIT};
use chrono::{NaiveDate, NaiveTime};
use primitive_app::{with_storage, App, Change, WIDTH};
use primitive_ui::{
    back_button, header, Canvas, Editor, Input, Press, Rect, Sheet, SheetAction, TextStyle, CAPTION,
};

use common::{
    editor, field, local_day, page_buttons, page_count, page_step, page_tap, press_key,
    read_bounded, rect, row, timezone_offset, State, PAGE_SIZE,
};
use date_picker::{DatePicker, Pick};

const TODAY: Rect = Rect::new(40, 174, 256, 104);
const WEEK: Rect = Rect::new(314, 174, 375, 104);
const ADD: Rect = Rect::new(878, 174, 346, 104);
const PICK_DAY: Rect = Rect::new(561, 304, 300, 104);
const IMPORT: Rect = Rect::new(878, 304, 346, 104);
const TITLE: Rect = Rect::new(40, 175, 1184, 104);
const DATE: Rect = Rect::new(40, 300, 1184, 106);
const TIME: Rect = Rect::new(40, 426, 819, 106);
const ALLDAY: Rect = Rect::new(878, 426, 346, 106);
const SAVE: Rect = Rect::new(836, 560, 388, 104);
const REMOVE: Rect = Rect::new(40, 560, 290, 104);
const EDIT: Rect = Rect::new(878, 500, 346, 104);
const IMPORT_ACTION: Rect = Rect::new(826, 1408, 398, 104);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Typing {
    None,
    Title,
    Time,
}

struct Form {
    id: Option<u64>,
    title: Editor,
    date: NaiveDate,
    time: Option<Editor>,
    typing: Typing,
}

enum Import {
    Files { paths: Vec<PathBuf>, page: usize },
    Review { name: String, batch: Parsed },
}

pub struct View {
    state: State<Agenda>,
    today: NaiveDate,
    live: bool,
    start: NaiveDate,
    week: bool,
    page: usize,
    detail: Option<u64>,
    form: Option<Form>,
    date: Option<DatePicker>,
    confirming: bool,
    import: Option<Import>,
    documents: Option<PathBuf>,
    notice: String,
}

impl View {
    pub fn open(path: PathBuf) -> Self {
        let documents = path.parent().map(|dir| dir.join("documents"));
        Self::new(
            State::open(path.join("agenda.json")),
            local_day(),
            documents,
            true,
        )
    }

    pub fn preview(today: NaiveDate, agenda: Agenda, documents: Option<PathBuf>) -> Self {
        Self::new(State::preview(agenda), today, documents, false)
    }

    fn new(state: State<Agenda>, today: NaiveDate, documents: Option<PathBuf>, live: bool) -> Self {
        Self {
            state,
            today,
            live,
            start: today,
            week: false,
            page: 0,
            detail: None,
            form: None,
            date: None,
            confirming: false,
            import: None,
            documents,
            notice: String::new(),
        }
    }

    fn notice(&self) -> &str {
        self.state.error.as_deref().unwrap_or(&self.notice)
    }

    fn events(&self) -> Vec<&Event> {
        self.state
            .data
            .between(self.start, if self.week { 7 } else { 1 })
    }

    fn reset_view(&mut self, week: bool) -> Change {
        self.start = self.today;
        self.week = week;
        self.page = 0;
        self.notice.clear();
        Change::Full
    }

    fn start_form(&mut self, id: Option<u64>) -> Change {
        let (title, date, time) = id
            .and_then(|id| self.state.data.event(id))
            .map_or(("", self.start, None), |event| {
                (event.title.as_str(), event.date, event.time)
            });
        self.form = Some(Form {
            id,
            title: editor(title, 100),
            date,
            time: time.map(|time| editor(&time.format("%H:%M").to_string(), 5)),
            typing: Typing::Title,
        });
        self.notice.clear();
        Change::Full
    }

    fn write(&mut self, next: Agenda, message: &str) -> bool {
        match self.state.replace(next) {
            Ok(()) => {
                self.notice = message.into();
                self.page = self.page.min(page_count(self.events().len()) - 1);
                true
            }
            Err(error) => {
                self.notice = format!("Could not save. {error}");
                false
            }
        }
    }

    fn save_form(&mut self) -> Change {
        let Some(form) = &self.form else {
            return Change::None;
        };
        let time = if let Some(field) = &form.time {
            let Some(time) = parse_time(&field.string()) else {
                self.notice = "Enter a time such as 09:30, or choose All day.".into();
                return Change::Partial;
            };
            Some(time)
        } else {
            None
        };
        let mut next = self.state.data.clone();
        let new_date = form.date;
        match next.put(form.id, &form.title.string(), new_date, time) {
            Err(error) => {
                self.notice = error.into();
                Change::Partial
            }
            Ok(()) if self.write(next, "Event saved.") => {
                self.form = None;
                self.detail = None;
                if !self.events().iter().any(|event| event.date == new_date) {
                    self.start = new_date;
                    self.week = false;
                }
                self.page = 0;
                Change::Full
            }
            Ok(()) => Change::Partial,
        }
    }

    fn scan_files(&mut self) -> Change {
        self.notice.clear();
        let paths = self.documents.as_ref().map_or_else(
            || Ok(Vec::new()),
            |directory| with_storage(|| calendar_paths(directory)),
        );
        match paths {
            Ok(paths) => {
                self.import = Some(Import::Files { paths, page: 0 });
                Change::Full
            }
            Err(error) => {
                self.notice = format!("Could not open Documents. {error}");
                Change::Partial
            }
        }
    }

    fn preview_file(&mut self, path: PathBuf) -> Change {
        let result = with_storage(|| {
            if !fs::symlink_metadata(&path)?.file_type().is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Choose a regular calendar file.",
                ));
            }
            let bytes = read_bounded(&path, IMPORT_LIMIT)?;
            let text = std::str::from_utf8(&bytes).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "Use a UTF-8 calendar file.")
            })?;
            parse_calendar(text, timezone_offset)
                .map_err(|message| io::Error::new(io::ErrorKind::InvalidData, message))
        });
        match result {
            Ok(parsed) => {
                let batch = self.state.data.preview_import(parsed);
                let name = path.file_name().map_or("Calendar".into(), |name| {
                    name.to_string_lossy().into_owned()
                });
                self.import = Some(Import::Review { name, batch });
                self.notice.clear();
                Change::Full
            }
            Err(error) => {
                self.notice = format!("Could not read calendar. {error}");
                Change::Partial
            }
        }
    }

    fn import_file(&mut self) -> Change {
        let Some(Import::Review { batch, .. }) = &self.import else {
            return Change::None;
        };
        if batch.events.is_empty() {
            return Change::None;
        }
        let mut next = self.state.data.clone();
        let first = batch.events.iter().map(|event| event.date).min();
        match next.import(batch.clone()) {
            Ok(count) if self.write(next, &format!("Imported {count} events.")) => {
                self.import = None;
                self.page = 0;
                if let Some(date) = first {
                    self.start = date;
                    self.week = true;
                }
                Change::Full
            }
            Ok(_) => Change::Partial,
            Err(error) => {
                self.notice = error.into();
                Change::Partial
            }
        }
    }

    fn draw_home(&self, canvas: &mut Canvas) {
        header(canvas, "Agenda", "Done");
        canvas.button(
            TODAY,
            "Today",
            !self.week && self.start == self.today,
            false,
        );
        canvas.button(
            WEEK,
            "Next 7 days",
            self.week && self.start == self.today,
            false,
        );
        canvas.button(ADD, "Add event", true, false);
        canvas.text(
            rect(40, 290, 503, 124),
            &self.start.format("%A, %-d %B").to_string(),
            TextStyle::new(39).bold(),
        );
        canvas.button(PICK_DAY, "Choose date", false, false);
        canvas.button(IMPORT, "Import .ics", false, false);
        let events = self.events();
        let mut previous = None;
        for (index, event) in events
            .iter()
            .skip(self.page * PAGE_SIZE)
            .take(PAGE_SIZE)
            .enumerate()
        {
            let subtitle = if previous == Some(event.date) {
                time_text(event)
            } else {
                format!("{} · {}", event.date.format("%a %-d %b"), time_text(event))
            };
            previous = Some(event.date);
            row(canvas, browse_row(index), &event.title, &subtitle);
        }
        if events.is_empty() {
            canvas.text(
                rect(40, 454, 1184, 105),
                "No appointments in this view.",
                CAPTION,
            );
        }
        canvas.text(rect(40, 1303, 1184, 127), self.notice(), CAPTION);
        page_buttons(canvas, self.page, events.len());
    }

    fn draw_detail(&self, canvas: &mut Canvas, event: &Event) {
        header(canvas, "Event", "Back");
        canvas.text(
            rect(40, 170, 1184, 146),
            &event.title,
            TextStyle::new(47).bold(),
        );
        canvas.text(
            rect(40, 330, 1184, 96),
            &event.date.format("%A, %-d %B %Y").to_string(),
            TextStyle::new(39),
        );
        canvas.text(rect(40, 430, 1184, 90), &time_text(event), CAPTION);
        canvas.button(EDIT, "Edit event", true, false);
    }

    fn draw_form(&self, canvas: &mut Canvas, form: &Form) {
        header(
            canvas,
            if form.id.is_some() {
                "Edit event"
            } else {
                "Add event"
            },
            "Back",
        );
        field(
            canvas,
            TITLE,
            "Event title",
            &form.title,
            form.typing == Typing::Title,
        );
        row(
            canvas,
            DATE,
            "Date",
            &form.date.format("%-d %B %Y").to_string(),
        );
        if let Some(time) = &form.time {
            field(
                canvas,
                TIME,
                "HHMM (24-hour time)",
                time,
                form.typing == Typing::Time,
            );
        } else {
            row(canvas, TIME, "Time", "All day");
        }
        canvas.button(
            ALLDAY,
            if form.time.is_some() {
                "All day"
            } else {
                "Set time"
            },
            false,
            false,
        );
        canvas.button(SAVE, "Save event", true, false);
        if form.id.is_some() {
            canvas.button(REMOVE, "Remove", false, false);
        }
        canvas.text(rect(40, 697, 1184, 152), self.notice(), CAPTION);
        if !self.confirming {
            let active = match form.typing {
                Typing::Title => Some(&form.title),
                Typing::Time => form.time.as_ref(),
                Typing::None => None,
            };
            if let Some(active) = active {
                canvas.keyboard(active.layer, "Done", None);
            }
        }
    }

    fn draw_import(&self, canvas: &mut Canvas, import: &Import) {
        match import {
            Import::Files { paths, page } => {
                header(canvas, "Calendar files", "Back");
                canvas.text(
                    rect(40, 162, 1184, 102),
                    "Copy an .ics file into Documents over USB.",
                    CAPTION,
                );
                if paths.is_empty() {
                    canvas.text(
                        rect(40, 300, 1184, 104),
                        "No .ics files in Documents.",
                        CAPTION,
                    );
                }
                for (i, path) in paths
                    .iter()
                    .skip(page * PAGE_SIZE)
                    .take(PAGE_SIZE)
                    .enumerate()
                {
                    let title = path.file_name().unwrap_or_default().to_string_lossy();
                    row(canvas, file_row(i), title.as_ref(), "");
                }
                canvas.text(rect(40, 1197, 1184, 129), self.notice(), CAPTION);
                page_buttons(canvas, *page, paths.len());
            }
            Import::Review { name, batch } => {
                header(canvas, "Review import", "Back");
                canvas.text(rect(40, 170, 1184, 99), name, TextStyle::new(44).bold());
                let counts = format!(
                    "{} ready · {} duplicates · {} unsupported · {} invalid",
                    batch.events.len(),
                    batch.duplicates,
                    batch.unsupported,
                    batch.invalid
                );
                canvas.text(rect(40, 268, 1184, 81), &counts, CAPTION);
                canvas.text(
                    rect(40, 353, 1184, 142),
                    "Uses each event's start. Repeats and named time zones are skipped.",
                    CAPTION,
                );
                for (index, event) in batch.events.iter().take(5).enumerate() {
                    row(
                        canvas,
                        rect(40, 530 + index as i32 * 121, 1184, 108),
                        &event.title,
                        &format!("{} · {}", event.date.format("%a %-d %b"), time_text(event)),
                    );
                }
                if batch.events.len() > 5 {
                    canvas.text(
                        rect(40, 1145, 1184, 66),
                        &format!("Plus {} more events.", batch.events.len() - 5),
                        CAPTION,
                    );
                }
                if !batch.events.is_empty() {
                    canvas.button(IMPORT_ACTION, "Import events", true, false);
                }
                canvas.text(rect(40, 1228, 1184, 129), self.notice(), CAPTION);
            }
        }
    }

    fn tap_home(&mut self, x: f32, y: f32) -> Change {
        if back_button(WIDTH as i32).contains(x, y) {
            return Change::Exit;
        }
        if TODAY.contains(x, y) {
            return self.reset_view(false);
        }
        if WEEK.contains(x, y) {
            return self.reset_view(true);
        }
        if ADD.contains(x, y) {
            return self.start_form(None);
        }
        if IMPORT.contains(x, y) {
            return self.scan_files();
        }
        if PICK_DAY.contains(x, y) {
            self.date = Some(DatePicker::new(self.today, Some(self.start), false));
            return Change::Full;
        }
        let count = self.events().len();
        if page_tap(&mut self.page, count, x, y) {
            return Change::Full;
        }
        for (index, event) in self
            .events()
            .iter()
            .skip(self.page * PAGE_SIZE)
            .take(PAGE_SIZE)
            .enumerate()
        {
            if browse_row(index).contains(x, y) {
                self.detail = Some(event.id);
                self.notice.clear();
                return Change::Full;
            }
        }
        Change::None
    }

    fn tap_detail(&mut self, x: f32, y: f32) -> Change {
        if EDIT.contains(x, y) {
            return self.start_form(self.detail);
        }
        if back_button(WIDTH as i32).contains(x, y) {
            self.detail = None;
            return Change::Full;
        }
        Change::None
    }

    fn tap_form(&mut self, x: f32, y: f32) -> Change {
        if self.confirming {
            let Some(action) = remove_sheet().hit(x, y) else {
                return Change::None;
            };
            self.confirming = false;
            if action == SheetAction::Confirm {
                let id = self
                    .form
                    .as_ref()
                    .and_then(|form| form.id)
                    .expect("existing event");
                let mut next = self.state.data.clone();
                next.remove(id);
                if self.write(next, "Event removed.") {
                    self.form = None;
                    self.detail = None;
                }
            }
            return Change::Full;
        }
        if back_button(WIDTH as i32).contains(x, y) {
            self.form = None;
            self.notice.clear();
            return Change::Full;
        }
        if SAVE.contains(x, y) {
            return self.save_form();
        }
        let Some(form) = &mut self.form else {
            return Change::None;
        };
        if REMOVE.contains(x, y) && form.id.is_some() {
            self.confirming = true;
            return Change::Full;
        }
        if TITLE.contains(x, y) {
            form.typing = Typing::Title;
            return Change::Full;
        }
        if DATE.contains(x, y) {
            self.date = Some(DatePicker::new(self.today, Some(form.date), false));
            return Change::Full;
        }
        if ALLDAY.contains(x, y) && form.time.is_some() {
            form.time = None;
            form.typing = Typing::None;
            return Change::Full;
        }
        if ALLDAY.contains(x, y) || TIME.contains(x, y) {
            form.time.get_or_insert_with(|| editor("", 5));
            form.typing = Typing::Time;
            return Change::Full;
        }
        let active = match form.typing {
            Typing::Title => Some(&mut form.title),
            Typing::Time => form.time.as_mut(),
            Typing::None => None,
        };
        match active.and_then(|active| press_key(active, x, y)) {
            Some(Press::Go) => {
                form.typing = Typing::None;
                Change::Full
            }
            Some(Press::Nothing) | None => Change::None,
            _ => Change::Partial,
        }
    }

    fn tap_date(&mut self, x: f32, y: f32) -> Change {
        let Some(picker) = &mut self.date else {
            return Change::None;
        };
        match picker.tap(x, y) {
            Pick::Date(date) => {
                if let Some(form) = &mut self.form {
                    form.date = date;
                } else {
                    self.start = date;
                    self.week = false;
                    self.page = 0;
                }
                self.date = None;
                Change::Full
            }
            Pick::Cancel => {
                self.date = None;
                Change::Full
            }
            Pick::Moved => Change::Full,
            _ => Change::None,
        }
    }

    fn tap_import(&mut self, x: f32, y: f32) -> Change {
        if back_button(WIDTH as i32).contains(x, y) {
            return if matches!(self.import, Some(Import::Review { .. })) {
                self.scan_files()
            } else {
                self.import = None;
                self.notice.clear();
                Change::Full
            };
        }
        match &mut self.import {
            Some(Import::Files { paths, page }) => {
                if page_tap(page, paths.len(), x, y) {
                    return Change::Full;
                }
                if let Some(index) = (0..PAGE_SIZE).find(|index| file_row(*index).contains(x, y)) {
                    if let Some(path) = paths.get(*page * PAGE_SIZE + index).cloned() {
                        return self.preview_file(path);
                    }
                }
            }
            Some(Import::Review { .. }) if IMPORT_ACTION.contains(x, y) => {
                return self.import_file()
            }
            _ => {}
        }
        Change::None
    }

    fn turn(&mut self, forward: bool) -> Change {
        if let Some(Import::Files { paths, page }) = &mut self.import {
            if page_step(page, paths.len(), forward) {
                Change::Full
            } else {
                Change::None
            }
        } else if self.import.is_none()
            && self.form.is_none()
            && self.detail.is_none()
            && self.date.is_none()
        {
            let count = self.events().len();
            if page_step(&mut self.page, count, forward) {
                Change::Full
            } else {
                Change::None
            }
        } else {
            Change::None
        }
    }
}

impl App for View {
    fn draw(&self, canvas: &mut Canvas) {
        if let Some(date) = &self.date {
            date.draw(canvas);
        } else if let Some(import) = &self.import {
            self.draw_import(canvas, import);
        } else if let Some(form) = &self.form {
            self.draw_form(canvas, form);
            if self.confirming {
                remove_sheet().draw(canvas, None);
            }
        } else if let Some(event) = self.detail.and_then(|id| self.state.data.event(id)) {
            self.draw_detail(canvas, event);
        } else {
            self.draw_home(canvas);
        }
    }

    fn input(&mut self, input: Input) -> Change {
        if self.live && !matches!(input, Input::Sleep) {
            let today = local_day();
            if today != self.today {
                if self.start == self.today {
                    self.start = today;
                }
                self.today = today;
                self.page = 0;
                return Change::Full;
            }
        }
        match input {
            Input::Tap(x, y) if self.date.is_some() => self.tap_date(x, y),
            Input::Tap(x, y) if self.import.is_some() => self.tap_import(x, y),
            Input::Tap(x, y) if self.form.is_some() => self.tap_form(x, y),
            Input::Tap(x, y) if self.detail.is_some() => self.tap_detail(x, y),
            Input::Tap(x, y) => self.tap_home(x, y),
            Input::Swipe { dx, dy, .. } if dx.abs() > dy.abs() => self.turn(dx < 0.0),
            Input::PageBack => self.turn(false),
            Input::PageForward => self.turn(true),
            _ => Change::None,
        }
    }

    fn storage_returned(&mut self) -> io::Result<()> {
        self.state.reload()?;
        self.start = self.today;
        self.page = 0;
        self.detail = None;
        self.form = None;
        self.date = None;
        self.confirming = false;
        self.import = None;
        self.notice.clear();
        Ok(())
    }
}

fn calendar_paths(directory: &Path) -> io::Result<Vec<PathBuf>> {
    let mut result = Vec::new();
    for item in fs::read_dir(directory)?.take(1024) {
        let item = item?;
        if item.file_type()?.is_file()
            && item
                .path()
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("ics"))
        {
            result.push(item.path());
        }
    }
    result.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    Ok(result)
}

fn parse_time(text: &str) -> Option<NaiveTime> {
    if text.len() == 4 && text.bytes().all(|byte| byte.is_ascii_digit()) {
        NaiveTime::parse_from_str(text, "%H%M").ok()
    } else if text.len() == 5 {
        NaiveTime::parse_from_str(text, "%H:%M").ok()
    } else {
        None
    }
}

fn time_text(event: &Event) -> String {
    event
        .time
        .map_or_else(|| "All day".into(), |time| time.format("%H:%M").to_string())
}

fn browse_row(index: usize) -> Rect {
    rect(40, 438 + index as i32 * 121, 1184, 108)
}
fn file_row(index: usize) -> Rect {
    rect(40, 278 + index as i32 * 121, 1184, 108)
}
fn remove_sheet() -> Sheet<'static> {
    Sheet::centered(
        rect(0, 0, 1264, 1680),
        "Remove event?",
        "This removes your local copy of the event.",
        "Remove",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_time_form_accepts_four_digits_and_standard_clock_text() {
        let expected = NaiveTime::from_hms_opt(9, 30, 0);
        assert_eq!(parse_time("0930"), expected);
        assert_eq!(parse_time("09:30"), expected);
        assert_eq!(parse_time("2460"), None);
        assert_eq!(parse_time("23"), None);
    }
}
