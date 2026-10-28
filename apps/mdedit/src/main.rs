//! mdedit: native Markdown editor. OWNER: agent "app".

mod app;
mod cli;
mod dialogs;
mod find;
mod fonts;
mod links;
mod menu;
mod prefs;
mod sidebar;
mod tabs;
mod theme;
mod toast;
#[cfg(test)]
mod ui_tests;

use cli::Cli;

fn main() -> std::process::ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let paths = match cli::parse_args(std::env::args_os().skip(1)) {
        Ok(Cli::Help) => {
            print!("{}", cli::HELP);
            return std::process::ExitCode::SUCCESS;
        }
        Ok(Cli::Version) => {
            println!("mdedit {}", cli::VERSION);
            return std::process::ExitCode::SUCCESS;
        }
        Ok(Cli::Run(paths)) => paths,
        Err(e) => {
            eprintln!("mdedit: {e}");
            return std::process::ExitCode::from(2);
        }
    };
    let targets = cli::classify(&paths);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id("io.github.mdedit.mdedit")
            .with_title("mdedit")
            .with_inner_size([1100.0, 750.0])
            .with_min_inner_size([480.0, 320.0])
            .with_drag_and_drop(true),
        persist_window: false,
        ..Default::default()
    };
    let res = eframe::run_native("mdedit", options, Box::new(move |cc| Ok(Box::new(app::MdApp::new(cc, targets)))));
    match res {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mdedit: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
