// No console window for the release app; tests keep theirs for output.
#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

mod alert;
mod cache;
mod json;
mod model;
mod panel;
mod providers;
mod schedule;
mod settings;
mod timefmt;
mod view;
#[cfg(windows)]
mod win;

fn main() {
    #[cfg(windows)]
    {
        if std::env::args().any(|arg| arg == "--check") {
            win::check();
        } else {
            win::run();
        }
    }
}
