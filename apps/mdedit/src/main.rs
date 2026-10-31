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
mod single;
mod svg_loader;
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
    let mut targets = cli::classify(&paths);
    let config = mdedit_workspace::Config::load();

    // Open in the running window if there is one (a failure here just means a new window).
    let mut listener = None;
    let socket = config.single_window.then(mdedit_workspace::socket_path).flatten();
    if let Some(sock) = &socket {
        match single::claim(sock, &targets.files) {
            Ok(single::Claim::Forwarded) => return std::process::ExitCode::SUCCESS,
            Ok(single::Claim::First(l)) => listener = Some(l),
            Err(e) => log::warn!("{}: {e}", sock.display()),
        }
    }
    let owns_socket = listener.is_some();

    // A separate window opened for specific files has nothing to do with the last session.
    targets.save_session = config.single_window || targets.files.is_empty();
    if config.restore_session && targets.save_session {
        targets.session = Some(mdedit_workspace::Session::load());
    }

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
    let res = eframe::run_native("mdedit", options, Box::new(move |cc| {
        let app = app::MdApp::new(cc, targets);
        if let Some(l) = listener {
            app.listen(l);
        }
        Ok(Box::new(app))
    }));
    if owns_socket && let Some(sock) = &socket {
        let _ = std::fs::remove_file(sock);
    }
    match res {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mdedit: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
