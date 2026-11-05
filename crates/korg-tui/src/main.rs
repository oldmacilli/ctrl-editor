//! Terminal UI for the KORG editor rewrite.
//!
//! Same core as `korg-slint`: an `EditorController` owns the document; device
//! receive and write run the measured exchange (`korg_app::run_job`) on a
//! worker thread so the screen stays live during a 2-minute Bluetooth receive.

mod app;
#[cfg(test)]
mod tests;
mod ui;

use app::{App, Hits, JobMessage, Screen};
use korg_core::{EditorController, SystemClock};
use korg_devices::Catalog;
use korg_midi_midir::MidirTransport;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind,
};
use ratatui::crossterm::execute;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

const USAGE: &str = "\
usage: korg-tui [--port NAME] [--midi-backend ID] [--open-device CLASS] [--timeout MS] [FILE]

  --port NAME          MIDI port to use (part of its name); default: match the device name
  --midi-backend ID    alsa or jack on Linux (also CTRL_MIDI_BACKEND); M switches it
  --open-device CLASS  start in the editor with this device (for example X14802)
  --timeout MS         reply timeout per device step (default 3000)
  FILE                 a KORG scene-set, scene or global file to open";

struct Options {
    port: Option<String>,
    midi_backend: Option<String>,
    open_device: Option<String>,
    timeout_ms: u64,
    file: Option<PathBuf>,
}

fn parse_args() -> Result<Options, String> {
    let mut options = Options {
        port: None,
        midi_backend: None,
        open_device: None,
        timeout_ms: korg_app::TRANSFER_TIMEOUT_MS,
        file: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "-h" | "--help" => return Err(String::new()),
            "--port" => options.port = Some(value("--port")?),
            "--midi-backend" => options.midi_backend = Some(value("--midi-backend")?),
            "--open-device" => options.open_device = Some(value("--open-device")?),
            "--timeout" => {
                options.timeout_ms = value("--timeout")?
                    .parse()
                    .map_err(|_| "--timeout must be milliseconds".to_owned())?
            }
            other if other.starts_with('-') => return Err(format!("unknown option: {other}")),
            other => options.file = Some(PathBuf::from(other)),
        }
    }
    Ok(options)
}

fn main() -> std::process::ExitCode {
    let options = match parse_args() {
        Ok(options) => options,
        Err(error) => {
            if !error.is_empty() {
                eprintln!("{error}");
            }
            eprintln!("{USAGE}");
            return std::process::ExitCode::from(if error.is_empty() { 0 } else { 2 });
        }
    };
    if let Err(error) = korg_app::panel::validate_external_panels() {
        eprintln!("device panels: {error}");
        return std::process::ExitCode::FAILURE;
    }
    // Flag, then CTRL_MIDI_BACKEND, then the saved choice, then the
    // platform's own; before the first transport.
    let saved = korg_app::preferences::load().midi_backend;
    if let Err(error) = korg_midi_midir::init_backend(options.midi_backend.as_deref(), &saved) {
        eprintln!("{error}");
        return std::process::ExitCode::from(2);
    }
    let catalog = match Catalog::load() {
        Ok(catalog) => catalog,
        Err(error) => {
            eprintln!("device catalog: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let controller = EditorController::new(MidirTransport::new(), SystemClock::new());
    // Its own connection: the controller's port scans stay on this thread.
    let monitor = korg_app::monitor::SceneMonitor::new(MidirTransport::new);
    let mut app = App::new(controller, monitor, catalog, korg_app::products_dir());
    app.port_override = options.port;
    app.midi = korg_app::MidiBackendSwitch::new(
        korg_midi_midir::backend_info(korg_midi_midir::backend()),
        korg_midi_midir::backends().len(),
        korg_midi_midir::switch_to_next_backend,
    );
    if app.midi.current().problem.is_some() {
        app.status = app.midi.status();
    }
    if let Some(class) = &options.open_device {
        match app.catalog.find_by_class(class).cloned() {
            Some(device) => {
                match korg_app::open_device(&mut app.controller, &device, &app.products) {
                    Ok(status) => {
                        app.class = Some(device.class);
                        app.screen = Screen::Editor;
                        app.status = status;
                    }
                    Err(error) => app.status = error,
                }
            }
            None => app.status = format!("Unknown device class {class}."),
        }
    }
    if let Some(path) = &options.file {
        match korg_app::load_path(&mut app.controller, &app.catalog, path) {
            Ok((class, status)) => {
                app.class = class;
                app.path = Some(path.clone());
                app.screen = Screen::Editor;
                app.status = status;
            }
            Err(error) => app.status = error,
        }
    }

    let mut terminal = ratatui::init();
    // ratatui's panic hook restores the terminal; release the mouse first.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(std::io::stdout(), DisableMouseCapture);
        hook(info);
    }));
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    let result = run(&mut terminal, &mut app, options.timeout_ms);
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("korg-tui: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App<MidirTransport, SystemClock>,
    timeout_ms: u64,
) -> std::io::Result<()> {
    let (sender, receiver) = mpsc::channel::<JobMessage>();
    while !app.quit {
        app.tick();
        while let Ok(message) = receiver.try_recv() {
            app.on_job(message);
        }
        if let Some(request) = app.take_job() {
            let sender = sender.clone();
            // The worker builds its own transport: the controller's stays on
            // this thread for port scans.
            std::thread::spawn(move || {
                let progress_sender = sender.clone();
                let result = korg_app::run_job_restoring(
                    MidirTransport::new(),
                    SystemClock::new(),
                    &request.input,
                    &request.output,
                    timeout_ms,
                    request.job,
                    request.active,
                    &mut |step| {
                        let _ = progress_sender.send(JobMessage::Progress(step));
                    },
                );
                let _ = sender.send(JobMessage::Done(result));
            });
        }
        if app.take_detect() {
            let sender = sender.clone();
            let catalog = app.catalog.clone();
            std::thread::spawn(move || {
                let result = korg_app::detect::detect(
                    MidirTransport::new,
                    &catalog,
                    korg_app::detect::PROBE_TIMEOUT,
                );
                let _ = sender.send(JobMessage::Detected(result));
            });
        }
        let mut hits = Hits::default();
        terminal.draw(|frame| hits = ui::draw(frame, app))?;
        app.hits = hits;
        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => app.on_key(key),
                Event::Mouse(mouse) => app.on_mouse(mouse),
                _ => {}
            }
        }
    }
    Ok(())
}
