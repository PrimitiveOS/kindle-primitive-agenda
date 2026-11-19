use std::{
    fs,
    path::{Path, PathBuf},
};

use agenda_core::Agenda;
use agenda_render::View;
use primitive_app::{font_paths, frame, write_png, App, Change, HEIGHT, WIDTH};
use primitive_ui::{Canvas, Input};

struct Fixture<'a>(&'a Path);
impl Drop for Fixture<'_> {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(self.0);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map_or_else(|| PathBuf::from("build/previews"), PathBuf::from);
    fs::create_dir_all(&output)?;
    let fonts =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../doannt-kindle-launcher/assets/fonts");
    let mut canvas = Canvas::new(WIDTH, HEIGHT, &font_paths(&fonts))?;
    let mut capture = |view: &View, name: &str, dark| {
        write_png(
            &output.join(name),
            WIDTH,
            HEIGHT,
            &frame(view, &mut canvas, dark),
        )
    };
    let today: chrono::NaiveDate = "2026-11-20".parse()?;

    let mut model = Agenda::default();
    model
        .put(None, "Return library books", today, None)
        .unwrap();
    model
        .put(None, "Study group", today, Some("14:00:00".parse()?))
        .unwrap();
    model
        .put(
            None,
            "Read chapter three",
            today.succ_opt().unwrap(),
            Some("10:30:00".parse()?),
        )
        .unwrap();
    let fixture =
        std::env::temp_dir().join(format!("primitive-agenda-preview-{}", std::process::id()));
    fs::create_dir(&fixture)?;
    let _cleanup = Fixture(&fixture);
    fs::write(
        fixture.join("Study calendar.ics"),
        concat!(
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:lesson\r\n",
            "SUMMARY:Review vocabulary\r\nDTSTART:20261120T163000\r\nEND:VEVENT\r\n",
            "BEGIN:VEVENT\r\nSUMMARY:Revision day\r\nDTSTART;VALUE=DATE:20261121\r\nEND:VEVENT\r\n",
            "END:VCALENDAR\r\n"
        ),
    )?;
    let mut view = View::preview(today, model, Some(fixture.clone()));
    assert_eq!(view.input(Input::Tap(475.0, 219.0)), Change::Full);
    capture(&view, "agenda-week.png", false)?;
    assert_eq!(view.input(Input::Tap(212.0, 590.0)), Change::Full);
    capture(&view, "agenda-event-dark.png", true)?;
    assert_eq!(view.input(Input::Tap(1110.0, 85.0)), Change::Full);
    assert_eq!(view.input(Input::Tap(1030.0, 340.0)), Change::Full);
    assert_eq!(view.input(Input::Tap(290.0, 320.0)), Change::Full);
    capture(&view, "agenda-import.png", false)?;
    assert_eq!(view.input(Input::Tap(1030.0, 1440.0)), Change::Full);
    Ok(())
}
