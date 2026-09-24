//! Wallet artwork UI with supervised native transactions (MIT).
use crate::{
    model::{
        Confirmation, Device, Discovery, error_message, preferred_device, progress_text, redacted,
    },
    storage::{Paths, Storage},
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
    Advanced,
    Help,
}
#[derive(Clone, Copy)]
enum Field {
    CardInput,
    CardOutput,
    Snapshot,
    BackupOutput,
    Recovery,
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
const SMOKE_NAMES: [&str; 16] = [
    "artwork-light",
    "artwork-dark",
    "help-light",
    "advanced-light",
    "confirmation-dark",
    "compact-light",
    "listening-light",
    "no-card-dark",
    "advanced-compact-light",
    "save-locations-dark",
    "restore-light",
    "save-locations-compact",
    "preview-compact",
    "empty-light",
    "empty-dark",
    "setup-compact",
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
    discovery: Discovery,
    devices: Vec<Device>,
    selected: Option<usize>,
    route: String,
    route_auto: bool,
    device_check: Option<bool>,
    cli_verified: bool,
    snapshot: String,
    backup_output: String,
    recovery: String,
    automatic_paths: bool,
    storage: Option<Storage>,
    storage_selection: Option<(String, String)>,
    planned_paths: Option<Paths>,
    active_backup: Option<String>,
    active_recovery: Option<String>,
    token: String,
    token_attempted: bool,
    journal: String,
    job: Option<worker::Job>,
    job_name: String,
    job_error: Option<String>,
    job_error_stage: Option<String>,
    job_writes: bool,
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
            app.backup_output = "local-data/aircard/operations/new/backup.json".into();
            app.card_hash = "fixture-card-identifier".into();
            app.token = "private-token.bin".into();
            app.journal = "recovery.json".into();
            app.stage = "Ready".into();
            app.status = "UI verification with synthetic data. Device access disabled.".into();
        } else {
            app.refresh();
        }
        if app.smoke.is_none() {
            match Storage::default() {
                Ok(storage) => app.storage = Some(storage),
                Err(message) => {
                    app.status = message;
                    app.automatic_paths = false;
                }
            }
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
            discovery: Discovery::default(),
            devices: vec![],
            selected: None,
            route: "usb".into(),
            route_auto: true,
            device_check: None,
            cli_verified: false,
            snapshot: String::new(),
            backup_output: String::new(),
            recovery: String::new(),
            automatic_paths: true,
            storage: None,
            storage_selection: None,
            planned_paths: None,
            active_backup: None,
            active_recovery: None,
            token: cli::sync_token::cached()
                .map_or_else(String::new, |p| p.to_string_lossy().into_owned()),
            token_attempted: false,
            journal: String::new(),
            job: None,
            job_name: String::new(),
            job_error: None,
            job_error_stage: None,
            job_writes: false,
            local: None,
            pending: None,
            restore_review: None,
            stage: "Idle".into(),
            status: "Prepare artwork and select your iPhone. Card detection and sync token setup are automatic.".into(),
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
        for widget in [
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
            &mut visuals.widgets.noninteractive,
        ] {
            widget.corner_radius = egui::CornerRadius::same(8);
        }
        ctx.set_visuals(visuals);
        ctx.style_mut(|s| {
            s.spacing.item_spacing = Vec2::new(10.0, 10.0);
            s.spacing.button_padding = Vec2::new(14.0, 8.0);
            s.text_styles
                .insert(egui::TextStyle::Body, egui::FontId::proportional(15.0));
            s.text_styles
                .insert(egui::TextStyle::Small, egui::FontId::proportional(13.0));
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
            .filter(|d| self.route_auto || d.route == self.route)
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
        self.job_error = None;
        self.job_error_stage = None;
        self.status = format!("{name} is running.");
        self.stage = name.into();
        self.job_name = name.into();
        self.job_writes = args.iter().any(|a| a == "--apply");
        if self.job_writes {
            self.active_backup = args
                .windows(2)
                .find(|p| p[0] == "--backup")
                .map(|p| p[1].clone());
            self.active_recovery = if args.first().is_some_and(|s| s == "card-recover") {
                args.get(1).cloned()
            } else {
                args.windows(2)
                    .find(|p| p[0] == "--journal")
                    .map(|p| p[1].clone())
            };
        }
        self.job = Some(worker::start(self.cli.clone(), args));
    }
    fn renew_save_paths(&mut self) {
        if self.automatic_paths
            && let (Some(storage), Some(device)) = (&self.storage, self.chosen())
        {
            let paths = storage.fresh(
                &device.udid,
                (!self.card_hash.is_empty()).then_some(self.card_hash.as_str()),
            );
            self.backup_output = paths.backup.to_string_lossy().into_owned();
            self.journal = paths.recovery.to_string_lossy().into_owned();
            self.planned_paths = Some(paths);
        }
    }
    fn sync_saved_paths(&mut self) {
        let selection = self.chosen().map(|d| (d.udid, self.card_hash.clone()));
        if selection == self.storage_selection || self.storage.is_none() {
            return;
        }
        self.storage_selection = selection.clone();
        self.snapshot.clear();
        self.recovery.clear();
        if let Some((device, card)) = selection {
            let (backup, recovery) = self
                .storage
                .as_ref()
                .unwrap()
                .recent(&device, (!card.is_empty()).then_some(card.as_str()));
            self.snapshot = backup.map_or_else(String::new, |p| p.to_string_lossy().into_owned());
            self.recovery = recovery.map_or_else(String::new, |p| p.to_string_lossy().into_owned());
            self.renew_save_paths();
        }
    }
    fn finish_save_paths(&mut self) {
        if !self.job_writes {
            return;
        }
        if let Some(path) = self.active_recovery.take() {
            if PathBuf::from(&path).is_dir() {
                self.recovery = path;
            } else if self.recovery == path {
                self.recovery.clear();
            }
        }
        self.active_backup = None;
        self.renew_save_paths();
    }
    fn refresh(&mut self) {
        if self.busy() {
            return;
        }
        self.cards.clear();
        self.card_hash.clear();
        self.discovery = Discovery::default();
        self.device_check = None;
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
    fn detect_card(&mut self) {
        if self.busy() || self.smoke.is_some() || !self.chosen().is_some_and(|d| d.paired) {
            return;
        }
        self.cards.clear();
        self.card_hash.clear();
        self.discovery.begin();
        self.device_job(
            "Detect Wallet card",
            vec![
                "scan".into(),
                "--duration".into(),
                "60".into(),
                "--show-hashes".into(),
                "--until-match".into(),
            ],
        );
        self.status =
            "Connecting to iPhone logs. Wait for the prompt before opening your card.".into();
    }
    fn setup_token(&mut self) {
        self.token_attempted = true;
        self.start("Set up sync token", vec!["setup-token".into()]);
    }
    fn auto_setup(&mut self) {
        if self.tab == Tab::Artwork
            && !self.busy()
            && self.pending.is_none()
            && self.smoke.is_none()
            && !self.close_when_idle
            && self.device_check != Some(false)
            && self.chosen().is_some_and(|d| d.paired)
        {
            if !self.discovery.started {
                self.detect_card();
            } else if !self.card_hash.is_empty() && self.token.is_empty() && !self.token_attempted {
                self.setup_token();
            }
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
            let mut dialog = rfd::FileDialog::new();
            if matches!(field, Field::CardInput) {
                dialog = dialog.add_filter("Artwork", &["png", "jpg", "jpeg", "webp"]);
            }
            let path = if matches!(field, Field::Recovery) {
                dialog.pick_folder()
            } else if save {
                dialog.save_file()
            } else {
                dialog.pick_file()
            };
            Ok(LocalResult::Picked(field, path))
        });
    }
    fn prepare_artwork(&mut self) {
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
    fn field(&mut self, field: Field) -> &mut String {
        match field {
            Field::CardInput => &mut self.card_input,
            Field::CardOutput => &mut self.card_output,
            Field::Snapshot => &mut self.snapshot,
            Field::BackupOutput => &mut self.backup_output,
            Field::Recovery => &mut self.recovery,
            Field::Token => &mut self.token,
        }
    }
    fn path_row(&mut self, ui: &mut egui::Ui, label: &str, field: Field, save: bool) {
        ui.label(RichText::new(label).strong());
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 95.0).max(120.0);
            let response = ui.add_sized(
                [width, 32.0],
                egui::TextEdit::singleline(self.field(field)).hint_text("Path"),
            );
            if response.changed() {
                self.invalidate(field);
            }
            if ui.button("Browse").clicked() {
                self.picker(field, save);
            } else if matches!(field, Field::CardInput)
                && response.lost_focus()
                && !self.card_input.trim().is_empty()
                && self.card.is_none()
            {
                self.prepare_artwork();
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
                    if v["event"] == "cli_ready" {
                        self.cli_verified = true;
                    }
                    if self.job_name == "Detect Wallet card" {
                        self.discovery.event(&v);
                        if self.discovery.listening {
                            self.stage = "Waiting for Wallet".into();
                            self.status = format!(
                                "Now open Wallet on your iPhone and tap the intended card. Detection ends automatically when an identifier appears ({} s left).",
                                self.discovery.remaining
                            );
                        }
                    }
                    if v["event"] == "token_ready"
                        && let Some(path) = v["path"].as_str()
                    {
                        self.token = path.into();
                    }
                    if v["event"] == "card_backup_saved"
                        && let Some(path) = &self.active_backup
                    {
                        self.snapshot = path.clone();
                    }
                    if (v["event"] == "error" || v["failure"].is_object())
                        && let Some(message) = error_message(&v)
                        && self.job_error.is_none()
                    {
                        self.job_error = Some(message);
                        self.job_error_stage = Some(self.stage.clone());
                    }
                    if let Some(d) = Device::from_event(&v) {
                        self.devices.push(d);
                    }
                    if v["event"] == "devices_complete" {
                        self.selected = preferred_device(
                            &self.devices,
                            (!self.route_auto).then_some(self.route.as_str()),
                        );
                    }
                    if v["event"] == "probe_complete" {
                        self.device_check = Some(true);
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
                    if let Some(stage) = progress_text(&v) {
                        self.stage = stage.into();
                        if !self.discovery.listening {
                            self.status = format!("{stage}…");
                        }
                    }
                    self.log(redacted(&v).to_string());
                }
                Message::Finished(result) => {
                    let cancelled = self
                        .job
                        .as_ref()
                        .is_some_and(|job| job.cancel.load(Ordering::Relaxed));
                    self.job = None;
                    match result {
                        Ok(()) => {
                            self.failed = false;
                            self.status = format!("{} completed.", self.job_name);
                            self.stage = "Complete".into();
                            if self.job_name == "Detect Wallet card" {
                                if self.cards.len() == 1 && !self.discovery.cancelled {
                                    self.card_hash = self.cards[0].clone();
                                }
                                self.status = self.discovery.result(self.cards.len());
                                self.stage = if self.discovery.cancelled {
                                    "Detection stopped"
                                } else if self.cards.is_empty() {
                                    "No card detected"
                                } else {
                                    "Card detected"
                                }
                                .into();
                            } else if self.job_name == "Set up sync token" {
                                self.status = "Sync token ready and selected. It will be reused next time. Close Wallet and Books before applying.".into();
                            } else if self.job_name == "Check device" {
                                self.status = "Trust and file access verified. Sync authentication and artwork compatibility are checked during the operation.".into();
                            } else if self.job_name == "Discover devices" && self.chosen().is_none()
                            {
                                self.status = if self.devices.is_empty() {
                                    "No iPhone found. Connect by USB, unlock it and refresh devices. Help includes connection diagnostics."
                                } else {
                                    "Choose your iPhone and connection. If trust is missing, pair it with this computer first."
                                }.into();
                            }
                            if let Some(c) = self.restore_review.take() {
                                self.pending = Some(c);
                            }
                        }
                        Err(e) => {
                            self.failed = true;
                            self.status = self.job_error.take().unwrap_or(e);
                            if let Some(stage) = self.job_error_stage.take() {
                                self.status = format!("Stopped during {stage}. {}", self.status);
                            }
                            if self.job_name == "Check device" {
                                self.device_check = Some(false);
                            }
                            if self.job_writes
                                && self
                                    .active_recovery
                                    .as_ref()
                                    .is_some_and(|p| PathBuf::from(p).is_dir())
                            {
                                self.status.push_str(" Keep this recovery directory. Reconnect the original iPhone, close Wallet and Books, then use Recover before another Apply.");
                            }
                            if self.job_name == "Detect Wallet card" {
                                if cancelled {
                                    self.discovery.cancelled = true;
                                    self.status = self.discovery.result(self.cards.len());
                                } else if self.discovery.finished && self.discovery.lines == 0 {
                                    self.status = self.discovery.result(0);
                                }
                                self.discovery.finished = true;
                                self.discovery.listening = false;
                                self.discovery.failure = (!cancelled).then(|| self.status.clone());
                                self.stage = "Detection stopped".into();
                            }
                            self.restore_review = None;
                            if self.job_name == "Validate backup" {
                                self.renew_save_paths();
                            }
                        }
                    }
                    self.finish_save_paths();
                }
            }
        }
        if disconnected && self.job.is_some() {
            self.job = None;
            self.failed = true;
            self.status =
                "CLI monitor stopped unexpectedly. Inspect recovery status before retrying.".into();
            self.finish_save_paths();
            if self.restore_review.take().is_some() {
                self.renew_save_paths();
            }
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
                    self.status =
                        "Artwork ready. Check the preview, card and device before applying.".into();
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
                    if matches!(field, Field::CardInput) {
                        self.prepare_artwork();
                    }
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
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        if ui.available_width() >= 760.0 {
            surface_row(ui, |left, right| {
                self.card_configuration(left);
                self.card_preview(right);
            });
        } else {
            surface(ui, |ui| self.card_configuration(ui));
            ui.add_space(10.0);
            surface(ui, |ui| self.card_preview(ui));
        }
    }
    fn card_configuration(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Card Configuration").size(21.0));
        ui.label(RichText::new("Choose a Wallet card and its replacement artwork.").weak());
        ui.add_space(12.0);
        self.card_selector(ui);
        ui.add_space(12.0);
        self.path_row(ui, "Select artwork", Field::CardInput, false);
        ui.label(
            RichText::new("PNG, JPEG or WebP / preview updates automatically")
                .small()
                .weak(),
        );
        ui.add_space(12.0);
        self.device_selector(ui);
        self.sync_saved_paths();
        ui.add_space(16.0);
        self.apply_artwork(ui);
    }
    fn card_preview(&self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Artwork Preview").size(21.0));
        ui.label(RichText::new("1536 × 969 px / centered crop").weak());
        ui.add_space(12.0);
        let width = ui.available_width();
        let size = Vec2::new(width, width * 969.0 / 1536.0);
        if let Some(texture) = &self.card_texture {
            ui.add(
                egui::Image::new(texture)
                    .fit_to_exact_size(size)
                    .corner_radius(16),
            );
        } else {
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            ui.painter()
                .rect_filled(rect, 16, ui.visuals().faint_bg_color);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "No artwork loaded",
                egui::FontId::proportional(18.0),
                ui.visuals().weak_text_color(),
            );
        }
        ui.add_space(12.0);
        ui.label(
            RichText::new("After applying, close Apple Wallet and reopen it.")
                .small()
                .weak(),
        );
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
        ui.label(
            RichText::new("Exports @3x.png, @2x.png and PDF artwork from the selected image.")
                .small()
                .weak(),
        );
    }
    fn export(&mut self, resources: Vec<Resource>, output: PathBuf) {
        self.local_job("Export resources", move || {
            let bytes = resources_zip(&resources).map_err(|e| e.to_string())?;
            cli::local::write_new(&output, &bytes).map_err(|e| e.to_string())?;
            Ok(LocalResult::Saved)
        });
    }
    fn device_selector(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Select device").strong());
        let previous = self.chosen();
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("device-select")
                .width((ui.available_width() - 100.0).max(120.0))
                .wrap_mode(egui::TextWrapMode::Truncate)
                .selected_text(
                    self.chosen()
                        .map_or_else(|| "Choose a device".into(), |d| d.label()),
                )
                .show_ui(ui, |ui| {
                    for (i, d) in self
                        .devices
                        .iter()
                        .enumerate()
                        .filter(|(_, d)| self.route_auto || d.route == self.route)
                    {
                        ui.selectable_value(&mut self.selected, Some(i), d.label());
                    }
                });
            if ui.button("Refresh").clicked() {
                self.refresh();
            }
        });
        self.device_selection_changed(previous);
        if let Some(d) = self.chosen() {
            ui.label(
                RichText::new(format!(
                    "iOS {} / {}",
                    d.ios,
                    if d.paired { "Paired" } else { &d.status }
                ))
                .small()
                .weak(),
            );
        } else {
            ui.label(
                RichText::new("Connect and unlock your iPhone, then refresh.")
                    .small()
                    .weak(),
            );
        }
    }
    fn device_selection_changed(&mut self, previous: Option<Device>) {
        if previous != self.chosen() {
            self.cards.clear();
            self.card_hash.clear();
            self.discovery = Discovery::default();
            self.device_check = None;
        }
    }
    fn card_selector(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Select cards").strong());
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("card-select")
                .width((ui.available_width() - 100.0).max(120.0))
                .wrap_mode(egui::TextWrapMode::Truncate)
                .selected_text(if self.card_hash.is_empty() {
                    if self.cards.is_empty() {
                        "Waiting for a card"
                    } else {
                        "Choose detected card"
                    }
                } else {
                    &self.card_hash
                })
                .show_ui(ui, |ui| {
                    for hash in &self.cards {
                        ui.selectable_value(&mut self.card_hash, hash.clone(), hash);
                    }
                });
            if ui
                .add_enabled(
                    self.chosen().is_some_and(|d| d.paired),
                    egui::Button::new("Scan"),
                )
                .on_hover_text("Detect card again")
                .clicked()
            {
                self.detect_card();
            }
        });
        ui.label(
            RichText::new(if self.discovery.listening {
                "Open Wallet and tap the intended card now."
            } else if !self.card_hash.is_empty() {
                "Check the selected card, then close Wallet before applying."
            } else if self.discovery.finished {
                "No card selected. Scan again and open the intended card in Wallet."
            } else {
                "When prompted, open Wallet and tap the intended card."
            })
            .small()
            .weak(),
        );
    }
    fn connection_options(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Connection").strong());
        let previous = self.chosen();
        let previous_auto = self.route_auto;
        ui.horizontal_wrapped(|ui| {
            if ui.selectable_label(self.route_auto, "Automatic").clicked() {
                self.route_auto = true;
            }
            for (route, label) in [("usb", "USB"), ("wifi", "Wi-Fi")] {
                if ui
                    .selectable_label(!self.route_auto && self.route == route, label)
                    .clicked()
                {
                    self.route_auto = false;
                    self.route = route.into();
                }
            }
            if ui
                .add_enabled(self.chosen().is_some(), egui::Button::new("Check device"))
                .clicked()
            {
                self.device_job("Check device", vec!["probe".into()]);
            }
        });
        if previous_auto != self.route_auto || self.chosen().is_none() {
            self.selected = preferred_device(
                &self.devices,
                (!self.route_auto).then_some(self.route.as_str()),
            );
        }
        self.device_selection_changed(previous);
        self.device_selector(ui);
        self.card_selector(ui);
        if let Some(ok) = self.device_check {
            ui.label(if ok { "Trust and file access checked. Sync compatibility is verified during the operation." }
                else { "Device check failed. Follow the status guidance below before applying." });
        }
    }
    fn operation_connected(&self) -> bool {
        self.chosen().is_some_and(|d| d.paired)
            && self.device_check != Some(false)
            && !self.token.trim().is_empty()
    }
    fn recovery_pending(&self) -> bool {
        !self.recovery.is_empty() && PathBuf::from(&self.recovery).is_dir()
    }
    fn operation_args(&self) -> Vec<String> {
        vec![
            "--journal".into(),
            self.journal.trim().into(),
            "--grappa-token".into(),
            self.token.trim().into(),
            "--timeout".into(),
            "25".into(),
        ]
    }
    fn apply_artwork(&mut self, ui: &mut egui::Ui) {
        let recovery_pending = self.recovery_pending();
        let ready =
            self.operation_connected() && !self.journal.trim().is_empty() && !recovery_pending;
        if ui
            .add_enabled(
                ready
                    && self.card.is_some()
                    && !self.card_hash.is_empty()
                    && !self.backup_output.trim().is_empty(),
                egui::Button::new(RichText::new("Apply Artwork…").color(Color32::WHITE))
                    .fill(ui.visuals().selection.bg_fill)
                    .min_size(Vec2::new(ui.available_width(), 40.0)),
            )
            .clicked()
        {
            let mut args = vec![
                "card-apply".into(),
                self.card_input.trim().into(),
                "--card-hash".into(),
                self.card_hash.clone(),
                "--backup".into(),
                self.backup_output.trim().into(),
            ];
            args.extend([
                "--expected-artwork-sha256".into(),
                aircard_core::sha256(&self.card.as_ref().unwrap().png),
            ]);
            args.extend(self.operation_args());
            self.review("Apply card artwork", "Replace this card's background and save its original artwork in the backup below. Keep Wallet and Books closed during the operation. Reopen Wallet when complete.", args);
        }
        if recovery_pending {
            ui.label("An unfinished operation was found. Open Advanced Options and recover on the original iPhone before applying another change.");
        }
    }
    fn advanced(&mut self, ui: &mut egui::Ui) {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        title(
            ui,
            "Advanced Options",
            "Manage backups, recovery and sync token setup.",
        );
        self.sync_saved_paths();
        if ui.available_width() >= 760.0 {
            surface_row(ui, |left, right| {
                self.backup_options(left);
                self.advanced_setup(right);
            });
        } else {
            surface(ui, |ui| self.backup_options(ui));
            ui.add_space(10.0);
            surface(ui, |ui| self.advanced_setup(ui));
        }
    }
    fn advanced_setup(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Sync token").strong());
        ui.label(if self.token.is_empty() {
            "The built-in token is saved privately on this computer after a card is selected. Setup works offline."
        } else {
            "Token selected. Saved setup tokens are reused on future launches."
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button("Set up sync token").clicked() {
                self.setup_token();
            }
            ui.hyperlink_to(
                "Token setup help",
                "https://github.com/hoicau/AirCard-Linux/blob/main/docs/SYNC-TOKEN.md",
            );
        });
        ui.collapsing("Use an existing token / show path", |ui| {
            self.path_row(ui, "Private 84-byte token file", Field::Token, false);
        });
        ui.add_space(16.0);
        self.connection_options(ui);
        self.sync_saved_paths();
        ui.add_space(16.0);
        ui.collapsing("Export artwork", |ui| self.card_exports(ui));
    }
    fn backup_options(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Backup & recovery").strong());
        ui.label("Your original card artwork is saved automatically so you can restore it later. Recovery files protect you if an operation is interrupted.");
        egui::CollapsingHeader::new("Advanced save locations")
            .open(self.smoke.as_ref().map(|s| matches!(s.phase, 9 | 11)))
            .show(ui, |ui| {
            if ui.checkbox(&mut self.automatic_paths, "Choose save locations automatically").changed() {
                self.renew_save_paths();
            }
            ui.label("Backups are grouped by iPhone and card. Each operation gets a fresh folder; previous backups are kept.");
            ui.add_enabled_ui(!self.automatic_paths, |ui| {
                self.path_row(ui, "New backup file for Apply", Field::BackupOutput, true);
                ui.label("New recovery folder for Apply or Restore");
                ui.add(egui::TextEdit::singleline(&mut self.journal).desired_width(f32::INFINITY).hint_text("Path"));
            });
            ui.label("These paths must be unused. AirCard creates the backup file and recovery folder when you confirm the operation.");
        });
        let connected = self.operation_connected();
        let recovery_pending = self.recovery_pending();
        let ready = connected && !self.journal.trim().is_empty() && !recovery_pending;
        let common = self.operation_args();
        egui::CollapsingHeader::new("Restore or recover")
            .default_open(true)
            .open(if recovery_pending { Some(true) } else { self.smoke.as_ref().map(|s| s.phase != 9 && s.phase != 11) })
            .show(ui, |ui| {
                ui.label("Restore returns a card to its saved artwork. Select a card to find its latest backup, or Browse for an older or imported backup.");
                self.path_row(ui, "Existing backup to restore", Field::Snapshot, false);
                if ui.add_enabled(ready && !self.snapshot.trim().is_empty(), egui::Button::new("Review restore…")).clicked() {
                    // An imported backup can belong to a different card from the current selection.
                    // Restore journals therefore use a separate device-level scope.
                    let mut common = common;
                    if self.automatic_paths && let (Some(storage), Some(device)) = (&self.storage, self.chosen()) {
                        let paths = storage.fresh(&device.udid, None);
                        self.journal = paths.recovery.to_string_lossy().into_owned();
                        common[1] = self.journal.clone();
                        self.planned_paths = Some(paths);
                    }
                    let mut args = vec!["card-restore".into(), self.snapshot.trim().into()]; args.extend(common);
                    self.restore_review = self.chosen().map(|device| Confirmation { title: "Restore card artwork".into(), summary: "Restore the card saved in this backup on its original device. Keep Wallet and Books closed.".into(), device, args: args.clone(), accepted:false });
                    self.start("Validate backup", args);
                }
                ui.add_space(8.0);
                ui.label("Recover finishes cleanup or restores originals after an interruption. Its existing recovery folder is selected automatically for operations saved here.");
                self.path_row(ui, "Existing recovery folder", Field::Recovery, false);
                if ui.add_enabled(connected && !self.recovery.trim().is_empty(), egui::Button::new("Review recovery…")).clicked() {
                    self.review("Recover interrupted card operation", "Restore originals or finish cleanup from this interrupted operation. Use the original iPhone and keep Wallet and Books closed.", vec!["card-recover".into(), self.recovery.trim().into(), "--grappa-token".into(), self.token.trim().into(), "--timeout".into(), "25".into()]);
                }
                ui.label("Keep backups until you no longer need Restore. Recovery folders are removed automatically after successful cleanup.");
            });
    }

    fn help(&self, ui: &mut egui::Ui) {
        title(
            ui,
            "AirCard Linux",
            concat!("Wallet card artwork / version ", env!("CARGO_PKG_VERSION")),
        );
        ui.label(if self.cli_verified {
            "Matching CLI version verified."
        } else {
            "CLI version is checked before every device or setup task."
        });
        ui.collapsing("Connection diagnostics", |ui| {
            ui.label("Use Check device in Advanced Options to verify trust and file access without changing the phone.");
            ui.label("If USB is missing, install usbmuxd and libimobiledevice with your distribution's package manager, then inspect the service:");
            ui.monospace("systemctl status usbmuxd --no-pager");
            ui.label("If Browse does not open, install your desktop's xdg-desktop-portal backend. You can also enter paths directly.");
            ui.monospace("systemctl --user status xdg-desktop-portal --no-pager");
            ui.hyperlink_to("Distribution install commands", "https://github.com/hoicau/AirCard-Linux/blob/main/docs/INSTALL.md");
        });
        for (heading, body) in [
            (
                "Prepare",
                "In Apply Artwork, open an image and inspect the centered 1536 × 969 crop.",
            ),
            (
                "Apply",
                "Select your paired iPhone. Detection starts automatically; wait for the prompt, then open Wallet and tap the intended card. A single detected identifier is filled in for review. Backup and recovery locations are chosen automatically; review them before confirming.",
            ),
            (
                "Sync token",
                "After a card is selected, AirCard saves its built-in compatibility token privately for reuse. Setup works offline. You can also choose an existing 84-byte token. No Apple Account login is needed. Compatibility still depends on iOS.",
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
                "Backups stay on this computer. Copy details redacts device and card identifiers. No Apple libraries or pairing records are distributed.",
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
                ui.set_max_width(440.0);
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
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
            if self.automatic_paths
                && c.args.first().is_some_and(|s| s != "card-recover")
                && self.smoke.is_none()
            {
                let prepared = match (&self.storage, &self.planned_paths) {
                    (Some(storage), Some(paths)) => storage.prepare(&c.device.udid, paths),
                    _ => Err("Automatic save locations are unavailable. Choose custom locations under Advanced save locations.".into()),
                };
                if let Err(message) = prepared {
                    self.status = message;
                    self.failed = true;
                    self.renew_save_paths();
                    return;
                }
            }
            let mut args = c.args;
            args.push("--apply".into());
            args.extend(c.device.selector());
            self.start(&c.title, args);
        } else if !dismiss {
            self.pending = Some(c);
        } else {
            self.renew_save_paths();
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
            self.dark = matches!(phase, 1 | 4 | 7 | 9 | 14);
            self.tab = match phase {
                2 => Tab::Help,
                3 | 8..=11 | 15 => Tab::Advanced,
                _ => Tab::Artwork,
            };
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(
                if matches!(phase, 5 | 8 | 11 | 12 | 15) {
                    Vec2::new(680.0, 520.0)
                } else {
                    Vec2::new(1080.0, 780.0)
                },
            ));
            self.style(ctx);
            if phase == 6 {
                self.discovery.begin();
                self.discovery.listening = true;
                self.discovery.remaining = 45;
                self.card_hash.clear();
                self.token.clear();
                self.stage = "Waiting for Wallet".into();
                self.status = "Now open Wallet on your iPhone and tap the intended card. Detection ends automatically when an identifier appears (45 s left).".into();
            }
            if phase == 7 {
                self.discovery.listening = false;
                self.discovery.finished = true;
                self.discovery.lines = 100;
                self.stage = "No card detected".into();
                self.status = self.discovery.result(0);
            }
            if phase >= 9 {
                self.discovery = Discovery::default();
                self.card_hash = if self.devices.is_empty() {
                    String::new()
                } else {
                    "fixture-card-identifier".into()
                };
                self.token = "private-token.bin".into();
                self.stage = "Ready".into();
                self.status = "UI verification with synthetic data. Device access disabled.".into();
            }
            if matches!(phase, 13 | 14) {
                self.card = None;
                self.card_texture = None;
                self.card_input.clear();
                self.devices.clear();
                self.selected = None;
                self.card_hash.clear();
            }
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
        self.auto_setup();
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
                ui.horizontal_wrapped(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("AirCard").size(23.0).strong());
                        ui.label(
                            RichText::new(concat!("v", env!("CARGO_PKG_VERSION")))
                                .size(14.0)
                                .weak(),
                        );
                    });
                    ui.add_space(12.0);
                    ui.add_enabled_ui(self.pending.is_none(), |ui| {
                        ui.horizontal(|ui| {
                            for (tab, label) in [
                                (Tab::Artwork, "Apply Artwork"),
                                (Tab::Advanced, "Advanced Options"),
                                (Tab::Help, "Help"),
                            ] {
                                if ui
                                    .add(
                                        egui::Button::selectable(self.tab == tab, label)
                                            .corner_radius(16),
                                    )
                                    .clicked()
                                {
                                    self.tab = tab;
                                }
                            }
                        });
                    });
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
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .inner_margin(24)
                    .fill(if self.tab == Tab::Help {
                        ctx.style().visuals.window_fill
                    } else {
                        ctx.style().visuals.panel_fill
                    }),
            )
            .show(ctx, |ui| {
                let mut scroll = egui::ScrollArea::vertical()
                    .id_salt(("page", self.tab as u8))
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .auto_shrink([false, false]);
                if let Some(smoke) = &self.smoke {
                    scroll = scroll.vertical_scroll_offset(match smoke.phase {
                        11 => 180.0,
                        12 => 500.0,
                        15 => 600.0,
                        _ => 0.0,
                    });
                }
                scroll.show(ui, |ui| {
                    ui.add_enabled_ui(!self.busy() && self.pending.is_none(), |ui| {
                        match self.tab {
                            Tab::Artwork => self.artwork(ui),
                            Tab::Advanced => self.advanced(ui),
                            Tab::Help => self.help(ui),
                        }
                    });
                });
            });
        // Repaint once after navigation or a device change so automatic setup can start.
        if self.tab == Tab::Artwork
            && !self.busy()
            && !self.discovery.started
            && self.device_check != Some(false)
            && self.chosen().is_some_and(|d| d.paired)
            && self.smoke.is_none()
        {
            ctx.request_repaint();
        }
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
fn surface_frame(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(ui.visuals().window_fill)
        .corner_radius(20)
        .inner_margin(22)
}
fn surface(ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui)) {
    surface_frame(ui).show(ui, |ui| {
        ui.set_width(ui.available_width());
        contents(ui);
    });
}
fn surface_row(ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui, &mut egui::Ui)) {
    ui.columns(2, |columns| {
        let mut left = surface_frame(&columns[0]).begin(&mut columns[0]);
        let mut right = surface_frame(&columns[1]).begin(&mut columns[1]);
        left.content_ui.set_width(left.content_ui.available_width());
        right
            .content_ui
            .set_width(right.content_ui.available_width());
        contents(&mut left.content_ui, &mut right.content_ui);
        // Measure both contents before painting so their backgrounds match this frame.
        let bottom = left
            .content_ui
            .min_rect()
            .bottom()
            .max(right.content_ui.min_rect().bottom());
        left.content_ui.expand_to_include_y(bottom);
        right.content_ui.expand_to_include_y(bottom);
        left.end(&mut columns[0]);
        right.end(&mut columns[1]);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, atomic::AtomicBool};

    #[test]
    fn choosing_artwork_prepares_preview_and_failed_replacement_clears_it() {
        let root = std::env::temp_dir().join(format!("aircard-gui-artwork-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        struct Clean(PathBuf);
        impl Drop for Clean {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _clean = Clean(root.clone());
        let source = root.join("artwork.png");
        image::RgbaImage::from_pixel(80, 50, image::Rgba([20, 90, 160, 255]))
            .save(&source)
            .unwrap();
        let invalid = root.join("invalid.png");
        std::fs::write(&invalid, b"not an image").unwrap();
        let ctx = egui::Context::default();
        let mut app = App::empty(PathBuf::new(), false);
        for (path, valid) in [(source, true), (invalid, false)] {
            let (tx, rx) = mpsc::channel();
            app.local = Some(rx);
            tx.send(Ok(LocalResult::Picked(
                Field::CardInput,
                Some(path.clone()),
            )))
            .unwrap();
            app.poll(&ctx);
            assert_eq!(PathBuf::from(&app.card_input), path);
            assert!(app.card.is_none() && app.card_texture.is_none());
            assert!(app.busy(), "selection must start preparation automatically");
            let start = Instant::now();
            while app.busy() && start.elapsed() < Duration::from_secs(5) {
                app.poll(&ctx);
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(!app.busy(), "artwork preparation deadline");
            assert_eq!(app.card.is_some(), valid);
            assert_eq!(app.card_texture.is_some(), valid);
            assert_eq!(app.failed, !valid);
            assert!(app.pending.is_none(), "preparation must not apply artwork");
        }
    }

    #[test]
    fn automatic_paths_keep_restore_inputs_and_recover_interrupted_operations_after_restart() {
        let root = std::env::temp_dir().join(format!("aircard-gui-paths-{}", std::process::id()));
        struct Clean(PathBuf);
        impl Drop for Clean {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _clean = Clean(root.clone());
        let mut app = App::empty(PathBuf::new(), false);
        app.storage = Some(Storage::at(root.clone()));
        app.devices.push(Device {
            udid: "fixture-phone".into(),
            route: "usb".into(),
            ios: "fixture".into(),
            paired: true,
            status: "paired_session_verified".into(),
        });
        app.selected = Some(0);
        app.card_hash = "fixture-card-a".into();
        app.sync_saved_paths();
        assert!(!app.backup_output.is_empty() && !app.journal.is_empty());
        assert!(app.snapshot.is_empty() && app.recovery.is_empty());
        assert!(!root.exists());
        let paths = app.planned_paths.clone().unwrap();
        app.storage
            .as_ref()
            .unwrap()
            .prepare("fixture-phone", &paths)
            .unwrap();
        cli::local::write_new(&paths.backup, b"synthetic backup").unwrap();
        cli::local::create_private_directory(&paths.recovery).unwrap();
        app.job_writes = true;
        app.active_backup = Some(app.backup_output.clone());
        app.active_recovery = Some(app.journal.clone());
        let (tx, receiver) = mpsc::sync_channel(4);
        app.job = Some(worker::Job {
            receiver,
            cancel: Arc::new(AtomicBool::new(false)),
        });
        tx.send(Message::Event(
            serde_json::json!({"event":"card_backup_saved"}),
        ))
        .unwrap();
        tx.send(Message::Finished(Err("interrupted cleanup".into())))
            .unwrap();
        app.poll(&egui::Context::default());
        assert_eq!(PathBuf::from(&app.snapshot), paths.backup);
        assert_eq!(PathBuf::from(&app.recovery), paths.recovery);
        assert_ne!(app.snapshot, app.backup_output);
        assert_ne!(app.recovery, app.journal);
        let mut reopened = App::empty(PathBuf::new(), false);
        reopened.storage = Some(Storage::at(root));
        reopened.devices = app.devices.clone();
        reopened.selected = Some(0);
        reopened.card_hash = app.card_hash.clone();
        reopened.sync_saved_paths();
        assert_eq!(reopened.snapshot, app.snapshot);
        assert_eq!(reopened.recovery, app.recovery);
        reopened.card_hash = "fixture-card-b".into();
        reopened.sync_saved_paths();
        assert!(reopened.snapshot.is_empty());
        assert_eq!(
            reopened.recovery, app.recovery,
            "Another card must not hide an unfinished operation"
        );
        let other_paths = reopened.planned_paths.as_ref().unwrap();
        assert_ne!(
            paths.backup.parent().unwrap().parent(),
            other_paths.backup.parent().unwrap().parent()
        );
        reopened.card_hash = app.card_hash.clone();
        reopened.sync_saved_paths();
        assert_eq!(reopened.snapshot, app.snapshot);
        std::fs::remove_dir(&paths.recovery).unwrap();
        reopened.job_writes = true;
        reopened.active_recovery = Some(reopened.recovery.clone());
        reopened.finish_save_paths();
        assert!(reopened.recovery.is_empty());
        assert_eq!(reopened.snapshot, app.snapshot);
        reopened.devices[0].udid = "different-phone".into();
        reopened.sync_saved_paths();
        assert!(reopened.snapshot.is_empty() && reopened.recovery.is_empty());
    }

    #[test]
    fn automatic_setup_runs_discovery_then_token_through_verified_worker() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("aircard-gui-flow-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        struct Clean(PathBuf);
        impl Drop for Clean {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _clean = Clean(dir.clone());
        let binary = dir.join("aircard");
        let script = r##"#!/bin/sh
case "$1" in
--version) echo 'aircard @VERSION@' ;;
devices)
 echo '{"event":"device","udid":"fixture-phone","transport":"wifi","info":{"pairing":"paired_session_verified","ios_version":"fixture"}}'
 echo '{"event":"device","udid":"fixture-phone","transport":"usb","info":{"pairing":"paired_session_verified","ios_version":"fixture"}}'
 echo '{"event":"devices_complete","count":2}' ;;
scan)
 echo '{"event":"service_started","service":"com.apple.syslog_relay"}'
 echo '{"event":"card_match","hash":"AAoUHigyPEZQWmRueIKMlqCqtL4="}'
 echo '{"event":"syslog_complete","lines":1,"cancelled":false}' ;;
setup-token) echo '{"event":"token_ready","path":"fixture-token.bin"}' ;;
*) exit 9 ;;
esac
"##.replace("@VERSION@", env!("CARGO_PKG_VERSION"));
        std::fs::write(&binary, script).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut app = App::empty(binary, false);
        app.token.clear();
        app.refresh();
        let ctx = egui::Context::default();
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            app.poll(&ctx);
            app.auto_setup();
            if !app.busy() && app.token_attempted {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!app.busy(), "automatic setup deadline");
        assert!(!app.failed, "{}", app.status);
        assert_eq!(app.chosen().unwrap().route, "usb");
        assert!(app.cli_verified);
        assert_eq!(app.card_hash, "AAoUHigyPEZQWmRueIKMlqCqtL4=");
        assert_eq!(app.token, "fixture-token.bin");
        app.auto_setup();
        assert!(!app.busy(), "completed setup must not loop");
    }

    #[test]
    fn discovery_fills_only_a_single_successful_candidate() {
        let hashes = [
            "AAoUHigyPEZQWmRueIKMlqCqtL4=",
            "AAsWISw3Qk1YY255hI+apbC7xtE=",
        ];
        for (count, cancelled) in [(0, false), (1, false), (2, false), (1, true)] {
            let mut app = App::empty(PathBuf::new(), false);
            let (tx, receiver) = mpsc::sync_channel(16);
            app.job = Some(worker::Job {
                receiver,
                cancel: Arc::new(AtomicBool::new(false)),
            });
            app.job_name = "Detect Wallet card".into();
            app.discovery.begin();
            for hash in &hashes[..count] {
                tx.send(Message::Event(
                    serde_json::json!({"event":"card_match","hash":hash}),
                ))
                .unwrap();
            }
            tx.send(Message::Event(
                serde_json::json!({"event":"syslog_complete","lines":10,"cancelled":cancelled}),
            ))
            .unwrap();
            tx.send(Message::Finished(Ok(()))).unwrap();
            app.poll(&egui::Context::default());
            assert_eq!(app.cards.len(), count);
            assert_eq!(!app.card_hash.is_empty(), count == 1 && !cancelled);
            assert!(app.job.is_none());
        }
    }

    #[test]
    fn token_setup_error_keeps_actionable_hint_and_existing_selection() {
        let mut app = App::empty(PathBuf::new(), false);
        app.token = "existing-token.bin".into();
        let (tx, receiver) = mpsc::sync_channel(4);
        app.job = Some(worker::Job {
            receiver,
            cancel: Arc::new(AtomicBool::new(false)),
        });
        app.job_name = "Set up sync token".into();
        tx.send(Message::Event(
            serde_json::json!({"event":"error","hint":"Choose a writable path and retry setup."}),
        ))
        .unwrap();
        tx.send(Message::Finished(Err("exit 1".into()))).unwrap();
        app.poll(&egui::Context::default());
        assert_eq!(app.token, "existing-token.bin");
        assert!(
            app.status
                .ends_with("Choose a writable path and retry setup.")
        );
        assert!(app.failed);
    }
}
