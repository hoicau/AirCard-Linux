//! Wallet artwork UI with supervised native transactions (MIT).
use crate::{
    model::{Confirmation, Device, redacted},
    worker::{self, Message},
};
use aircard_core::assets::{PreparedCard, Resource, resources_zip};
use eframe::egui::{self, Color32, RichText, TextureHandle, Vec2};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        atomic::Ordering,
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Artwork,
    Device,
    Help,
}
#[derive(Clone, Copy)]
enum Field {
    CardInput,
    CardOutput,
    Snapshot,
    Token,
}
enum LocalResult {
    Card(PreparedCard),
    Saved,
    Picked(Field, Option<PathBuf>),
}
type LocalReceiver = Receiver<Result<LocalResult, String>>;
struct Smoke {
    dir: PathBuf,
    phase: usize,
    frames: u32,
    requested: bool,
    started: Instant,
}
const SMOKE_NAMES: [&str; 6] = [
    "artwork-light",
    "artwork-dark",
    "help-light",
    "device-light",
    "confirmation-dark",
    "compact-light",
];
pub struct App {
    cli: PathBuf,
    tab: Tab,
    dark: bool,
    card_input: String,
    card_output: String,
    card: Option<PreparedCard>,
    card_texture: Option<TextureHandle>,
    cards: Vec<String>,
    card_hash: String,
    devices: Vec<Device>,
    selected: Option<usize>,
    route: String,
    snapshot: String,
    token: String,
    journal: String,
    job: Option<worker::Job>,
    job_name: String,
    local: Option<LocalReceiver>,
    pending: Option<Confirmation>,
    restore_review: Option<Confirmation>,
    stage: String,
    status: String,
    failed: bool,
    logs: VecDeque<String>,
    details: bool,
    close_when_idle: bool,
    smoke: Option<Smoke>,
}
impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        cli: PathBuf,
        dark: bool,
        smoke_dir: Option<PathBuf>,
    ) -> Self {
        let mut app = Self::empty(cli, dark);
        app.style(&cc.egui_ctx);
        if let Some(dir) = smoke_dir {
            // Never enumerate hardware in deterministic UI checks.
            app.smoke = Some(Smoke {
                dir,
                phase: 0,
                frames: 0,
                requested: false,
                started: Instant::now(),
            });
            app.devices.push(Device {
                udid: "synthetic-test-000001".into(),
                route: "usb".into(),
                ios: "27.0 (fixture)".into(),
                paired: true,
                status: "paired_session_verified".into(),
            });
            app.selected = Some(0);
            app.card_input = "Synthetic artwork / no user data".into();
            app.card_output = "wallet-resources.zip".into();
            let image = image::RgbaImage::from_fn(480, 320, |x, y| {
                image::Rgba([
                    ((x * 180 / 480) + 35) as u8,
                    ((y * 100 / 320) + 70) as u8,
                    155,
                    255,
                ])
            });
            if let Ok(card) = PreparedCard::from_image(image::DynamicImage::ImageRgba8(image)) {
                app.install_card(&cc.egui_ctx, card);
            }
            app.snapshot = "card-backup.json".into();
            app.card_hash = "fixture-card-identifier".into();
            app.token = "private-token.bin".into();
            app.journal = "recovery.json".into();
            app.stage = "Ready".into();
            app.status = "UI verification with synthetic data. Device access disabled.".into();
        } else {
            app.refresh();
        }
        app
    }
    fn empty(cli: PathBuf, dark: bool) -> Self {
        Self {
            cli,
            tab: Tab::Artwork,
            dark,
            card_input: String::new(),
            card_output: String::new(),
            card: None,
            card_texture: None,
            cards: vec![],
            card_hash: String::new(),
            devices: vec![],
            selected: None,
            route: "usb".into(),
            snapshot: String::new(),
            token: String::new(),
            journal: String::new(),
            job: None,
            job_name: String::new(),
            local: None,
            pending: None,
            restore_review: None,
            stage: "Idle".into(),
            status: "Prepare artwork, select your iPhone and scan the target card.".into(),
            failed: false,
            logs: VecDeque::new(),
            details: false,
            close_when_idle: false,
            smoke: None,
        }
    }
    fn style(&self, ctx: &egui::Context) {
        let mut visuals = if self.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        visuals.panel_fill = if self.dark {
            Color32::from_rgb(18, 23, 28)
        } else {
            Color32::from_rgb(247, 249, 251)
        };
        visuals.window_fill = if self.dark {
            Color32::from_rgb(27, 33, 39)
        } else {
            Color32::WHITE
        };
        visuals.selection.bg_fill = Color32::from_rgb(18, 116, 109);
        visuals.selection.stroke.color = Color32::WHITE;
        ctx.set_visuals(visuals);
        ctx.style_mut(|s| {
            s.spacing.item_spacing = Vec2::new(10.0, 10.0);
            s.spacing.button_padding = Vec2::new(14.0, 8.0);
            s.text_styles
                .insert(egui::TextStyle::Body, egui::FontId::proportional(15.0));
            s.text_styles
                .insert(egui::TextStyle::Heading, egui::FontId::proportional(27.0));
        });
    }
    fn busy(&self) -> bool {
        self.job.is_some() || self.local.is_some()
    }
    fn chosen(&self) -> Option<Device> {
        self.selected
            .and_then(|i| self.devices.get(i))
            .filter(|d| d.route == self.route)
            .cloned()
    }
    fn log(&mut self, s: String) {
        if self.logs.len() >= 300 {
            self.logs.pop_front();
        }
        self.logs.push_back(s);
    }
    fn start(&mut self, name: &str, args: Vec<String>) {
        if self.busy() || self.smoke.is_some() {
            return;
        }
        self.failed = false;
        self.status = format!("{name} is running.");
        self.stage = name.into();
        self.job_name = name.into();
        self.job = Some(worker::start(self.cli.clone(), args));
    }
    fn refresh(&mut self) {
        if self.busy() {
            return;
        }
        self.cards.clear();
        self.card_hash.clear();
        self.devices.clear();
        self.selected = None;
        self.start(
            "Discover devices",
            vec!["devices".into(), "--show-identifiers".into()],
        );
    }
    fn device_job(&mut self, name: &str, mut args: Vec<String>) {
        if let Some(d) = self.chosen() {
            args.extend(d.selector());
            self.start(name, args);
        }
    }
    fn local_job(
        &mut self,
        name: &str,
        work: impl FnOnce() -> Result<LocalResult, String> + Send + 'static,
    ) {
        if self.busy() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.local = Some(rx);
        self.failed = false;
        self.stage = name.into();
        self.status = format!("{name} is running.");
        std::thread::spawn(move || {
            let _ = tx.send(work());
        });
    }
    fn picker(&mut self, field: Field, save: bool) {
        self.local_job("Choose file", move || {
            let dialog = rfd::FileDialog::new();
            let path = if save {
                dialog.save_file()
            } else {
                dialog.pick_file()
            };
            Ok(LocalResult::Picked(field, path))
        });
    }
    fn field(&mut self, field: Field) -> &mut String {
        match field {
            Field::CardInput => &mut self.card_input,
            Field::CardOutput => &mut self.card_output,
            Field::Snapshot => &mut self.snapshot,
            Field::Token => &mut self.token,
        }
    }
    fn path_row(&mut self, ui: &mut egui::Ui, label: &str, field: Field, save: bool) {
        ui.label(RichText::new(label).strong());
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 95.0).max(120.0);
            if ui
                .add_sized(
                    [width, 32.0],
                    egui::TextEdit::singleline(self.field(field))
                        .hint_text("Choose a file or enter its path"),
                )
                .changed()
            {
                self.invalidate(field);
            }
            if ui.button("Browse").clicked() {
                self.picker(field, save);
            }
        });
    }
    fn invalidate(&mut self, field: Field) {
        if let Field::CardInput = field {
            self.card = None;
            self.card_texture = None;
        }
    }
    fn install_card(&mut self, ctx: &egui::Context, card: PreparedCard) {
        self.card_texture = Some(ctx.load_texture(
            "artwork",
            egui::ColorImage::from_rgba_unmultiplied([1536, 969], &card.rgba),
            egui::TextureOptions::LINEAR,
        ));
        self.card = Some(card);
    }
    fn poll(&mut self, ctx: &egui::Context) {
        let mut messages = vec![];
        let mut disconnected = false;
        if let Some(job) = &self.job {
            loop {
                match job.receiver.try_recv() {
                    Ok(m) => messages.push(m),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        for message in messages {
            match message {
                Message::Event(v) => {
                    if let Some(d) = Device::from_event(&v) {
                        self.devices.push(d);
                        if self.selected.is_none() {
                            self.selected = self.devices.iter().position(|d| d.route == self.route);
                        }
                    }
                    if v["event"] == "card_match"
                        && let Some(hash) = v["hash"].as_str()
                        && aircard_core::customization::Target::WalletArtwork(hash.into())
                            .validate()
                            .is_ok()
                        && !self.cards.iter().any(|h| h == hash)
                    {
                        self.cards.push(hash.into());
                    }
                    if let Some(stage) = v["stage"].as_str().or(v["state"].as_str()) {
                        self.stage = stage.into();
                    }
                    self.log(redacted(&v).to_string());
                }
                Message::Finished(result) => {
                    self.job = None;
                    match result {
                        Ok(()) => {
                            self.failed = false;
                            self.status = format!("{} completed.", self.job_name);
                            self.stage = "Complete".into();
                            if let Some(c) = self.restore_review.take() {
                                self.pending = Some(c);
                            }
                        }
                        Err(e) => {
                            self.failed = true;
                            self.status = e;
                            self.restore_review = None;
                        }
                    }
                }
            }
        }
        if disconnected && self.job.is_some() {
            self.job = None;
            self.failed = true;
            self.status =
                "CLI monitor stopped unexpectedly. Inspect recovery status before retrying.".into();
        }
        let result = self.local.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(r) => Some(r),
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("Background task stopped unexpectedly.".into()))
            }
            Err(_) => None,
        });
        if let Some(result) = result {
            self.local = None;
            match result {
                Ok(LocalResult::Card(card)) => {
                    self.install_card(ctx, card);
                    self.status = "Artwork prepared. Preview and export are ready.".into();
                    self.stage = "Prepared".into();
                }
                Ok(LocalResult::Saved) => {
                    self.status = "Export saved. Existing files were preserved.".into();
                    self.stage = "Saved".into();
                }
                Ok(LocalResult::Picked(field, Some(path))) => {
                    *self.field(field) = path.to_string_lossy().into_owned();
                    self.invalidate(field);
                    self.stage = "Ready".into();
                    self.status = "File selected.".into();
                }
                Ok(LocalResult::Picked(_, None)) => {
                    self.stage = "Ready".into();
                    self.status = "File selection cancelled.".into();
                }
                Err(e) => {
                    self.failed = true;
                    self.log(e.clone());
                    self.status = e;
                    self.stage = "Failed".into();
                }
            }
        }
    }
    fn review(&mut self, title: &str, summary: &str, args: Vec<String>) {
        if let Some(device) = self.chosen() {
            self.pending = Some(Confirmation {
                title: title.into(),
                summary: summary.into(),
                device,
                args,
                accepted: false,
            });
        }
    }
    fn artwork(&mut self, ui: &mut egui::Ui) {
        title(
            ui,
            "Card artwork",
            "Prepare the artwork you will apply to your Wallet card.",
        );
        self.path_row(ui, "Source image", Field::CardInput, false);
        if ui
            .add_enabled(
                !self.card_input.trim().is_empty(),
                egui::Button::new("Prepare artwork"),
            )
            .clicked()
        {
            let input = PathBuf::from(self.card_input.trim());
            if self.card_output.is_empty() {
                self.card_output = input
                    .with_file_name("wallet-resources.zip")
                    .display()
                    .to_string();
            }
            self.local_job("Prepare artwork", move || {
                let bytes = cli::local::read(&input, aircard_core::assets::MAX_IMAGE_BYTES, false)
                    .map_err(|e| e.to_string())?;
                PreparedCard::from_bytes(&bytes)
                    .map(LocalResult::Card)
                    .map_err(|e| e.to_string())
            });
        }
        ui.add_space(12.0);
        if ui.available_width() > 680.0 {
            ui.columns(2, |columns| {
                self.card_preview(&mut columns[0]);
                self.card_exports(&mut columns[1]);
            });
        } else {
            self.card_preview(ui);
            ui.add_space(12.0);
            self.card_exports(ui);
        }
    }
    fn card_preview(&self, ui: &mut egui::Ui) {
        if let Some(texture) = &self.card_texture {
            let width = ui.available_width().min(420.0);
            ui.add(
                egui::Image::new(texture)
                    .fit_to_exact_size(Vec2::new(width, width * 969.0 / 1536.0))
                    .corner_radius(16),
            );
            ui.label(
                RichText::new("1536 × 969 / centered crop / PNG + PDF")
                    .small()
                    .weak(),
            );
        } else {
            empty_preview(ui, "Your artwork", "PNG, JPEG or WebP / up to 32 MiB");
        }
    }
    fn card_exports(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        self.path_row(ui, "Resource archive", Field::CardOutput, true);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    self.card.is_some() && !self.card_output.trim().is_empty(),
                    egui::Button::new("Export resources"),
                )
                .clicked()
            {
                let resources = self.card.as_ref().unwrap().resources();
                let output = PathBuf::from(self.card_output.trim());
                self.export(resources, output);
            }
            if ui
                .add_enabled(self.card.is_some(), egui::Button::new("Save PNG preview…"))
                .clicked()
            {
                let png = self.card.as_ref().unwrap().png.clone();
                self.local_job("Save preview", move || {
                    let Some(path) = rfd::FileDialog::new()
                        .set_file_name("card-preview.png")
                        .save_file()
                    else {
                        return Ok(LocalResult::Picked(Field::CardOutput, None));
                    };
                    cli::local::write_new(&path, &png).map_err(|e| e.to_string())?;
                    Ok(LocalResult::Saved)
                });
            }
        });
        ui.add_space(8.0);
        ui.label(RichText::new("Exports @3x.png, @2x.png and PDF artwork. Select Apply & restore to use this artwork on your iPhone.").small().weak());
    }
    fn export(&mut self, resources: Vec<Resource>, output: PathBuf) {
        self.local_job("Export resources", move || {
            let bytes = resources_zip(&resources).map_err(|e| e.to_string())?;
            cli::local::write_new(&output, &bytes).map_err(|e| e.to_string())?;
            Ok(LocalResult::Saved)
        });
    }
    fn device(&mut self, ui: &mut egui::Ui) {
        title(
            ui,
            "Apply & restore",
            "Select one paired iPhone and one Wallet card.",
        );
        let previous = self.chosen();
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.route, "usb".into(), "USB");
            ui.selectable_value(&mut self.route, "wifi".into(), "Wi-Fi");
            if ui.button("Refresh devices").clicked() {
                self.refresh();
            }
        });
        egui::ComboBox::from_id_salt("device-select")
            .width(ui.available_width().min(440.0))
            .selected_text(
                self.chosen()
                    .map_or_else(|| "Choose a device".into(), |d| d.label()),
            )
            .show_ui(ui, |ui| {
                for (i, d) in self
                    .devices
                    .iter()
                    .enumerate()
                    .filter(|(_, d)| d.route == self.route)
                {
                    ui.selectable_value(&mut self.selected, Some(i), d.label());
                }
            });
        if previous != self.chosen() {
            self.cards.clear();
            self.card_hash.clear();
        }
        let paired = self.chosen().is_some_and(|d| d.paired);
        if let Some(d) = self.chosen() {
            ui.label(format!(
                "iOS {} / {} / {}",
                d.ios,
                d.route.to_uppercase(),
                if d.paired { "Paired" } else { &d.status }
            ));
        }
        ui.label(
            "Open Wallet and select the intended card while scanning. Select its identifier below.",
        );
        if ui
            .add_enabled(paired, egui::Button::new("Scan Wallet / 20 s"))
            .clicked()
        {
            self.cards.clear();
            self.card_hash.clear();
            self.device_job(
                "Scan Wallet",
                vec![
                    "scan".into(),
                    "--duration".into(),
                    "20".into(),
                    "--show-hashes".into(),
                ],
            );
        }
        egui::ComboBox::from_id_salt("card-select")
            .selected_text(if self.card_hash.is_empty() {
                "Choose detected card"
            } else {
                &self.card_hash
            })
            .show_ui(ui, |ui| {
                for hash in &self.cards {
                    ui.selectable_value(&mut self.card_hash, hash.clone(), hash);
                }
            });
        self.path_row(
            ui,
            "Private card backup (new for Apply, existing for Restore)",
            Field::Snapshot,
            false,
        );
        if ui.button("Choose new backup path…").clicked() {
            self.picker(Field::Snapshot, true);
        }
        self.path_row(ui, "Private sync token", Field::Token, false);
        ui.label(RichText::new("Recovery directory").strong());
        ui.add(
            egui::TextEdit::singleline(&mut self.journal)
                .hint_text("Unused directory for Apply / Restore; existing directory for Recover"),
        );
        let ready = paired && !self.token.trim().is_empty() && !self.journal.trim().is_empty();
        let common = vec![
            "--journal".into(),
            self.journal.trim().into(),
            "--grappa-token".into(),
            self.token.trim().into(),
            "--timeout".into(),
            "25".into(),
        ];
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(ready && self.card.is_some() && !self.card_hash.is_empty() && !self.snapshot.trim().is_empty(), egui::Button::new("Review apply…")).clicked() {
                let mut args = vec!["card-apply".into(), self.card_input.trim().into(), "--card-hash".into(), self.card_hash.clone(), "--backup".into(), self.snapshot.trim().into()];
                args.extend(["--expected-artwork-sha256".into(), aircard_core::sha256(&self.card.as_ref().unwrap().png)]);
                args.extend(common.clone());
                self.review("Apply card artwork", "Replace this card's existing background images, invalidate its display caches and save a private restore backup. Keep Wallet and Books closed during the operation. Reopen Wallet when complete.", args);
            }
            if ui.add_enabled(ready && !self.snapshot.trim().is_empty(), egui::Button::new("Review restore…")).clicked() {
                let mut args = vec!["card-restore".into(), self.snapshot.trim().into()]; args.extend(common.clone());
                self.restore_review = self.chosen().map(|device| Confirmation { title: "Restore card artwork".into(), summary: "Restore the artwork and display caches from this private backup on its original device. Keep Wallet and Books closed.".into(), device, args: args.clone(), accepted:false });
                self.start("Validate backup", args);
            }
            if ui.add_enabled(ready, egui::Button::new("Review recovery…")).clicked() {
                self.review("Recover interrupted card operation", "Restore the card's originals and complete cleanup from the private transaction directory. Use the original device and keep Wallet and Books closed.", vec!["card-recover".into(), self.journal.trim().into(), "--grappa-token".into(), self.token.trim().into(), "--timeout".into(), "25".into()]);
            }
        });
        ui.label(RichText::new("Keep backups until you no longer need Restore. Keep interrupted recovery directories until recovery succeeds.").small().weak());
    }
    fn help(&self, ui: &mut egui::Ui) {
        title(ui, "AirCard Linux", "Wallet card artwork / version 0.1.0");
        for (heading, body) in [
            (
                "Prepare",
                "Open an image, inspect the centered 1536 × 969 crop, then select Apply & restore.",
            ),
            (
                "Apply",
                "Select your paired iPhone, scan the intended Wallet card and choose a new private backup and recovery directory. Review the target and confirm.",
            ),
            (
                "Recover",
                "Cancellation attempts restoration. If a transaction fails or the device disconnects, retain its directory and run Recover on the same device before another operation.",
            ),
            (
                "Compatibility",
                "USB was tested on one iOS 27.0 device. Wi-Fi and other iOS versions still need hardware acceptance. Pairing and trust must already be established.",
            ),
            (
                "Installation",
                "Keep aircard and aircard-gui together. See docs/INSTALL.md for Debian/Ubuntu, Arch/Manjaro and Fedora dependencies. You can type paths if the desktop file picker is unavailable.",
            ),
            (
                "Privacy",
                "Backups stay on this computer. Copy details redacts device and card identifiers. No Apple libraries, pairing records or token table are distributed.",
            ),
        ] {
            ui.add_space(12.0);
            ui.label(RichText::new(heading).strong());
            ui.label(body);
        }
    }
    fn confirmation(&mut self, ctx: &egui::Context) {
        let Some(mut c) = self.pending.take() else {
            return;
        };
        let mut dismiss = false;
        let mut apply = false;
        egui::Window::new("Confirm device operation")
            .id(egui::Id::new("confirm-operation"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(440.0)
            .show(ctx, |ui| {
                ui.heading(&c.title);
                ui.label(RichText::new(c.device.label()).strong());
                ui.label(format!("iOS {} / Paired", c.device.ios));
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("UDID: {}", c.device.udid))
                            .monospace()
                            .small(),
                    )
                    .wrap(),
                );
                ui.add_space(8.0);
                ui.label(&c.summary);
                ui.add_space(8.0);
                for pair in c.args.windows(2) {
                    if ["--journal", "--backup", "--card-hash"].contains(&pair[0].as_str()) {
                        ui.label(format!("{}: {}", pair[0].trim_start_matches('-'), pair[1]));
                    }
                }
                if c.args.first().is_some_and(|s| s == "card-restore") {
                    ui.label(format!("Backup: {}", c.args[1]));
                }
                ui.checkbox(
                    &mut c.accepted,
                    "I confirm this device and the changes above.",
                );
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        dismiss = true;
                    }
                    if ui
                        .add_enabled(c.can_apply(), egui::Button::new("Confirm and run"))
                        .clicked()
                    {
                        apply = true;
                    }
                });
            });
        if apply {
            let mut args = c.args;
            args.push("--apply".into());
            args.extend(c.device.selector());
            self.start(&c.title, args);
        } else if !dismiss {
            self.pending = Some(c);
        }
    }
    fn smoke_before(&mut self, ctx: &egui::Context) {
        let Some(smoke) = &mut self.smoke else {
            return;
        };
        if smoke.phase >= SMOKE_NAMES.len() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if smoke.frames == 0 {
            smoke.started = Instant::now();
            let phase = smoke.phase;
            self.dark = matches!(phase, 1 | 4);
            self.tab = match phase {
                2 => Tab::Help,
                3 | 4 => Tab::Device,
                _ => Tab::Artwork,
            };
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(if phase == 5 {
                Vec2::new(700.0, 540.0)
            } else {
                Vec2::new(1080.0, 780.0)
            }));
            self.style(ctx);
            if phase == 4 {
                self.review("Apply card artwork", "Replace the selected card background and save its original artwork in a private backup. This is a UI fixture; no device operation will run.", vec!["card-apply".into(), "synthetic.png".into(), "--backup".into(), "card-backup.json".into(), "--journal".into(), "recovery".into(), "--card-hash".into(), "fixture-card-identifier".into()]);
            }
        }
    }
    fn smoke_after(&mut self, ctx: &egui::Context) {
        let Some(smoke) = &mut self.smoke else {
            return;
        };
        if smoke.phase >= SMOKE_NAMES.len() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        smoke.frames += 1;
        let screenshots = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| {
                    if let egui::Event::Screenshot { image, .. } = e {
                        Some(image.clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        });
        for screenshot in screenshots {
            let path = smoke.dir.join(format!("{}.png", SMOKE_NAMES[smoke.phase]));
            let rgba: Vec<_> = screenshot
                .pixels
                .iter()
                .flat_map(|p| p.to_array())
                .collect();
            let image = image::RgbaImage::from_raw(
                screenshot.size[0] as u32,
                screenshot.size[1] as u32,
                rgba,
            )
            .expect("screenshot dimensions");
            let bytes = aircard_core::assets::encode_png(&image::DynamicImage::ImageRgba8(image))
                .expect("screenshot PNG");
            cli::local::write_new(&path, &bytes).expect("new screenshot path");
            smoke.phase += 1;
            smoke.frames = 0;
            smoke.requested = false;
            self.pending = None;
            if smoke.phase == SMOKE_NAMES.len() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
        }
        if smoke.frames >= 8
            && smoke.started.elapsed() >= Duration::from_millis(600)
            && !smoke.requested
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            smoke.requested = true;
        }
        ctx.request_repaint_after(Duration::from_millis(40));
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        self.smoke_before(ctx);
        if ctx.input(|i| i.viewport().close_requested()) && self.busy() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_when_idle = true;
            if let Some(job) = &self.job {
                job.cancel.store(true, Ordering::Relaxed);
            }
            self.status = "Closing after the running task and recovery finish…".into();
        }
        if self.close_when_idle && !self.busy() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        egui::TopBottomPanel::top("header")
            .frame(
                egui::Frame::new()
                    .inner_margin(18)
                    .fill(ctx.style().visuals.panel_fill),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("AirCard").size(23.0).strong());
                    ui.label(RichText::new("LINUX / WALLET").small().weak());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.selectable_label(self.dark, "Dark").clicked() {
                            self.dark = true;
                            self.style(ctx);
                        }
                        if ui.selectable_label(!self.dark, "Light").clicked() {
                            self.dark = false;
                            self.style(ctx);
                        }
                    });
                });
            });
        egui::TopBottomPanel::bottom("status")
            .frame(
                egui::Frame::new()
                    .inner_margin(14)
                    .fill(ctx.style().visuals.panel_fill),
            )
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if self.busy() {
                        ui.spinner();
                    }
                    ui.label(RichText::new(&self.stage).strong());
                    if let Some(job) = &self.job
                        && ui.button("Cancel task").clicked()
                    {
                        job.cancel.store(true, Ordering::Relaxed);
                    }
                    if ui.button("Details").clicked() {
                        self.details = !self.details;
                    }
                    if ui.button("Copy details").clicked() {
                        ctx.copy_text(self.logs.iter().cloned().collect::<Vec<_>>().join("\n"));
                    }
                });
                ui.label(RichText::new(&self.status).color(if self.failed {
                    ctx.style().visuals.error_fg_color
                } else {
                    ctx.style().visuals.text_color()
                }));
            });
        egui::SidePanel::left("navigation")
            .exact_width(158.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .inner_margin(16)
                    .fill(ctx.style().visuals.panel_fill),
            )
            .show(ctx, |ui| {
                for (tab, label) in [
                    (Tab::Artwork, "Card artwork"),
                    (Tab::Device, "Apply & restore"),
                    (Tab::Help, "Help"),
                ] {
                    ui.add_enabled_ui(self.pending.is_none(), |ui| {
                        ui.selectable_value(&mut self.tab, tab, label);
                    });
                }
                ui.add_space(28.0);
                ui.label(RichText::new("CONNECTION").small().weak());
                if let Some(d) = self.chosen() {
                    ui.label(d.route.to_uppercase());
                    ui.label(format!("iOS {}", d.ios));
                    ui.label(if d.paired {
                        "Paired"
                    } else {
                        "Pairing required"
                    });
                } else {
                    ui.label("No selection");
                }
                if self.smoke.is_some() {
                    ui.add_space(24.0);
                    ui.label(RichText::new("Synthetic UI test").small().weak());
                }
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .inner_margin(24)
                    .fill(ctx.style().visuals.window_fill),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_enabled_ui(!self.busy() && self.pending.is_none(), |ui| match self
                            .tab
                        {
                            Tab::Artwork => self.artwork(ui),
                            Tab::Device => self.device(ui),
                            Tab::Help => self.help(ui),
                        });
                    });
            });
        self.confirmation(ctx);
        if self.details {
            egui::Window::new("Operation details / identifiers redacted")
                .open(&mut self.details)
                .default_size([680.0, 340.0])
                .show(ctx, |ui| {
                    egui::ScrollArea::both()
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            for line in &self.logs {
                                ui.monospace(line);
                            }
                        });
                });
        }
        if self.busy() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        self.smoke_after(ctx);
    }
}
impl Drop for App {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
}
fn title(ui: &mut egui::Ui, heading: &str, subtitle: &str) {
    ui.heading(heading);
    ui.label(RichText::new(subtitle).weak());
    ui.add_space(16.0);
}
fn empty_preview(ui: &mut egui::Ui, heading: &str, description: &str) {
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .corner_radius(16)
        .inner_margin(28)
        .show(ui, |ui| {
            ui.set_min_size(Vec2::new(ui.available_width().min(500.0), 170.0));
            ui.add_space(30.0);
            ui.label(RichText::new(heading).size(22.0));
            ui.label(RichText::new(description).weak());
        });
}
