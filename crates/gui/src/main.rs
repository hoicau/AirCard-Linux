#![forbid(unsafe_code)]
mod app;
mod model;
mod worker;
use clap::Parser;
use std::path::PathBuf;
#[derive(Parser)]
#[command(
    name = "aircard-gui",
    version,
    about = "AirCard-Linux: native Wallet card artwork and recovery"
)]
struct Args {
    /// Matching CLI executable; defaults to aircard beside this executable.
    #[arg(long)]
    cli: Option<PathBuf>,
    #[arg(long)]
    dark: bool,
    /// Deterministic UI verification only: synthetic data, no device access, screenshots then exit.
    #[arg(long, hide = true)]
    smoke_dir: Option<PathBuf>,
}
fn main() -> eframe::Result {
    let args = Args::parse();
    let cli = args.cli.unwrap_or_else(|| {
        std::env::current_exe()
            .unwrap_or_default()
            .with_file_name("aircard")
    });
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("AirCard Linux")
            .with_inner_size([1080.0, 780.0])
            .with_min_inner_size([680.0, 520.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "AirCard Linux",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, cli, args.dark, args.smoke_dir)))),
    )
}
