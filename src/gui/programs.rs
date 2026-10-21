//! Programs tab: installed programs, details, single and batch uninstall.

use super::app::App;
use super::plan::RemovalFlow;
use super::theme;
use super::widgets::*;
use super::Job;
use egui::RichText;
use egui_extras::{Column, TableBuilder};
use setupsweep_core::i18n::{t, tf};
use setupsweep_core::leftovers::{find_leftovers, LeftoverScope};
use setupsweep_core::model::{fold, human_size};
use setupsweep_core::platform::{self, ChildProcess};
use setupsweep_core::programs::{self, InstalledProgram, Source};
use setupsweep_core::revert::{build_plan, Action, Plan, PlanItem};
use setupsweep_core::silent::{self, Engine, ExitMeaning};
use setupsweep_core::trust::{self, Trust};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    All,
    Monitored,
    Broken,
    Unsigned,
    /// Not opened for six months or more.
    Unused,
    Store,
}

/// On what evidence a program counts as long-unused: a last-opened time
/// Windows actually recorded, or — when it never recorded the program
/// being opened — only the date it was installed. The two must not be
/// presented to the user as the same fact: an install date says nothing
/// about when the program was last opened.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnusedBasis {
    /// Days since the program was last opened, from Windows' records.
    LastOpened(i64),
    /// Days since the program was installed; no last-opened record exists.
    Installed(i64),
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum SortCol {
    #[default]
    Name,
    Publisher,
    Version,
    Date,
    Size,
}

#[derive(Default)]
pub struct ProgramsState {
    /// The filtered, sorted list and the monitored-program set, recomputed
    /// only when their inputs change (not on every frame).
    pub visible_cache: Option<(u64, std::rc::Rc<Vec<InstalledProgram>>)>,
    pub monitored_cache: Option<(u64, std::rc::Rc<HashSet<String>>)>,
    pub list: Vec<InstalledProgram>,
    pub store_apps: Vec<InstalledProgram>,
    pub broken: HashSet<String>,
    pub sizes: HashMap<String, u64>,
    /// Last opened, memory, CPU and startup delay, by program key.
    pub usage: HashMap<String, setupsweep_core::usage::ProgramUsage>,
    pub usage_job: Option<Job<HashMap<String, setupsweep_core::usage::ProgramUsage>>>,
    pub job: Option<Job<(Vec<InstalledProgram>, HashSet<String>)>>,
    pub store_job: Option<Job<Vec<InstalledProgram>>>,
    pub size_job: Option<Job<HashMap<String, u64>>>,
    /// Program icons by registry key (loaded in the background).
    pub icons: HashMap<String, egui::TextureHandle>,
    pub icon_job: Option<Job<Vec<(String, egui::ColorImage)>>>,
    pub icons_requested: HashSet<String>,
    /// Digital-signature status by registry key (checked in the background).
    pub trust: HashMap<String, Trust>,
    /// Sizes and signature results kept between launches.
    pub cache: std::sync::Arc<std::sync::Mutex<setupsweep_core::cache::Cache>>,
    pub trust_job: Option<Job<Vec<(String, Trust)>>>,
    pub restore_job: Option<Job<Result<(), String>>>,
    pub filter: String,
    pub view: View,
    pub sort: SortCol,
    pub ascending: bool,
    pub selected: Option<String>,
    pub checked: BTreeSet<String>,
    pub needs_refresh: bool,
    pub sorted_once: bool,
}

#[derive(PartialEq, Eq)]
enum Stage {
    Confirm,
    RestorePoint,
    Running,
    Scanning,
}

/// Progress of one program in the uninstall queue.
#[derive(Clone, Debug, PartialEq)]
enum ItemStatus {
    Pending,
    Waiting,
    Running,
    Done,
    /// Removed; Windows needs a restart to finish.
    Reboot,
    Failed(String),
    Skipped(String),
    TimedOut,
}

/// Unattended limits: one uninstaller may run this long before SetupSweep moves on…
const UNATTENDED_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15 * 60);
/// …and an uninstaller that exited gets this long for its entry to disappear
/// (many relaunch themselves from %TEMP% and finish in the background).
const EXIT_GRACE: std::time::Duration = std::time::Duration::from_secs(60);
/// How long to wait for another Windows Installer job before trying anyway.
const BUSY_WAIT: std::time::Duration = std::time::Duration::from_secs(10 * 60);

pub struct Wizard {
    pub programs: Vec<InstalledProgram>,
    /// Skip the vendor uninstaller and go straight to cleanup.
    pub force: bool,
    /// Unattended: silent switches, no windows or prompts, no restarts.
    quiet: bool,
    /// In unattended mode, skip programs whose uninstaller can't run silently.
    skip_manual: bool,
    engines: Vec<Engine>,
    detect_job: Option<Job<Vec<Engine>>>,
    status: Vec<ItemStatus>,
    exited_at: Option<Instant>,
    waiting_since: Option<Instant>,
    retries: u32,
    restore_point: bool,
    /// A restore point was actually created for this run (set when the
    /// restore-point job succeeds). The leftover-removal flow reads it
    /// so its progress/summary text can name the rollback that exists.
    restore_point_created: bool,
    /// Store apps (by display name) the user confirmed for deprovisioning
    /// in the Bloatware Shield. Consumed once, when the uninstalls finish:
    /// only names whose program was actually removed are deprovisioned.
    /// Empty for every other wizard; dropping the wizard (cancel) drops
    /// the list unrun.
    pub deprovision: Vec<String>,
    stage: Stage,
    index: usize,
    launched: bool,
    child: Option<ChildProcess>,
    started: Option<Instant>,
    notes: Vec<String>,
    /// Restore point; returns notes.
    rp_job: Option<Job<std::result::Result<(), String>>>,
    store_job: Option<Job<Result<String, String>>>,
    scan_job: Option<Job<(Plan, Vec<String>, bool)>>,
}

impl Wizard {
    pub fn new(programs: Vec<InstalledProgram>, force: bool, restore_point: bool) -> Self {
        Wizard {
            programs: programs.clone(),
            status: vec![ItemStatus::Pending; programs.len()],
            force,
            quiet: false,
            skip_manual: false,
            engines: Vec::new(),
            detect_job: None,
            exited_at: None,
            waiting_since: None,
            retries: 0,
            restore_point,
            restore_point_created: false,
            deprovision: Vec::new(),
            stage: Stage::Confirm,
            index: 0,
            launched: false,
            child: None,
            started: None,
            notes: Vec::new(),
            rp_job: None,
            store_job: None,
            scan_job: None,
        }
    }

    /// Finish the current program with `status` and move to the next.
    fn advance_with(&mut self, status: ItemStatus) {
        if let Some(s) = self.status.get_mut(self.index) {
            *s = status;
        }
        self.index += 1;
        self.launched = false;
        self.child = None;
        self.started = None;
        self.store_job = None;
        self.exited_at = None;
        self.waiting_since = None;
        self.retries = 0;
    }

    fn advance(&mut self) {
        self.advance_with(ItemStatus::Done);
    }

    fn engine(&self, i: usize) -> Engine {
        self.engines
            .get(i)
            .copied()
            .unwrap_or_else(|| silent::quick_engine(&self.programs[i]))
    }

    /// Summary lines for the leftover report.
    fn report(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (p, s) in self.programs.iter().zip(&self.status) {
            let n = p.name.clone();
            match s {
                ItemStatus::Failed(why) => out.push(tf(
                    "{name}: not removed ({reason})",
                    &[("name", n), ("reason", why.clone())],
                )),
                ItemStatus::Skipped(why) => out.push(tf(
                    "{name}: skipped ({reason})",
                    &[("name", n), ("reason", why.clone())],
                )),
                ItemStatus::TimedOut => out.push(tf(
                    "{name}: its uninstaller was still running after 15 minutes",
                    &[("name", n)],
                )),
                _ => {}
            }
        }
        out
    }
}

/// Small shield showing a program's signature status, with details on hover.
fn trust_glyph(ui: &mut egui::Ui, t_: Option<&Trust>) {
    let (sym, color, tip) = match t_ {
        Some(Trust::Signed { signer }) => (
            "🛡",
            theme::OK,
            tf("Signed by {name}", &[("name", signer.clone())]),
        ),
        Some(Trust::Unsigned) => ("🛡", theme::WARN, t("Not digitally signed")),
        Some(Trust::Invalid { signer, reason }) => {
            let who = if signer.is_empty() {
                String::new()
            } else {
                format!(" ({signer})")
            };
            (
                "✖",
                theme::BAD,
                format!("{}{who}: {}", t("Signature problem"), t(reason)),
            )
        }
        _ => return,
    };
    super::a11y::explain(ui.label(RichText::new(sym).color(color).small()), &tip);
}

/// Merge plan items, dropping duplicates.
pub fn merge_plans(into: &mut Plan, other: Plan) {
    let mut seen: HashSet<String> = into
        .items
        .iter()
        .map(|i| fold(&i.action.describe()))
        .collect();
    for item in other.items {
        if seen.insert(fold(&item.action.describe())) {
            into.items.push(item);
        }
    }
    into.info.extend(other.info);
}

pub use setupsweep_core::programs::log_for;

impl App {
    pub fn refresh_programs(&mut self, ctx: &egui::Context) {
        let reg = self.reg.clone();
        let hidden = self.settings.system_components_shown();
        let root = self.store.root.clone();
        self.programs.job = Some(Job::spawn(ctx, move |_| {
            let mut list = programs::list_programs(reg.as_ref(), hidden);
            list.extend(programs::load_manual(&root));
            let broken = list
                .iter()
                .filter(|p| p.is_broken())
                .map(|p| p.key.clone())
                .collect();
            (list, broken)
        }));
        if self.settings.show_store_apps {
            self.programs.store_job = Some(Job::spawn(ctx, |_| programs::list_store_apps()));
        } else {
            self.programs.store_apps.clear();
        }
        self.programs.needs_refresh = false;
    }

    /// Days since a program was last opened; for programs Windows has no
    /// record of opening, days since it was installed. None while unknown.
    pub fn unused_days(&self, p: &InstalledProgram) -> Option<i64> {
        match self.unused_basis(p) {
            Some(UnusedBasis::LastOpened(days)) | Some(UnusedBasis::Installed(days)) => Some(days),
            None => None,
        }
    }

    /// The evidence behind `unused_days` for `p`, so callers that tell the
    /// user a program "hasn't been opened" can distinguish a recorded
    /// last-opened date from a mere install date. None while the program
    /// has no usage record at all, or neither date is known.
    pub fn unused_basis(&self, p: &InstalledProgram) -> Option<UnusedBasis> {
        let u = self.programs.usage.get(&p.key)?;
        match u.last_opened {
            Some(ts) => {
                let now = setupsweep_core::time::unix_now();
                Some(UnusedBasis::LastOpened((now - ts) / 86_400))
            }
            None => setupsweep_core::nlquery::parse_date(&p.install_date).map(|d| {
                UnusedBasis::Installed(setupsweep_core::time::Date::today_local().days_since(d))
            }),
        }
    }

    pub fn poll_programs(&mut self, ctx: &egui::Context) {
        if let Some(u) = self.programs.usage_job.as_ref().and_then(|j| j.poll()) {
            self.programs.usage_job = None;
            self.programs.usage = u;
            self.programs.visible_cache = None;
        }
        if self.programs.needs_refresh && self.removal.as_ref().is_none_or(|r| r.job.is_none()) {
            self.refresh_programs(ctx);
        }
        if let Some((list, broken)) = self.programs.job.as_ref().and_then(|j| j.poll()) {
            self.programs.job = None;
            self.programs.list = list;
            self.programs.broken = broken;
            let n = self.programs.list.len();
            self.set_status(tf("{n} programs installed", &[("n", n.to_string())]));
            self.load_icons(ctx);
            self.check_trust(ctx);
            // How each program is used, worked out in the background.
            let reg = self.reg.clone();
            let root = self.store.root.clone();
            let list = self.programs.list.clone();
            self.programs.usage_job = Some(Job::spawn(ctx, move |_| {
                let data = setupsweep_core::usage::UsageData::gather(reg.as_ref(), &root);
                list.iter()
                    .map(|p| (p.key.clone(), data.for_program(p)))
                    .collect()
            }));
            // Measure folders for programs that don't report their size.
            let todo: Vec<(String, String)> = self
                .programs
                .list
                .iter()
                .filter(|p| {
                    p.size_kb == 0
                        && !p.install_location.is_empty()
                        && !self.programs.sizes.contains_key(&p.key)
                })
                .map(|p| (p.key.clone(), p.install_location.clone()))
                .collect();
            // Forget programs that were uninstalled; reuse sizes measured recently.
            let now = setupsweep_core::cache::now();
            let versions: HashMap<String, String> = self
                .programs
                .list
                .iter()
                .map(|p| (p.key.clone(), p.version.clone()))
                .collect();
            let todo: Vec<(String, String, String)> = {
                let mut c = self.programs.cache.lock().unwrap();
                c.retain_keys(&versions.keys().cloned().collect());
                todo.into_iter()
                    .filter_map(|(k, loc)| {
                        let v = versions.get(&k).cloned().unwrap_or_default();
                        match c.size(&k, &loc, &v, now) {
                            Some(bytes) => {
                                self.programs.sizes.insert(k, bytes);
                                None
                            }
                            None => Some((k, loc, v)),
                        }
                    })
                    .collect()
            };
            if !todo.is_empty() {
                let protected = setupsweep_core::util::protected_dirs();
                let cache = self.programs.cache.clone();
                self.programs.size_job = Some(Job::spawn(ctx, move |pr| {
                    let mut m = HashMap::new();
                    for (k, loc, v) in todo {
                        if pr.cancelled() {
                            break;
                        }
                        if !setupsweep_core::util::is_protected_dir(&loc, &protected)
                            && std::path::Path::new(&loc).is_dir()
                        {
                            let bytes = programs::dir_size(&loc);
                            cache.lock().unwrap().put_size(&k, &loc, &v, bytes, now);
                            m.insert(k, bytes);
                        }
                    }
                    m
                }));
            }
        }
        if let Some(apps) = self.programs.store_job.as_ref().and_then(|j| j.poll()) {
            self.programs.store_job = None;
            self.programs.store_apps = apps;
        }
        if let Some(sizes) = self.programs.size_job.as_ref().and_then(|j| j.poll()) {
            self.programs.size_job = None;
            self.programs.sizes.extend(sizes);
            self.save_cache();
        }
        if let Some(results) = self.programs.trust_job.as_ref().and_then(|j| j.poll()) {
            self.programs.trust_job = None;
            self.programs.trust.extend(results);
            self.save_cache();
        }
        if let Some(icons) = self.programs.icon_job.as_ref().and_then(|j| j.poll()) {
            self.programs.icon_job = None;
            for (key, img) in icons {
                let tex =
                    ctx.load_texture(format!("icon:{key}"), img, egui::TextureOptions::LINEAR);
                self.programs.icons.insert(key, tex);
            }
        }
    }

    /// Verify each program's digital signature, off the UI thread.
    fn check_trust(&mut self, ctx: &egui::Context) {
        if self.programs.trust_job.is_some() {
            return;
        }
        let todo: Vec<InstalledProgram> = self
            .programs
            .list
            .iter()
            .filter(|p| !self.programs.trust.contains_key(&p.key))
            .cloned()
            .collect();
        if todo.is_empty() {
            return;
        }
        let reg = self.reg.clone();
        let cache = self.programs.cache.clone();
        self.programs.trust_job = Some(Job::spawn(ctx, move |_| {
            todo.iter()
                .map(|p| {
                    let Some(path) = trust::trust_target(p, reg.as_ref()) else {
                        return (p.key.clone(), Trust::Unknown);
                    };
                    let stamp = setupsweep_core::cache::file_stamp(&path);
                    if let Some(t) = cache.lock().unwrap().trust(&p.key, &path, stamp) {
                        return (p.key.clone(), t);
                    }
                    let t = trust::check_file(&path);
                    cache.lock().unwrap().put_trust(&p.key, &path, stamp, &t);
                    (p.key.clone(), t)
                })
                .collect()
        }));
    }

    /// Write the cache in the background.
    fn save_cache(&self) {
        let cache = self.programs.cache.lock().unwrap().clone();
        let path = self.store.cache_path();
        std::thread::spawn(move || {
            let _ = cache.save(&path);
        });
    }

    /// Extract icons for programs we haven't seen yet, off the UI thread.
    fn load_icons(&mut self, ctx: &egui::Context) {
        if self.programs.icon_job.is_some() {
            return;
        }
        let todo: Vec<(String, String, i32)> = self
            .programs
            .list
            .iter()
            .filter(|p| !self.programs.icons_requested.contains(&p.key))
            .filter_map(|p| {
                p.icon_source()
                    .map(|(path, idx)| (p.key.clone(), path, idx))
            })
            .collect();
        for p in &self.programs.list {
            self.programs.icons_requested.insert(p.key.clone());
        }
        if todo.is_empty() {
            return;
        }
        self.programs.icon_job = Some(Job::spawn(ctx, move |_| {
            todo.into_iter()
                .filter_map(|(key, path, idx)| {
                    let (w, h, rgba) = platform::icon_rgba(&path, idx, 32)?;
                    Some((
                        key,
                        egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba),
                    ))
                })
                .collect()
        }));
    }

    fn size_of(&self, p: &InstalledProgram) -> u64 {
        if p.size_kb > 0 {
            p.size_kb * 1024
        } else {
            self.programs.sizes.get(&p.key).copied().unwrap_or(0)
        }
    }

    /// Cheap fingerprint of the data the program views are built from.
    fn data_sig(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let st = &self.programs;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (st.list.as_ptr() as usize, st.list.len()).hash(&mut h);
        (st.store_apps.as_ptr() as usize, st.store_apps.len()).hash(&mut h);
        (st.sizes.len(), st.trust.len(), st.broken.len()).hash(&mut h);
        (self.logs.logs.as_ptr() as usize, self.logs.logs.len()).hash(&mut h);
        h.finish()
    }

    /// Keys of installed programs that have an install log.
    pub fn monitored_keys(&mut self) -> std::rc::Rc<HashSet<String>> {
        let sig = self.data_sig();
        if let Some((s, m)) = &self.programs.monitored_cache {
            if *s == sig {
                return m.clone();
            }
        }
        use setupsweep_core::model::fold;
        let by_key: HashSet<String> = self
            .logs
            .logs
            .iter()
            .filter(|l| !l.uninstall_key.is_empty())
            .map(|l| fold(&l.uninstall_key))
            .collect();
        let by_name: HashSet<String> = self.logs.logs.iter().map(|l| fold(&l.name)).collect();
        let m: HashSet<String> = self
            .programs
            .list
            .iter()
            .filter(|p| by_key.contains(&fold(&p.key)) || by_name.contains(&fold(&p.name)))
            .map(|p| p.key.clone())
            .collect();
        let m = std::rc::Rc::new(m);
        self.programs.monitored_cache = Some((sig, m.clone()));
        m
    }

    fn visible_programs(&mut self) -> std::rc::Rc<Vec<InstalledProgram>> {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.data_sig().hash(&mut h);
        let st = &self.programs;
        (&st.filter, st.view as u8, st.sort as u8, st.ascending).hash(&mut h);
        let sig = h.finish();
        if let Some((s, v)) = &self.programs.visible_cache {
            if *s == sig {
                return v.clone();
            }
        }
        let monitored = self.monitored_keys();
        let st = &self.programs;
        let f = st.filter.to_lowercase();
        let source: Vec<&InstalledProgram> = match st.view {
            View::Store => st.store_apps.iter().collect(),
            _ => st.list.iter().collect(),
        };
        let mut v: Vec<InstalledProgram> = source
            .into_iter()
            .filter(|p| match st.view {
                View::Monitored => monitored.contains(&p.key),
                View::Broken => st.broken.contains(&p.key),
                View::Unsigned => matches!(
                    st.trust.get(&p.key),
                    Some(Trust::Unsigned | Trust::Invalid { .. })
                ),
                View::Unused => self.unused_days(p).is_some_and(|d| d >= 180),
                _ => true,
            })
            .filter(|p| {
                f.is_empty()
                    || p.name.to_lowercase().contains(&f)
                    || p.publisher.to_lowercase().contains(&f)
            })
            .cloned()
            .collect();
        match st.sort {
            SortCol::Name => v.sort_by_cached_key(|p| p.name.to_lowercase()),
            SortCol::Publisher => v.sort_by_cached_key(|p| p.publisher.to_lowercase()),
            SortCol::Version => v.sort_by(|a, b| a.version.cmp(&b.version)),
            SortCol::Date => v.sort_by(|a, b| a.install_date.cmp(&b.install_date)),
            SortCol::Size => v.sort_by_key(|p| self.size_of(p)),
        }
        if !self.programs.ascending {
            v.reverse();
        }
        let v = std::rc::Rc::new(v);
        self.programs.visible_cache = Some((sig, v.clone()));
        v
    }

    pub fn start_uninstall(&mut self, programs: Vec<InstalledProgram>, force: bool) {
        if programs.is_empty() {
            return;
        }
        self.wizard = Some(Wizard::new(
            programs,
            force,
            self.settings.create_restore_point,
        ));
    }

    /// Start the uninstall wizard carrying confirmed deprovisioning intent
    /// (Bloatware Shield). The names are consumed only after the matching
    /// programs were actually removed; closing the wizard before then —
    /// at the review stage or mid-run — discards them unrun.
    pub fn start_uninstall_with_deprovision(
        &mut self,
        programs: Vec<InstalledProgram>,
        deprovision: Vec<String>,
    ) {
        if programs.is_empty() {
            return;
        }
        let mut w = Wizard::new(programs, false, self.settings.create_restore_point);
        w.deprovision = deprovision;
        self.wizard = Some(w);
    }

    pub fn ui_programs(&mut self, ctx: &egui::Context) {
        if !self.programs.sorted_once {
            self.programs.ascending = true;
            self.programs.sorted_once = true;
        }
        let visible = self.visible_programs();
        let selected = self
            .programs
            .selected
            .as_ref()
            .and_then(|k| visible.iter().find(|p| &p.key == k).cloned());

        egui::SidePanel::right("program_details")
            .frame(theme::side_frame())
            .show_separator_line(false)
            .resizable(true)
            .default_width(310.0)
            .min_width(260.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| match &selected {
                    Some(p) => self.program_details(ui, p),
                    None => {
                        // Keep the panel's width so the table doesn't shift.
                        ui.set_min_width(ui.available_width());
                        ui.add_space(30.0);
                        ui.vertical_centered(|ui| {
                            muted(ui, t("Select a program to see its details."));
                        });
                    }
                });
            });

        egui::CentralPanel::default()
            .frame(theme::content_frame())
            .show(ctx, |ui| {
                // The focused control's explanation line, for keyboard
                // and touch users (see a11y::explain).
                super::a11y::focus_hint(ui, theme::MUTED);
                ui.horizontal(|ui| {
                    let r = super::a11y::explain(
                        search_box(ui, &mut self.programs.filter, "Search programs", 260.0),
                        &t("Search by program name or publisher. The list below filters as you type."),
                    );
                    if self.focus_search {
                        r.request_focus();
                        self.focus_search = false;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if super::a11y::explain(
                            ui.button(format!("⟳ {}", t("Refresh"))),
                            &t("Reload the list of installed programs from Windows."),
                        )
                        .clicked()
                        {
                            self.refresh_programs(ctx);
                        }
                        if super::a11y::explain(
                            ui.button(format!("➕ {}", t("Add program…"))),
                            &t("Add a program Windows doesn't list (for example a portable program) by choosing its folder, so SetupSweep can analyze and remove it with everything it left behind."),
                        )
                        .clicked()
                        {
                            if let Some(dir) = pick_folder(&t("Choose the program's folder")) {
                                match programs::add_manual(&self.store.root, &dir.to_string_lossy(), "") {
                                    Ok(p) => {
                                        self.programs.selected = Some(p.key.clone());
                                        self.set_status(tf("Added {name}", &[("name", p.name)]));
                                        self.refresh_programs(ctx);
                                    }
                                    Err(e) => self.set_status(e.to_string()),
                                }
                            }
                        }
                        let n = self.programs.checked.len();
                        if n > 0 {
                            if super::a11y::explain(
                                theme::primary_button(
                                    ui,
                                    tf("Uninstall {n} selected", &[("n", n.to_string())]),
                                ),
                                &t("Uninstall the ticked programs. Each one goes through the uninstall wizard, where you review before anything is removed."),
                            )
                            .clicked()
                            {
                                let mut all = self.programs.list.clone();
                                all.extend(self.programs.store_apps.clone());
                                let chosen: Vec<InstalledProgram> = all
                                    .into_iter()
                                    .filter(|p| self.programs.checked.contains(&p.key))
                                    .collect();
                                self.start_uninstall(chosen, false);
                            }
                            if super::a11y::explain(
                                ui.small_button(t("Clear")),
                                &t("Untick every program. Nothing is removed."),
                            )
                            .clicked()
                            {
                                self.programs.checked.clear();
                            }
                            if super::a11y::explain(
                                ui.button(tf("Transfer {n}…", &[("n", n.to_string())])),
                                &t("Pack the selected programs to move them to another PC."),
                            )
                            .clicked()
                            {
                                let mut all = self.programs.list.clone();
                                all.extend(self.programs.store_apps.clone());
                                let chosen: Vec<InstalledProgram> = all
                                    .into_iter()
                                    .filter(|p| self.programs.checked.contains(&p.key))
                                    .collect();
                                self.start_transfer_export(chosen);
                            }
                        }
                    });
                });
                ui.horizontal_wrapped(|ui| {
                    let views = [
                        (View::All, t("All"), t("Show every installed program.")),
                        (
                            View::Monitored,
                            t("Monitored"),
                            t("Show only programs SetupSweep recorded being installed. Those can be removed precisely, from their install log."),
                        ),
                        (
                            View::Broken,
                            tf(
                                "Broken ({n})",
                                &[("n", self.programs.broken.len().to_string())],
                            ),
                            t("Show entries whose program files or uninstaller are missing — usually leftovers of programs removed another way."),
                        ),
                        (
                            View::Unsigned,
                            tf(
                                "Unsigned ({n})",
                                &[(
                                    "n",
                                    self.programs
                                        .trust
                                        .values()
                                        .filter(|t| {
                                            matches!(t, Trust::Unsigned | Trust::Invalid { .. })
                                        })
                                        .count()
                                        .to_string(),
                                )],
                            ),
                            t("Show programs that are not digitally signed, or whose signature has a problem."),
                        ),
                        (
                            View::Unused,
                            t("Not used for 6 months"),
                            t("Show programs with no record of being opened in six months. Where Windows has no last-opened record, programs installed at least six months ago are shown."),
                        ),
                        (
                            View::Store,
                            t("Store apps"),
                            t("Show apps installed from the Microsoft Store."),
                        ),
                    ];
                    let views = views.to_vec();
                    for (v, label, why) in views {
                        if super::a11y::explain(
                            ui.selectable_label(self.programs.view == v, label),
                            &why,
                        )
                        .clicked()
                        {
                            self.programs.view = v;
                            if v == View::Store
                                && self.programs.store_apps.is_empty()
                                && self.programs.store_job.is_none()
                            {
                                self.programs.store_job =
                                    Some(Job::spawn(ctx, |_| programs::list_store_apps()));
                            }
                        }
                    }
                });
                ui.add_space(4.0);
                if self.programs.job.is_some()
                    || (self.programs.view == View::Store && self.programs.store_job.is_some())
                {
                    job_progress(ui, &t("Reading installed programs…"), 0);
                    return;
                }
                if visible.is_empty() {
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| muted(ui, t("No programs match.")));
                    return;
                }
                self.programs_table(ui, &visible);
            });
    }

    fn programs_table(&mut self, ui: &mut egui::Ui, visible: &[InstalledProgram]) {
        let row_h = 24.0;
        // Keyboard: arrows/Home/End/Page keys move the selection, Space ticks it.
        let blocked = self.wizard.is_some() || self.removal.is_some() || self.confirm.is_some();
        let cur = self
            .programs
            .selected
            .as_ref()
            .and_then(|k| visible.iter().position(|p| &p.key == k));
        let nav = super::a11y::list_nav(ui.ctx(), visible.len(), cur, blocked);
        if let Some(n) = nav {
            self.programs.selected = Some(visible[n].key.clone());
        }
        if let Some(n) = nav.or(cur) {
            if super::a11y::space_pressed(ui.ctx(), blocked) {
                let k = visible[n].key.clone();
                if !self.programs.checked.remove(&k) {
                    self.programs.checked.insert(k);
                }
            }
        }
        let mut clicked: Option<String> = None;
        let mut double: Option<InstalledProgram> = None;
        let mut sort_click: Option<SortCol> = None;
        let monitored = self.monitored_keys();
        let st = &mut self.programs;
        let sizes = &st.sizes;
        let size_of = |p: &InstalledProgram| {
            if p.size_kb > 0 {
                p.size_kb * 1024
            } else {
                sizes.get(&p.key).copied().unwrap_or(0)
            }
        };
        let header =
            |ui: &mut egui::Ui, label: &str, col: SortCol, cur: SortCol, asc: bool| -> bool {
                let arrow = if col == cur {
                    if asc {
                        " ⏶"
                    } else {
                        " ⏷"
                    }
                } else {
                    ""
                };
                let why = match col {
                    SortCol::Name => {
                        t("Sort the list by program name. Activate again to reverse the order.")
                    }
                    SortCol::Publisher => {
                        t("Sort the list by publisher. Activate again to reverse the order.")
                    }
                    SortCol::Version => {
                        t("Sort the list by version. Activate again to reverse the order.")
                    }
                    SortCol::Date => {
                        t("Sort the list by install date. Activate again to reverse the order.")
                    }
                    SortCol::Size => {
                        t("Sort the list by size. Activate again to reverse the order.")
                    }
                };
                super::a11y::explain(
                    ui.add(
                        egui::Button::new(RichText::new(format!("{}{arrow}", t(label))).strong())
                            .frame(false),
                    ),
                    &why,
                )
                .clicked()
            };
        let (cur, asc) = (st.sort, st.ascending);
        let mut table = TableBuilder::new(ui);
        if let Some(n) = nav {
            table = table.scroll_to_row(n, None);
        }
        table
            .striped(true)
            .sense(egui::Sense::click())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::exact(22.0))
            // Minimum widths add up to less than the narrowest normal window,
            // so the Size column never gets cut off.
            .column(Column::remainder().at_least(130.0).clip(true))
            .column(
                Column::initial(150.0)
                    .at_least(80.0)
                    .clip(true)
                    .resizable(true),
            )
            .column(
                Column::initial(90.0)
                    .at_least(60.0)
                    .clip(true)
                    .resizable(true),
            )
            .column(Column::initial(92.0).at_least(60.0).resizable(true))
            .column(Column::initial(92.0).at_least(70.0).resizable(true))
            .header(row_h, |mut h| {
                h.col(|_| {});
                h.col(|ui| {
                    if header(ui, "Name", SortCol::Name, cur, asc) {
                        sort_click = Some(SortCol::Name);
                    }
                });
                h.col(|ui| {
                    if header(ui, "Publisher", SortCol::Publisher, cur, asc) {
                        sort_click = Some(SortCol::Publisher);
                    }
                });
                h.col(|ui| {
                    if header(ui, "Version", SortCol::Version, cur, asc) {
                        sort_click = Some(SortCol::Version);
                    }
                });
                h.col(|ui| {
                    if header(ui, "Installed", SortCol::Date, cur, asc) {
                        sort_click = Some(SortCol::Date);
                    }
                });
                h.col(|ui| {
                    if header(ui, "Size", SortCol::Size, cur, asc) {
                        sort_click = Some(SortCol::Size);
                    }
                });
            })
            .body(|body| {
                body.rows(row_h, visible.len(), |mut row| {
                    let p = &visible[row.index()];
                    let is_sel = st.selected.as_deref() == Some(&p.key);
                    row.set_selected(is_sel);
                    row.col(|ui| {
                        let mut c = st.checked.contains(&p.key);
                        let r = super::a11y::explain(
                            ui.checkbox(&mut c, ""),
                            &tf(
                                "Select {name} for a batch action: ticked programs can be uninstalled or transferred together.",
                                &[("name", p.name.clone())],
                            ),
                        );
                        let name = p.name.clone();
                        r.widget_info(|| {
                            egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, c, &name)
                        });
                        if r.changed() {
                            if c {
                                st.checked.insert(p.key.clone());
                            } else {
                                st.checked.remove(&p.key);
                            }
                        }
                    });
                    row.col(|ui| {
                        if monitored.contains(&p.key) {
                            super::a11y::explain(
                                ui.label(RichText::new("⏺").color(theme::ACCENT_TEXT)),
                                &t("Monitored: precise uninstall available"),
                            );
                        }
                        if st.broken.contains(&p.key) {
                            super::a11y::explain(
                                ui.label(RichText::new("⚠").color(theme::WARN)),
                                &t("Broken entry: uninstaller or folder is missing"),
                            );
                        }
                        match st.icons.get(&p.key) {
                            Some(tex) => {
                                ui.add(
                                    egui::Image::new(tex).fit_to_exact_size(egui::vec2(18.0, 18.0)),
                                );
                            }
                            None => {
                                ui.add_sized(
                                    [18.0, 18.0],
                                    egui::Label::new(
                                        RichText::new("📦").color(theme::MUTED).small(),
                                    ),
                                );
                            }
                        }
                        trust_glyph(ui, st.trust.get(&p.key));
                        ui.add(egui::Label::new(&p.name).truncate().selectable(false));
                    });
                    row.col(|ui| {
                        ui.add(
                            egui::Label::new(RichText::new(&p.publisher).color(theme::MUTED))
                                .truncate()
                                .selectable(false),
                        );
                    });
                    row.col(|ui| {
                        ui.add(egui::Label::new(&p.version).truncate().selectable(false));
                    });
                    row.col(|ui| {
                        ui.add(egui::Label::new(&p.install_date).selectable(false));
                    });
                    row.col(|ui| {
                        let s = size_of(p);
                        ui.add(
                            egui::Label::new(if s > 0 { human_size(s) } else { String::new() })
                                .selectable(false),
                        );
                    });
                    let resp = row.response();
                    if resp.clicked() {
                        clicked = Some(p.key.clone());
                    }
                    if resp.double_clicked() {
                        double = Some(p.clone());
                    }
                });
            });
        if let Some(c) = sort_click {
            if st.sort == c {
                st.ascending = !st.ascending;
            } else {
                st.sort = c;
                st.ascending = c != SortCol::Size && c != SortCol::Date;
            }
        }
        if let Some(k) = clicked {
            st.selected = Some(k);
        }
        if let Some(p) = double {
            self.start_uninstall(vec![p], false);
        }
    }

    fn program_details(&mut self, ui: &mut egui::Ui, p: &InstalledProgram) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if let Some(tex) = self.programs.icons.get(&p.key) {
                ui.add(egui::Image::new(tex).fit_to_exact_size(egui::vec2(32.0, 32.0)));
            }
            ui.add(egui::Label::new(RichText::new(&p.name).heading().strong()).wrap());
        });
        ui.horizontal_wrapped(|ui| {
            theme::badge(ui, t(p.source.label()), theme::MUTED);
            if p.is_msi {
                theme::badge(ui, "MSI", theme::MUTED);
            }
            if log_for(&self.logs.logs, p).is_some() {
                theme::badge(ui, t("Monitored"), theme::ACCENT_TEXT);
            }
            if self.programs.broken.contains(&p.key) {
                theme::badge(ui, t("Broken entry"), theme::WARN);
            }
        });
        ui.add_space(8.0);
        let size = self.size_of(p);
        egui::Grid::new("pd")
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                field(ui, "Publisher", &p.publisher);
                field(ui, "Version", &p.version);
                field(ui, "Installed", &p.install_date);
                field(
                    ui,
                    "Size",
                    &if size > 0 {
                        human_size(size)
                    } else {
                        String::new()
                    },
                );
                field(ui, "Location", &p.install_location);
                if let Some(u) = self.programs.usage.get(&p.key) {
                    let ago = |ts: i64| {
                        let days = (setupsweep_core::time::unix_now() - ts) / 86_400;
                        match days {
                            0 => t("today"),
                            1 => t("yesterday"),
                            d if d < 60 => tf("{n} days ago", &[("n", d.to_string())]),
                            d if d < 730 => tf("{n} months ago", &[("n", (d / 30).to_string())]),
                            d => tf("{n} years ago", &[("n", (d / 365).to_string())]),
                        }
                    };
                    let opened = match (u.last_opened, u.runs) {
                        (Some(ts), Some(r)) => {
                            format!("{} ({})", ago(ts), tf("{n} times", &[("n", r.to_string())]))
                        }
                        (Some(ts), None) => ago(ts),
                        (None, _) => t("No record of it being opened"),
                    };
                    field(ui, "Last opened", &opened);
                    if let (Some(avg), Some(peak)) = (u.avg_ram, u.peak_ram) {
                        field(
                            ui,
                            "Memory",
                            &tf(
                                "{avg} on average, up to {peak}",
                                &[("avg", human_size(avg)), ("peak", human_size(peak))],
                            ),
                        );
                    }
                    if let Some(c) = u.cpu_percent {
                        field(
                            ui,
                            "Processor",
                            &tf(
                                "{n}% of one core while running",
                                &[("n", format!("{c:.1}"))],
                            ),
                        );
                    }
                    if let Some(ms) = u.startup_ms {
                        field(
                            ui,
                            "Startup delay",
                            &tf(
                                "{n} s at each start of Windows",
                                &[("n", format!("{:.1}", ms as f64 / 1000.0))],
                            ),
                        );
                    }
                }
                if let Some(tr) = self.programs.trust.get(&p.key) {
                    let text = match tr {
                        Trust::Signed { signer } => {
                            tf("Signed by {name}", &[("name", signer.clone())])
                        }
                        Trust::Invalid { signer, reason } if !signer.is_empty() => {
                            format!("{} — {} ({signer})", t("Signature problem"), t(reason))
                        }
                        Trust::Invalid { reason, .. } => {
                            format!("{} — {}", t("Signature problem"), t(reason))
                        }
                        other => t(other.label()),
                    };
                    field(ui, "Digital signature", &text);
                }
                if p.source != Source::Store && self.settings.advanced_mode {
                    field(ui, "Uninstaller", &p.uninstall_string);
                    field(ui, "Registry key", &p.key);
                } else if self.settings.advanced_mode {
                    field(ui, "Package", &p.key);
                }
                if !p.comments.is_empty() {
                    field(ui, "Comments", &p.comments);
                }
            });
        if let Some(Trust::Invalid { reason, .. }) = self.programs.trust.get(&p.key) {
            ui.add_space(8.0);
            theme::warning_banner(
                ui,
                tf("This program's signature has a problem: {error}. Its files may have been altered.", &[("error", t(reason))]),
            );
        }
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            if super::a11y::explain(
                theme::primary_button(ui, format!("🗑 {}", t("Uninstall"))),
                &t("Start the uninstall wizard for this program. You review everything before anything is removed."),
            )
            .clicked()
            {
                self.start_uninstall(vec![p.clone()], false);
            }
            if p.source != Source::Store
                && silent::quick_engine(p).is_silent()
                && super::a11y::explain(
                    ui.button(t("Uninstall silently")),
                    &t("No windows or prompts; SetupSweep runs the uninstaller in its silent mode."),
                )
                .clicked()
            {
                let mut w = Wizard::new(vec![p.clone()], false, self.settings.create_restore_point);
                w.quiet = true;
                self.wizard = Some(w);
            }
        });
        ui.horizontal_wrapped(|ui| {
            if p.source != Source::Store
                && self.settings.advanced_mode
                && super::a11y::explain(
                    ui.button(t("Forced removal")),
                    &t("Skip the program's own uninstaller and remove everything SetupSweep can find."),
                )
                .clicked()
            {
                self.start_uninstall(vec![p.clone()], true);
            }
            if p.source != Source::Store
                && super::a11y::explain(
                    ui.button(t("Transfer to another PC…")),
                    &t("Pack this program with its files and settings to move it to another PC."),
                )
                .clicked()
            {
                self.start_transfer_export(vec![p.clone()]);
            }
            if !p.install_location.is_empty()
                && super::a11y::explain(
                    ui.button(t("Open folder")),
                    &t("Open the folder this program is installed in, in File Explorer."),
                )
                .clicked()
            {
                platform::shell_open(&p.install_location);
            }
            if p.source != Source::Store
                && self.settings.advanced_mode
                && super::a11y::explain(
                    ui.button(t("Open in Registry Editor")),
                    &t("Open this program's uninstall entry in Registry Editor. Changes there apply immediately, outside SetupSweep."),
                )
                .clicked()
            {
                if let Err(e) = platform::open_in_regedit(&p.key) {
                    self.set_status(e.to_string());
                }
            }
            if super::a11y::explain(
                ui.button(t("Search online")),
                &t("Search the web for this program in your browser."),
            )
            .clicked()
            {
                let q: String = p.name.chars().map(|c| if c.is_alphanumeric() { c.to_string() } else { "+".into() }).collect();
                platform::shell_open(&format!("https://www.bing.com/search?q={q}"));
            }
            if !p.url.is_empty()
                && super::a11y::explain(
                    ui.button(t("Website")),
                    &t("Open the publisher's website for this program in your browser."),
                )
                .clicked()
            {
                platform::shell_open(&p.url);
            }
            if p.source == Source::Manual
                && super::a11y::explain(
                    ui.button(t("Remove from list")),
                    &t("Only removes it from this list; nothing is deleted."),
                )
                .clicked()
            {
                let _ = programs::remove_manual(&self.store.root, &p.key);
                self.programs.selected = None;
                self.programs.needs_refresh = true;
            }
        });
        if p.source != Source::Store {
            ui.add_space(8.0);
            self.alternatives_ui(ui, std::slice::from_ref(&p.name));
        }
        if let Some(log) = log_for(&self.logs.logs, p) {
            ui.add_space(10.0);
            let id = log.id.clone();
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.label(RichText::new(format!("⏺ {}", t("Monitored installation"))).color(theme::ACCENT_TEXT).strong());
                small_muted(ui, t("SetupSweep recorded this installation, so it can remove exactly what was added."));
                if super::a11y::explain(
                    ui.link(t("View install log")),
                    &t("Open the Install Logs page with this installation selected, to see everything it changed."),
                )
                .clicked()
                {
                    self.logs.selected = Some(id);
                    self.tab = super::app::Tab::Logs;
                }
            });
        }
    }

    pub fn ui_wizard(&mut self, ctx: &egui::Context) {
        let Some(w) = &mut self.wizard else { return };
        let reg = self.reg.clone();
        let mut close = false;
        let mut start_scan = false;
        let names: Vec<String> = w.programs.iter().map(|p| p.name.clone()).collect();
        let title = if w.force {
            t("Forced removal")
        } else {
            t("Uninstall")
        };

        // --- state machine -------------------------------------------------
        if w.stage == Stage::RestorePoint {
            if let Some(r) = w.rp_job.as_ref().and_then(|j| j.poll()) {
                w.rp_job = None;
                match r {
                    Ok(()) => {
                        w.restore_point_created = true;
                        w.stage = if w.force {
                            Stage::Scanning
                        } else {
                            Stage::Running
                        };
                        if w.force {
                            start_scan = true;
                        }
                    }
                    Err(error) => {
                        self.status = tf(
                            "Restore point not created; uninstall canceled: {error}",
                            &[("error", error)],
                        );
                        close = true;
                    }
                }
            }
        }
        if w.detect_job.is_none() && w.engines.is_empty() && !w.force {
            let progs = w.programs.clone();
            w.detect_job = Some(Job::spawn(ctx, move |_| {
                progs.iter().map(silent::detect).collect()
            }));
        }
        if let Some(e) = w.detect_job.as_ref().and_then(|j| j.poll()) {
            w.engines = e;
        }
        if w.stage == Stage::Running {
            if w.index >= w.programs.len() {
                let report = w.report();
                w.notes.extend(report);
                if w.status.contains(&ItemStatus::Reboot) || silent::reboot_pending(reg.as_ref()) {
                    w.notes
                        .push(t("Restart Windows to finish removing some programs."));
                    self.status = t("Restart Windows to finish removing some programs.");
                }
                w.stage = Stage::Scanning;
                start_scan = true;
            } else {
                let p = w.programs[w.index].clone();
                let engine = w.engine(w.index);
                // Unattended: wait while another Windows Installer job runs
                // (starting a second one fails with error 1618).
                let busy = !w.launched && w.quiet && platform::installer_busy();
                if busy && w.waiting_since.get_or_insert_with(Instant::now).elapsed() < BUSY_WAIT {
                    w.status[w.index] = ItemStatus::Waiting;
                    ctx.request_repaint_after(std::time::Duration::from_secs(1));
                } else if !w.launched {
                    w.launched = true;
                    w.started = Some(Instant::now());
                    w.status[w.index] = ItemStatus::Running;
                    let silent_cmd = if w.quiet {
                        silent::silent_command(&p, engine)
                    } else {
                        None
                    };
                    if p.source == Source::Store {
                        let key = p.key;
                        w.store_job = Some(Job::spawn(ctx, move |_| {
                            programs::remove_store_app(&key).map_err(|e| e.to_string())
                        }));
                    } else if w.quiet && silent_cmd.is_none() && w.skip_manual {
                        w.advance_with(ItemStatus::Skipped(t("can't run unattended")));
                    } else if let Some(cmd) = silent_cmd.or_else(|| p.uninstall_command(false)) {
                        match platform::spawn_command_line(&platform::expand_env(&cmd)) {
                            Ok(c) => w.child = Some(c),
                            Err(e) => {
                                w.notes.push(format!("{}: {e}", p.name));
                                w.advance_with(ItemStatus::Failed(e.to_string()));
                            }
                        }
                    } else {
                        w.notes.push(tf(
                            "{name} has no uninstaller; cleaning up directly.",
                            &[("name", p.name)],
                        ));
                        w.advance();
                    }
                } else if let Some(r) = w.store_job.as_ref().and_then(|j| j.poll()) {
                    match r {
                        Ok(_) => w.advance(),
                        Err(e) => {
                            w.notes.push(format!("{}: {e}", p.name));
                            w.advance_with(ItemStatus::Failed(e));
                        }
                    }
                } else if let Some(c) = &w.child {
                    let code = c.try_wait();
                    // Many uninstallers relaunch themselves from %TEMP% and exit
                    // at once; wait for the Uninstall entry to disappear too.
                    let gone = !reg.exists(&p.key);
                    let meaning = code.map(|c| silent::exit_meaning(engine, c));
                    let ran_for = w.started.map(|s| s.elapsed()).unwrap_or_default();
                    match meaning {
                        Some(ExitMeaning::Busy) if w.retries < 3 => {
                            // Another installation started meanwhile: try again.
                            w.retries += 1;
                            w.launched = false;
                            w.child = None;
                            w.waiting_since = Some(Instant::now());
                        }
                        Some(ExitMeaning::Cancelled) if !gone => {
                            w.advance_with(ItemStatus::Failed(t("cancelled")));
                        }
                        Some(m) if gone || p.is_msi => {
                            if m == ExitMeaning::RebootRequired {
                                w.advance_with(ItemStatus::Reboot);
                            } else if m == ExitMeaning::Failed && !gone {
                                w.advance_with(ItemStatus::Failed(tf(
                                    "exit code {code}",
                                    &[("code", code.unwrap_or_default().to_string())],
                                )));
                            } else {
                                w.advance();
                            }
                        }
                        Some(_) if w.quiet => {
                            // Exited but still listed: give a relaunched copy time to finish.
                            if w.exited_at.get_or_insert_with(Instant::now).elapsed() > EXIT_GRACE {
                                w.advance_with(ItemStatus::Failed(tf(
                                    "still installed after its uninstaller finished (exit code {code})",
                                    &[("code", code.unwrap_or_default().to_string())],
                                )));
                            }
                        }
                        None if w.quiet && ran_for > UNATTENDED_TIMEOUT => {
                            w.advance_with(ItemStatus::TimedOut);
                        }
                        _ => {}
                    }
                    ctx.request_repaint_after(std::time::Duration::from_millis(500));
                }
            }
        }
        if let Some((plan, logs, offer)) = w.scan_job.as_ref().and_then(|j| j.poll()) {
            w.scan_job = None;
            // Deprovisioning, deferred from the Bloatware Shield: the
            // uninstallers have finished, so the confirmed names whose
            // program was really removed are deprovisioned now — never
            // before the removal, and never for skipped or failed ones.
            // The names arrived with the user's confirmation; a wizard
            // cancelled at any earlier stage took them with it.
            let pending = std::mem::take(&mut w.deprovision);
            if !pending.is_empty() {
                let removed: Vec<String> = w
                    .programs
                    .iter()
                    .zip(w.status.iter())
                    .filter(|(_, s)| matches!(s, ItemStatus::Done | ItemStatus::Reboot))
                    .map(|(p, _)| p.name.clone())
                    .collect();
                let (go, skipped) =
                    setupsweep_core::debloat::partition_deprovision(&pending, &removed);
                self.debloat.deprovision_job = Some(Job::spawn(ctx, move |_| {
                    let mut rows: Vec<setupsweep_core::backup::ActionResult> = go
                        .iter()
                        .map(|n| match setupsweep_core::debloat::deprovision(n) {
                            Ok(()) => setupsweep_core::backup::ActionResult::ok(
                                n,
                                t("Deprovisioned: new user accounts won't get this app"),
                            ),
                            Err(e) => {
                                setupsweep_core::backup::ActionResult::err(n, format!("{e:#}"))
                            }
                        })
                        .collect();
                    rows.extend(skipped.iter().map(|n| {
                        setupsweep_core::backup::ActionResult::err(
                            n,
                            t("Not deprovisioned: the app is still installed"),
                        )
                    }));
                    rows
                }));
            }
            let name = if names.len() == 1 {
                names[0].clone()
            } else {
                tf("{n} programs", &[("n", names.len().to_string())])
            };
            let mut flow = RemovalFlow::new(
                tf("Leftovers: {name}", &[("name", name)]),
                plan,
                offer,
                self.settings.remove_unlogged_enabled(),
            );
            flow.logs_to_delete = logs;
            flow.names = names;
            flow.restore_point_created = w.restore_point_created;
            flow.plan.info.extend(w.notes.clone());
            self.open_removal(flow);
            self.wizard = None;
            self.programs.checked.clear();
            self.programs.needs_refresh = true;
            return;
        }
        if w.stage == Stage::Scanning && w.scan_job.is_none() {
            start_scan = true;
        }
        if start_scan {
            let progs = w.programs.clone();
            let logs = self.logs.logs.clone();
            w.scan_job = Some(Job::spawn(ctx, move |pr| {
                let mut plan = Plan::default();
                let mut used_logs = Vec::new();
                let scope = LeftoverScope::default();
                for p in &progs {
                    pr.set_phase(p.name.clone());
                    if let Some(log) = log_for(&logs, p) {
                        let mut lp = build_plan(&log.changes, reg.as_ref());
                        if reg.exists(&p.key) {
                            lp.items.push(PlanItem {
                                action: Action::DeleteKey {
                                    path: p.key.clone(),
                                },
                                selected: true,
                                exists: true,
                                note: String::new(),
                            });
                        }
                        merge_plans(&mut plan, lp);
                        used_logs.push(log.id.clone());
                    }
                    // Heuristics catch what happened outside monitoring (e.g. updates).
                    let mut lo = find_leftovers(p, reg.as_ref(), &scope);
                    if !used_logs.is_empty() {
                        lo.items.iter_mut().for_each(|i| {
                            if matches!(i.action, Action::DeleteTree { .. }) {
                                i.selected = false;
                                i.note = "Name match".into();
                            }
                        });
                    }
                    merge_plans(&mut plan, lo);
                }
                plan.items
                    .sort_by(|a, b| a.action.category().cmp(b.action.category()));
                let offer = !used_logs.is_empty();
                (plan, used_logs, offer)
            }));
        }

        // --- window --------------------------------------------------------
        let elevated = self.elevated;
        let mut open = true;
        egui::Window::new(title)
            .id(egui::Id::new("wizard"))
            .collapsible(false)
            .resizable(false)
            .default_width(520.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                let Some(w) = &mut self.wizard else { return };
                match w.stage {
                    Stage::Confirm => {
                        ui.label(if w.force {
                            t("SetupSweep will skip the programs' own uninstallers and remove everything it can find:")
                        } else {
                            t("These programs will be uninstalled, then SetupSweep scans for leftovers:")
                        });
                        let detecting = w.detect_job.is_some();
                        egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                            for (i, p) in w.programs.iter().enumerate() {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(format!("• {}", p.name)).strong());
                                    if w.quiet && !w.force {
                                        let e = w.engines.get(i).copied();
                                        match e {
                                            None if detecting => theme::badge(ui, t("Checking…"), theme::MUTED),
                                            Some(Engine::Manual) | None => {
                                                let txt = if w.skip_manual { t("Will be skipped") } else { t("Needs its own window") };
                                                theme::badge(ui, txt, theme::WARN)
                                            }
                                            Some(e) => theme::badge(ui, format!("✔ {}", t(e.label())), theme::OK),
                                        }
                                    }
                                    if log_for(&self.logs.logs, p).is_some() {
                                        theme::badge(ui, t("Monitored"), theme::ACCENT_TEXT);
                                    }
                                    if let Some(Trust::Invalid { .. }) = self.programs.trust.get(&p.key) {
                                        theme::badge(ui, t("Signature problem"), theme::BAD);
                                    }
                                });
                            }
                        });
                        ui.add_space(6.0);
                        if !w.force {
                            ui.label(t("Each program's own uninstaller runs first. SetupSweep cannot undo changes made by a program's own uninstaller."));
                            ui.label(if self.settings.backup_enabled() {
                                t("Afterwards, SetupSweep removes any leftovers it finds. That removal is backed up first, so it can be undone from the Backups page.")
                            } else {
                                t("Afterwards, SetupSweep removes any leftovers it finds. Backups are off in Settings, so that removal will not be backed up and cannot be undone.")
                            });
                            ui.add_space(6.0);
                            super::a11y::explain(
                                ui.checkbox(&mut w.quiet, t("Unattended: no windows, prompts or restarts")),
                                &t("SetupSweep uses each uninstaller's silent mode, runs them one at a time, holds back restarts until the end, and moves on if one gets stuck. You can walk away."),
                            );
                            if w.quiet {
                                let manual = w.engines.iter().filter(|e| **e == Engine::Manual).count();
                                if manual > 0 {
                                    ui.indent("skip_manual", |ui| {
                                        super::a11y::explain(
                                            ui.checkbox(
                                                &mut w.skip_manual,
                                                tf("Skip programs that need their own window ({n}); uninstall them later", &[("n", manual.to_string())]),
                                            ),
                                            &t("Leave the programs that can't run silently installed instead of stopping at each one. You can uninstall them later with their windows shown."),
                                        );
                                    });
                                }
                            }
                        }
                        let rp_why = if elevated {
                            t("Create a Windows restore point before uninstalling, so you can roll the PC back if an uninstall causes problems. It holds system files and settings, not the programs themselves.")
                        } else {
                            t("Unavailable: creating a restore point needs administrator rights. Run SetupSweep as administrator to use this.")
                        };
                        ui.add_enabled_ui(elevated, |ui| {
                            super::a11y::explain(
                                ui.checkbox(&mut w.restore_point, t("Create a System Restore point first")),
                                &rp_why,
                            );
                        });
                        small_muted(ui, t("A restore point lets Windows roll your PC back to how it is now, if an uninstall causes problems."));
                        if !elevated {
                            small_muted(ui, t("Needs administrator rights, so no restore point will be created. Run SetupSweep as administrator to turn this on."));
                        }
                        ui.add_space(8.0);
                        let start_why = if w.force {
                            t("Start the forced removal. SetupSweep scans for everything these programs left behind, and you review the list before anything is removed.")
                        } else {
                            t("Start uninstalling. Each program's own uninstaller runs first — SetupSweep cannot undo what that does — then you review the leftovers before they are removed.")
                        };
                        ui.horizontal(|ui| {
                            if super::a11y::explain(theme::primary_button(ui, t("Start")), &start_why)
                                .clicked()
                            {
                                if w.restore_point && elevated {
                                    w.stage = Stage::RestorePoint;
                                    w.rp_job = Some(Job::spawn(ui.ctx(), move |_| {
                                        setupsweep_core::restore::create_restore_point(
                                            "Before SetupSweep uninstall",
                                        )
                                        .map(|_| ())
                                        .map_err(|error| error.to_string())
                                    }));
                                } else if w.force {
                                    w.stage = Stage::Scanning;
                                    start_scan_later(ui.ctx());
                                } else {
                                    w.stage = Stage::Running;
                                }
                            }
                            if super::a11y::explain(
                                ui.button(t("Cancel")),
                                &t("Close the wizard without uninstalling anything."),
                            )
                            .clicked()
                            {
                                close = true;
                            }
                        });
                    }
                    Stage::RestorePoint => job_progress(ui, &t("Creating a restore point…"), 0),
                    Stage::Running => {
                        let total = w.programs.len();
                        let cur = w.index.min(total.saturating_sub(1));
                        let p = w.programs[cur].clone();
                        ui.label(tf("Uninstalling {done} of {total}", &[("done", (cur + 1).to_string()), ("total", total.to_string())]));
                        ui.add(egui::ProgressBar::new(cur as f32 / total as f32).desired_height(6.0));
                        ui.add_space(6.0);
                        if total > 1 || w.quiet {
                            egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                                for (q, s) in w.programs.iter().zip(&w.status) {
                                    ui.horizontal(|ui| {
                                        let (icon, color, text) = match s {
                                            ItemStatus::Pending => ("•", theme::MUTED, String::new()),
                                            ItemStatus::Waiting => ("…", theme::WARN, t("Waiting for another installation to finish")),
                                            ItemStatus::Running => ("⟳", theme::ACCENT_TEXT, t("Uninstalling")),
                                            ItemStatus::Done => ("✔", theme::OK, t("Removed")),
                                            ItemStatus::Reboot => ("✔", theme::WARN, t("Removed; restart needed")),
                                            ItemStatus::Failed(r) => ("✖", theme::BAD, r.clone()),
                                            ItemStatus::Skipped(r) => ("⏭", theme::MUTED, r.clone()),
                                            ItemStatus::TimedOut => ("⏱", theme::BAD, t("Took too long; moved on")),
                                        };
                                        ui.label(RichText::new(icon).color(color).strong());
                                        ui.label(&q.name);
                                        if !text.is_empty() {
                                            small_muted(ui, text);
                                        }
                                    });
                                }
                            });
                            ui.add_space(6.0);
                        } else {
                            ui.horizontal(|ui| {
                                super::widgets::spinner(ui);
                                ui.label(RichText::new(&p.name).strong());
                            });
                        }
                        let unattended_now = w.quiet && w.engine(cur).is_silent();
                        small_muted(
                            ui,
                            if unattended_now {
                                t("Running silently. You can leave this running; SetupSweep continues by itself.")
                            } else {
                                t("Follow the program's own uninstaller. When it has finished, SetupSweep continues automatically.")
                            },
                        );
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if !unattended_now
                                && super::a11y::explain(
                                    ui.button(t("It's finished, continue")),
                                    &t("Tell SetupSweep this program's uninstaller has finished, so it moves on to scanning for leftovers."),
                                )
                                .clicked()
                            {
                                w.advance();
                            }
                            if super::a11y::explain(
                                ui.button(t("Skip this program")),
                                &t("Skip this program and leave it installed. SetupSweep moves on to the next one."),
                            )
                            .clicked()
                            {
                                w.notes.push(tf("Skipped {name}", &[("name", p.name.clone())]));
                                w.advance_with(ItemStatus::Skipped(t("skipped by you")));
                            }
                        });
                    }
                    Stage::Scanning => {
                        let phase = w.scan_job.as_ref().map(|j| j.progress.phase()).unwrap_or_default();
                        job_progress(ui, &tf("Scanning for leftovers… {name}", &[("name", phase)]), 0);
                    }
                }
                // The focused control's explanation line, at the foot
                // of the wizard (see a11y::explain).
                super::a11y::focus_hint(ui, theme::MUTED);
            });
        let at_start = self
            .wizard
            .as_ref()
            .is_some_and(|w| w.stage == Stage::Confirm);
        if at_start && self.confirm.is_none() && super::a11y::escape_pressed(ctx) {
            close = true;
        }
        if !open || close {
            self.wizard = None;
        }
    }
}

/// The scan starts on the next frame (the window closure can't start jobs
/// that need `self`), so just make sure that frame happens.
fn start_scan_later(ctx: &egui::Context) {
    ctx.request_repaint();
}
