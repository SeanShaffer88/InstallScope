//! The InstallScope desktop interface (egui, rendered with DirectX 12 on Windows).

mod a11y;
mod alt_ui;
mod app;
mod ask;
mod backups;
mod builder;
mod cleaner;
mod dashboard;
mod debloat_ui;
mod difftree_ui;
mod extinv_ui;
#[cfg(windows)]
mod filedialog;
mod gatekeeper_ui;
mod lab;
mod logs;
mod monitor;
#[cfg(all(windows, feature = "native-ui-preview"))]
mod native_programs;
mod plan;
mod precheck;
mod programs;
mod restore_ui;
mod settings_tab;
mod shell_ui;
mod slowdown_ui;
mod startup_tab;
mod storage_ui;
mod theme;
mod transfer_ui;
mod widgets;

use installscope_core::snapshot::Progress;
use std::sync::mpsc;
use std::sync::Arc;

/// Reloads a list in the background. A reload asked for while one is
/// running is queued, so the newest data always wins.
pub struct Reload<T> {
    job: Option<Job<T>>,
    again: bool,
}

impl<T> Default for Reload<T> {
    fn default() -> Self {
        Reload {
            job: None,
            again: false,
        }
    }
}

impl<T: Send + 'static> Reload<T> {
    pub fn request(&mut self, ctx: &egui::Context, f: impl FnOnce() -> T + Send + 'static) {
        if self.job.is_some() {
            self.again = true;
        } else {
            self.job = Some(Job::spawn(ctx, move |_| f()));
        }
    }

    /// The new data when a reload finished. `rerun` is called to start the
    /// queued reload, if any.
    pub fn poll(&mut self) -> Option<(T, bool)> {
        let r = self.job.as_ref()?.poll()?;
        self.job = None;
        Some((r, std::mem::take(&mut self.again)))
    }

    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
}

/// Work running on a background thread, polled once per frame.
pub struct Job<T> {
    rx: mpsc::Receiver<T>,
    pub progress: Arc<Progress>,
}

impl<T: Send + 'static> Job<T> {
    pub fn spawn(ctx: &egui::Context, f: impl FnOnce(&Progress) -> T + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        let progress = Arc::new(Progress::default());
        let p = progress.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            #[cfg(windows)]
            installscope_core::platform::windows::lower_thread_priority();
            let r = f(&p);
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        Job { rx, progress }
    }

    /// The result, once the work has finished.
    pub fn poll(&self) -> Option<T> {
        self.rx.try_recv().ok()
    }

    pub fn cancel(&self) {
        self.progress
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Window/taskbar icon: the small-size variant (red card, black E), which
/// stays legible at taskbar size.
fn load_icon() -> Option<egui::IconData> {
    let bytes = include_bytes!("../../assets/icon-taskbar.png");
    let img = installscope_core::png::decode(bytes).ok()?;
    Some(egui::IconData {
        rgba: img.pixels,
        width: img.width,
        height: img.height,
    })
}

/// Use Windows' own fonts: Segoe UI for the interface, plus the CJK fonts
/// that ship with Windows 11 so Chinese/Japanese/Korean text renders. Nothing
/// is bundled or downloaded.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let windir = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    let candidates: [(&str, &str, bool); 5] = [
        ("segoe", "segoeui.ttf", true),
        ("yahei", "msyh.ttc", false),
        ("yugothic", "YuGothM.ttc", false),
        ("malgun", "malgun.ttf", false),
        ("segoe-symbol", "seguisym.ttf", false),
    ];
    let mut primary_set = false;
    for (name, file, primary) in candidates {
        let path = std::path::Path::new(&windir).join("Fonts").join(file);
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        fonts.font_data.insert(
            name.to_string(),
            Arc::new(egui::FontData::from_owned(bytes)),
        );
        let fam = fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default();
        if primary {
            fam.insert(0, name.to_string());
            primary_set = true;
        } else {
            fam.push(name.to_string());
        }
        // CJK fallbacks for monospace text (paths, registry keys) too.
        if !primary {
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push(name.to_string());
        }
    }
    let _ = primary_set;
    ctx.set_fonts(fonts);
}

pub fn run(install_target: Option<String>, launch_args: Vec<String>) -> anyhow::Result<()> {
    #[cfg(all(windows, feature = "native-ui-preview"))]
    {
        return native_programs::run(install_target, launch_args);
    }

    #[cfg(not(all(windows, feature = "native-ui-preview")))]
    {
        let mut viewport = egui::ViewportBuilder::default()
            .with_title("InstallScope")
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([820.0, 520.0])
            .with_app_id("InstallScope");
        if let Some(icon) = load_icon() {
            viewport = viewport.with_icon(Arc::new(icon));
        }
        let options = eframe::NativeOptions {
            viewport,
            renderer: eframe::Renderer::Wgpu,
            centered: true,
            ..Default::default()
        };
        eframe::run_native(
            "InstallScope",
            options,
            Box::new(|cc| {
                install_fonts(&cc.egui_ctx);
                Ok(Box::new(app::App::new(cc, install_target, launch_args)))
            }),
        )?;
        Ok(())
    }
}
