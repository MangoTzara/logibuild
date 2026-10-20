//! LogiBuild: digital logic circuit editor and simulator.

mod app;
mod autoroute;
mod canvas;
mod icons;
mod palette;
mod props;
mod routing;
mod settings;
mod shapes;
mod theme;
mod waveform;

fn main() -> iced::Result {
    let mut file = None;
    let mut run = false;
    for arg in std::env::args_os().skip(1) {
        match arg.to_str() {
            Some("--run") => run = true,
            Some("--help" | "-h") => {
                println!("usage: logibuild [--run] [FILE.lbc]\n  --run  start the simulation immediately");
                return Ok(());
            }
            _ => file = Some(std::path::PathBuf::from(arg)),
        }
    }
    iced::application(
        move || app::App::new(file.clone(), run, settings::Settings::user_path()),
        app::App::update,
        app::App::view,
    )
    .font(theme::FONT_DATA[0])
    .font(theme::FONT_DATA[1])
    .title(app::App::title)
    .subscription(app::App::subscription)
    .theme(|_: &app::App| theme::theme())
    .default_font(theme::FONT)
    .window_size((1440.0, 900.0))
    .exit_on_close_request(false)
    .antialiasing(true)
    .run()
}

#[cfg(test)]
mod tests;
