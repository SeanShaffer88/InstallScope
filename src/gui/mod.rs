//! The SetupScope desktop interface (egui, rendered with DirectX 12 on Windows).

mod a11y;
mod app;
mod build_log_ui;
mod build_run_ui;
mod container_readiness_ui;
mod dashboard;
mod dependency_ui;
mod devcheck_ui;
mod devinspect_ui;
mod diagnostic_bundle_ui;
mod envprofile_ui;
#[cfg(windows)]
mod filedialog;
mod native_toolchain_ui;
mod project_readiness_ui;
mod projectdoctor_ui;
pub(crate) mod system_info;
mod theme;
mod tool_window;
mod widgets;

use setupscope_core::snapshot::Progress;
use std::sync::mpsc;
use std::sync::Arc;

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
            setupscope_core::platform::windows::lower_thread_priority();
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

/// Scoped Precision S mark, mechanically resized for the taskbar.
fn load_icon() -> Option<egui::IconData> {
    let bytes = include_bytes!("../../assets/icon-taskbar.png");
    let img = setupscope_core::png::decode(bytes).ok()?;
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
    {
        let mut viewport = egui::ViewportBuilder::default()
            .with_title("SetupScope")
            .with_inner_size([960.0, 600.0])
            .with_maximized(false)
            .with_min_inner_size([740.0, 300.0])
            .with_app_id("SetupScope");
        if let Some(icon) = load_icon() {
            viewport = viewport.with_icon(Arc::new(icon));
        }
        let options = eframe::NativeOptions {
            viewport,
            renderer: eframe::Renderer::Wgpu,
            centered: true,
            persist_window: false,
            ..Default::default()
        };
        eframe::run_native(
            "SetupScope",
            options,
            Box::new(|cc| {
                install_fonts(&cc.egui_ctx);
                Ok(Box::new(app::App::new(cc, install_target, launch_args)?))
            }),
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod branding_tests {
    #[test]
    fn selected_taskbar_art_decodes() {
        let icon = super::load_icon().expect("selected brand icon must decode");
        assert_eq!((icon.width, icon.height), (32, 32));
        assert_eq!(icon.rgba.len(), 32 * 32 * 4);
        assert!(icon.rgba.chunks_exact(4).any(|p| p[3] == 0));
        assert!(icon
            .rgba
            .chunks_exact(4)
            .any(|p| p[0] > 150 && p[0] > p[1] * 2 && p[3] > 200));
    }
}
