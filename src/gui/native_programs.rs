//! Experimental native Win32 Programs view. The existing interface remains
//! the release default until native workflows are ready to replace it.

use anyhow::{anyhow, Context, Result};
use installscope_core::backup::{ActionResult, BackupWriter};
use installscope_core::i18n::{self, t, tf};
use installscope_core::leftovers::{self, LeftoverScope};
use installscope_core::model::InstallLog;
use installscope_core::platform::{self, Registry};
use installscope_core::programs::{self, InstalledProgram};
use installscope_core::revert::{self, ExecOptions, Plan};
use installscope_core::store::Store;
use std::ffi::c_void;
use std::sync::{mpsc, Arc};
use std::time::Instant;
use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontW, CreateSolidBrush, DeleteObject, DrawFocusRect, DrawTextW, FillRect,
    GetStockObject, SelectObject, SetBkColor, SetBkMode, SetTextColor, UpdateWindow,
    DEFAULT_CHARSET, DEFAULT_GUI_FONT, DEFAULT_PITCH, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT,
    DT_RIGHT, DT_SINGLELINE, DT_VCENTER, FW_SEMIBOLD, HBRUSH, HDC, HFONT, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Controls::{SetWindowTheme, DRAWITEMSTRUCT, MEASUREITEMSTRUCT};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, ReleaseCapture, SetCapture, SetFocus, VK_ESCAPE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateMenu, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyWindow,
    DispatchMessageW, DrawMenuBar, GetAncestor, GetClientRect, GetCursorPos, GetMessageW,
    GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, KillTimer,
    LoadCursorW, MoveWindow, PostQuitMessage, RegisterClassW, SendMessageW, SetCursor, SetMenu,
    SetMenuInfo, SetTimer, SetWindowLongPtrW, SetWindowTextW, ShowWindow, TranslateMessage,
    WindowFromPoint, CREATESTRUCTW, CW_USEDEFAULT, GA_ROOT, GWLP_USERDATA, HMENU, IDC_ARROW,
    IDC_CROSS, LBS_EXTENDEDSEL, LBS_HASSTRINGS, LBS_NOTIFY, LBS_OWNERDRAWFIXED, LB_ADDSTRING,
    LB_GETCURSEL, LB_GETSELCOUNT, LB_GETSELITEMS, LB_RESETCONTENT, LB_SETCURSEL,
    MB_ICONINFORMATION, MB_ICONWARNING, MB_YESNO, MENUINFO, MF_GRAYED, MF_OWNERDRAW, MF_POPUP,
    MF_STRING, MIM_APPLYTOSUBMENUS, MIM_BACKGROUND, MSG, SW_HIDE, SW_SHOW, WM_CANCELMODE,
    WM_CAPTURECHANGED, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_DESTROY, WM_DRAWITEM, WM_ERASEBKGND,
    WM_KEYDOWN, WM_LBUTTONUP, WM_MEASUREITEM, WM_NCCREATE, WM_NCDESTROY, WM_SETCURSOR, WM_SIZE,
    WM_TIMER, WNDCLASSW, WS_CHILD, WS_CLIPCHILDREN, WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE,
    WS_VSCROLL,
};

const CLASS_NAME: &str = "InstallScopeNativeProgramsPreview";
const WINDOW_WIDTH: i32 = 1000;
const WINDOW_HEIGHT: i32 = 680;
const ID_SEARCH: u16 = 101;
const ID_REFRESH: u16 = 102;
const ID_LIST: u16 = 103;
const ID_HUNTER: u16 = 104;
const ID_UNINSTALL: u16 = 105;
const ID_TOGGLE_ITEM: u16 = 106;
const ID_REMOVE_SELECTED: u16 = 107;
const ID_BACK: u16 = 108;
const ID_BRAND_MARK: u16 = 109;
const ID_BRAND_NAME: u16 = 110;
const ID_NAV_PROGRAMS: u16 = 111;
const ID_PAGE_TITLE: u16 = 112;
const ID_PAGE_SUBTITLE: u16 = 113;
const ID_LIST_HEADING: u16 = 114;
const ID_TABLE_HEADER: u16 = 115;
const ID_FOOTER: u16 = 116;
const ID_ADD_MANUAL: u16 = 117;
const ID_RESTORE_POINT: u16 = 118;
const ID_NAV_BASE: u16 = 120;
const ID_NAV_TOOLS: u16 = ID_NAV_BASE + 6;
const ID_MENU_EXIT: u16 = 200;
const ID_MENU_REFRESH: u16 = 201;
const ID_MENU_HUNTER: u16 = 202;
const ID_MENU_UNINSTALL: u16 = 203;
const ID_MENU_ABOUT: u16 = 204;
const ID_MENU_SCAN_ORPHANS: u16 = 205;
const ID_MENU_LABEL_FILE: u16 = 210;
const ID_MENU_LABEL_TOOLS: u16 = 211;
const ID_MENU_LABEL_VIEW: u16 = 212;
const ID_MENU_LABEL_HELP: u16 = 213;
const WM_PROGRAMS_READY: u32 = 0x8001;
const WM_PLAN_READY: u32 = 0x8002;
const WM_REMOVE_READY: u32 = 0x8003;
const WM_STORE_UNINSTALL_READY: u32 = 0x8004;
const WM_RESTORE_POINT_READY: u32 = 0x8005;
const TIMER_UNINSTALL: usize = 1;
const SEARCH_CHANGED: u16 = 0x0300;
const LIST_SELECTION_CHANGED: u16 = 1;
const BUTTON_CLICKED: u16 = 0;
const ES_AUTOHSCROLL: u32 = 0x0080;
const ES_AUTOVSCROLL: u32 = 0x0040;
const ES_LEFT: u32 = 0;
const ES_MULTILINE: u32 = 0x0004;
const ES_READONLY: u32 = 0x0800;
const SS_LEFT: u32 = 0;
const SS_NOPREFIX: u32 = 0x0080;
const BS_OWNERDRAW: u32 = 0x000B;
const BS_AUTOCHECKBOX: u32 = 0x0003;
const BM_GETCHECK: u32 = 0x00F0;
const BM_SETCHECK: u32 = 0x00F1;
const COLOR_BG: (u8, u8, u8) = (20, 17, 19);
const COLOR_PANEL: (u8, u8, u8) = (25, 21, 23);
const COLOR_CARD: (u8, u8, u8) = (34, 29, 32);
const COLOR_TEXT: (u8, u8, u8) = (236, 230, 233);
const COLOR_MUTED: (u8, u8, u8) = (168, 158, 163);
const COLOR_ACCENT: (u8, u8, u8) = (107, 7, 0);

type ProgramResult = std::result::Result<Vec<InstalledProgram>, String>;
type RestorePointResult = std::result::Result<(), String>;
type PlanResult = std::result::Result<Plan, String>;
type RemoveResult =
    std::result::Result<(Vec<ActionResult>, Option<String>, Option<String>), String>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Programs,
    RestorePoint,
    Uninstalling,
    Scanning,
    Review,
    Removing,
    Results,
}

/// What to start once the pre-uninstall restore point finishes. A failed
/// restore point cancels the pending uninstall, matching the main
/// interface: the safety net was requested, so the operation needs it.
enum PendingUninstall {
    Single {
        program: InstalledProgram,
        command: Option<String>,
    },
    Batch,
}

struct NativePrograms {
    hwnd: HWND,
    brand_mark: HWND,
    brand_name: HWND,
    nav_programs: HWND,
    nav_items: [HWND; 17],
    page_title: HWND,
    page_subtitle: HWND,
    list_heading: HWND,
    table_header: HWND,
    footer: HWND,
    search_label: HWND,
    search: HWND,
    list: HWND,
    details: HWND,
    status: HWND,
    refresh: HWND,
    hunter: HWND,
    add_manual: HWND,
    uninstall_button: HWND,
    restore_point_check: HWND,
    toggle_item: HWND,
    remove_selected: HWND,
    back: HWND,
    hunter_armed: bool,
    view: View,
    registry: Arc<dyn Registry>,
    store_root: std::path::PathBuf,
    backup_root: std::path::PathBuf,
    logs: Vec<InstallLog>,
    include_store_apps: bool,
    advanced_mode: bool,
    backup_enabled: bool,
    recycle_bin: bool,
    restart_explorer: bool,
    close_programs: bool,
    /// From the "Create a System Restore point before uninstalling" setting.
    create_restore_point: bool,
    /// Restore points need administrator rights.
    elevated: bool,
    programs: Vec<InstalledProgram>,
    visible: Vec<InstalledProgram>,
    labels: Vec<String>,
    receiver: Option<mpsc::Receiver<ProgramResult>>,
    uninstall_program: Option<InstalledProgram>,
    batch_queue: Vec<InstalledProgram>,
    batch_active: bool,
    batch_total: usize,
    batch_completed: usize,
    uninstall_child: Option<platform::ChildProcess>,
    uninstall_exit: Option<(u32, Instant)>,
    store_uninstall_receiver: Option<mpsc::Receiver<std::result::Result<String, String>>>,
    restore_point_receiver: Option<mpsc::Receiver<RestorePointResult>>,
    pending_uninstall: Option<PendingUninstall>,
    plan_receiver: Option<mpsc::Receiver<PlanResult>>,
    system_orphan_scan: bool,
    remove_receiver: Option<mpsc::Receiver<RemoveResult>>,
    plan: Option<Plan>,
    selected_plan_item: Option<usize>,
    brushes: [HBRUSH; 5],
    fonts: [HFONT; 3],
    font: isize,
}

impl Drop for NativePrograms {
    fn drop(&mut self) {
        for brush in self.brushes.iter().copied().filter(|b| !b.is_null()) {
            unsafe { DeleteObject(brush as _) };
        }
        for font in self.fonts.iter().copied().filter(|font| !font.is_null()) {
            unsafe { DeleteObject(font as _) };
        }
    }
}

impl NativePrograms {
    fn request_refresh(&mut self) {
        if self.view != View::Programs || self.receiver.is_some() {
            return;
        }
        self.set_status(&t("Reading installed programs…"));
        unsafe { EnableWindow(self.refresh, 0) };

        let registry = self.registry.clone();
        let store_root = self.store_root.clone();
        let include_store_apps = self.include_store_apps;
        let window = self.hwnd as isize;
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut list = programs::list_programs(registry.as_ref(), false);
                list.extend(programs::load_manual(&store_root));
                if include_store_apps {
                    list.extend(programs::list_store_apps());
                }
                list.sort_by_cached_key(|p| p.name.to_lowercase());
                list
            }))
            .map_err(|_| t("Could not read installed programs."));
            let _ = tx.send(result);
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                    window as HWND,
                    WM_PROGRAMS_READY,
                    0,
                    0,
                )
            };
        });
    }

    fn finish_refresh(&mut self) {
        let Some(receiver) = self.receiver.take() else {
            return;
        };
        let completed = match receiver.try_recv() {
            Ok(Ok(programs)) => {
                self.programs = programs;
                self.filter_programs();
                self.set_status(&tf(
                    "{n} programs",
                    &[("n", self.programs.len().to_string())],
                ));
                true
            }
            Ok(Err(error)) => {
                self.set_status(&error);
                show_error(self.hwnd, &error);
                true
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.receiver = Some(receiver);
                false
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                let message = t("The program list could not be loaded.");
                self.set_status(&message);
                show_error(self.hwnd, &message);
                true
            }
        };
        if completed {
            unsafe { EnableWindow(self.refresh, 1) };
        }
    }

    fn read_search(&self) -> String {
        unsafe {
            let len = GetWindowTextLengthW(self.search).clamp(0, 1024) as usize;
            let mut text = vec![0u16; len + 1];
            let read = GetWindowTextW(self.search, text.as_mut_ptr(), text.len() as i32);
            String::from_utf16_lossy(&text[..read.max(0) as usize])
        }
    }

    fn filter_programs(&mut self) {
        self.visible = filter_programs(&self.programs, &self.read_search());
        unsafe {
            SendMessageW(self.list, LB_RESETCONTENT, 0, 0);
            for p in &self.visible {
                let row = wide(&format!("{}    {}    {}", p.name, p.publisher, p.version));
                let result = SendMessageW(self.list, LB_ADDSTRING, 0, row.as_ptr() as LPARAM);
                if result < 0 {
                    let message = t("The program list is too large to display.");
                    self.set_status(&message);
                    show_error(self.hwnd, &message);
                    self.visible.clear();
                    break;
                }
            }
            if !self.visible.is_empty() {
                SendMessageW(self.list, LB_SETCURSEL, 0, 0);
                self.show_selected(0);
            } else {
                SetWindowTextW(
                    self.details,
                    wide(&t("Select a program to see its details.")).as_ptr(),
                );
            }
        }
        self.update_selection_action();
        if self.programs.is_empty() {
            self.set_status(&t("No installed programs found."));
        } else if self.visible.is_empty() {
            self.set_status(&t("No programs match."));
        }
    }

    fn show_selected(&self, index: usize) {
        let Some(program) = self.visible.get(index) else {
            return;
        };
        self.show_program(program);
    }

    fn show_program(&self, program: &InstalledProgram) {
        let details = format!(
            "{}\r\n\r\n{}: {}\r\n{}: {}\r\n{}: {}\r\n{}: {}\r\n{}: {}\r\n\r\n{}",
            program.name,
            t("Publisher"),
            value_or_unknown(&program.publisher),
            t("Version"),
            value_or_unknown(&program.version),
            t("Installed"),
            value_or_unknown(&program.install_date),
            t("Location"),
            value_or_unknown(&program.install_location),
            t("Source"),
            t(program.source.label()),
            t("The vendor uninstaller runs first; any leftover cleanup is shown for review.")
        );
        unsafe { SetWindowTextW(self.details, wide(&details).as_ptr()) };
    }

    fn selected_indices(&self) -> Vec<usize> {
        unsafe {
            let count = SendMessageW(self.list, LB_GETSELCOUNT, 0, 0);
            if count <= 0 {
                return SendMessageW(self.list, LB_GETCURSEL, 0, 0)
                    .try_into()
                    .ok()
                    .into_iter()
                    .collect();
            }
            let mut indices = vec![0i32; count as usize];
            let selected = SendMessageW(
                self.list,
                LB_GETSELITEMS,
                indices.len(),
                indices.as_mut_ptr() as LPARAM,
            );
            if selected < 0 {
                return Vec::new();
            }
            indices.truncate(selected as usize);
            indices
                .into_iter()
                .filter_map(|index| usize::try_from(index).ok())
                .collect()
        }
    }

    fn update_selection_action(&self) {
        if self.view != View::Programs {
            return;
        }
        let indices = self.selected_indices();
        let label = if indices.len() == 1
            && self
                .visible
                .get(indices[0])
                .is_some_and(|program| program.source == programs::Source::Manual)
        {
            t("Remove from list")
        } else if indices.len() > 1 {
            tf(
                "Uninstall {n} selected",
                &[("n", indices.len().to_string())],
            )
        } else {
            t("Uninstall")
        };
        unsafe { SetWindowTextW(self.uninstall_button, wide(&label).as_ptr()) };
    }

    fn add_manual_program(&mut self) {
        let Some(folder) = super::filedialog::pick_folder(&t("Choose the program's folder")) else {
            return;
        };
        match programs::add_manual(&self.store_root, &folder.to_string_lossy(), "") {
            Ok(program) => {
                self.set_status(&tf("Added {name}", &[("name", program.name)]));
                self.request_refresh();
            }
            Err(error) => {
                let message = format!("{}: {error:#}", t("Could not add the program"));
                self.set_status(&message);
                show_error(self.hwnd, &message);
            }
        }
    }

    fn remove_manual_program(&mut self, program: InstalledProgram) {
        let prompt = tf(
            "Remove {name} from InstallScope's program list? Its files and settings will not be deleted.",
            &[("name", program.name.clone())],
        );
        let confirmed = unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                self.hwnd,
                wide(&prompt).as_ptr(),
                wide("Remove from list").as_ptr(),
                MB_YESNO | MB_ICONWARNING,
            ) == 6
        };
        if !confirmed {
            return;
        }
        match programs::remove_manual(&self.store_root, &program.key) {
            Ok(()) => {
                self.set_status(&tf(
                    "Removed {name} from InstallScope's list. No files were deleted.",
                    &[("name", program.name)],
                ));
                self.request_refresh();
            }
            Err(error) => {
                let message = format!("{}: {error:#}", t("Could not remove the list entry"));
                self.set_status(&message);
                show_error(self.hwnd, &message);
            }
        }
    }

    fn uninstall_command(&self, program: &InstalledProgram) -> Result<Option<String>, String> {
        if program.source == programs::Source::Manual {
            return Err(t(
                "Manually added entries are analysis-only. Remove them from InstallScope's list instead.",
            ));
        }
        if program.no_remove {
            return Err(t("This program does not allow removal."));
        }
        if program.source == programs::Source::Store {
            return Ok(None);
        }
        if !self.registry.exists(&program.key) {
            return Err(t(
                "This program is no longer registered; refresh the list before uninstalling.",
            ));
        }
        program
            .uninstall_command(false)
            .map(Some)
            .ok_or_else(|| t("This program has no registered uninstaller."))
    }

    fn start_selected_action(&mut self) {
        let indices = self.selected_indices();
        let selected: Vec<InstalledProgram> = indices
            .iter()
            .filter_map(|index| self.visible.get(*index).cloned())
            .collect();
        if selected.is_empty() {
            self.set_status(&t("Select a program first."));
            return;
        }
        if selected.len() == 1 && selected[0].source == programs::Source::Manual {
            self.remove_manual_program(selected[0].clone());
        } else if selected.len() > 1 {
            self.start_batch_uninstall(selected);
        } else {
            self.start_uninstall(indices[0]);
        }
    }

    fn start_batch_uninstall(&mut self, selected: Vec<InstalledProgram>) {
        if self.restore_point_receiver.is_some() {
            return;
        }
        for program in &selected {
            if let Err(error) = self.uninstall_command(program) {
                let message = format!("{}: {error}", program.name);
                self.set_status(&message);
                show_error(self.hwnd, &message);
                return;
            }
        }
        let prompt = tf(
            "Uninstall {n} selected programs one at a time? InstallScope will show a separate leftover review after each program. Nothing is removed until you review that program's findings.",
            &[("n", selected.len().to_string())],
        );
        let confirmed = unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                self.hwnd,
                wide(&prompt).as_ptr(),
                wide("Confirm batch uninstall").as_ptr(),
                MB_YESNO | MB_ICONWARNING,
            ) == 6
        };
        if !confirmed {
            return;
        }
        self.batch_total = selected.len();
        self.batch_completed = 0;
        self.batch_queue = selected;
        self.batch_active = true;
        if self.create_restore_point && self.elevated {
            self.begin_restore_point(PendingUninstall::Batch);
        } else {
            self.launch_next_batch_program();
        }
    }

    fn launch_next_batch_program(&mut self) {
        let Some(program) = self.batch_queue.first().cloned() else {
            self.finish_batch();
            return;
        };
        match self.uninstall_command(&program) {
            Ok(command) => {
                self.set_status(&tf(
                    "Batch {current} of {total}: uninstalling {name}…",
                    &[
                        ("current", (self.batch_completed + 1).to_string()),
                        ("total", self.batch_total.to_string()),
                        ("name", program.name.clone()),
                    ],
                ));
                if let Err(error) = self.launch_uninstall(program, command) {
                    self.abort_batch(&format!("{error:#}"));
                }
            }
            Err(error) => self.abort_batch(&format!("{}: {error}", program.name)),
        }
    }

    fn finish_batch(&mut self) {
        self.batch_active = false;
        self.batch_queue.clear();
        self.set_status(&tf(
            "Batch uninstall complete: {n} programs reviewed.",
            &[("n", self.batch_completed.to_string())],
        ));
        self.request_refresh();
        self.layout_controls();
    }

    fn abort_batch(&mut self, error: &str) {
        self.batch_active = false;
        self.batch_queue.clear();
        self.set_status(error);
        show_error(self.hwnd, error);
        self.view = View::Programs;
        self.uninstall_program = None;
        self.layout_controls();
    }

    fn identify_window_target(&mut self) {
        self.hunter_armed = false;
        unsafe {
            ReleaseCapture();
            SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_ARROW));
        }
        let mut point: POINT = unsafe { std::mem::zeroed() };
        if unsafe { GetCursorPos(&mut point) } == 0 {
            self.set_status(&t("Could not read the pointer position."));
            return;
        }
        let target = unsafe { WindowFromPoint(point) };
        if target.is_null() {
            self.set_status(&t("No window was found at that position."));
            return;
        }
        let root = unsafe { GetAncestor(target, GA_ROOT) };
        if root.is_null() || root == self.hwnd {
            self.set_status(&t("Window identification canceled."));
            return;
        }
        let mut process_id = 0;
        if unsafe { GetWindowThreadProcessId(root, &mut process_id) } == 0 || process_id == 0 {
            self.set_status(&t("Could not identify the selected window's process."));
            return;
        }
        match process_image_path(process_id) {
            Ok(image_path) => match find_program_for_image(&self.programs, &image_path).cloned() {
                Some(program) => {
                    let key = program.key.clone();
                    if !self.visible.iter().any(|visible| visible.key == key) {
                        unsafe { SetWindowTextW(self.search, wide("").as_ptr()) };
                        self.filter_programs();
                    }
                    if let Some(index) = self.visible.iter().position(|p| p.key == key) {
                        unsafe { SendMessageW(self.list, LB_SETCURSEL, index, 0) };
                        self.show_selected(index);
                    }
                    self.set_status(&tf(
                        "Matched {program} to {path}",
                        &[
                            ("program", program.name.clone()),
                            ("path", image_path.display().to_string()),
                        ],
                    ));
                }
                None => self.set_status(&tf(
                    "No unique installed program matches {path}",
                    &[("path", image_path.display().to_string())],
                )),
            },
            Err(error) => self.set_status(&format!("{error:#}")),
        }
    }

    fn arm_window_hunter(&mut self) {
        self.hunter_armed = true;
        unsafe {
            SetFocus(self.hwnd);
            SetCapture(self.hwnd);
            SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_CROSS));
        }
        self.set_status(&t(
            "Click a window to identify its installed program. Press Escape to cancel.",
        ));
    }

    fn cancel_window_hunter(&mut self) {
        if !self.hunter_armed {
            return;
        }
        self.hunter_armed = false;
        unsafe {
            ReleaseCapture();
            SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_ARROW));
        }
        self.set_status(&t("Window identification cancelled."));
    }

    fn start_uninstall(&mut self, index: usize) {
        let Some(program) = self.visible.get(index).cloned() else {
            return;
        };
        if program.source == programs::Source::Manual {
            self.remove_manual_program(program);
            return;
        }
        if self.restore_point_receiver.is_some() {
            return;
        }
        let command = match self.uninstall_command(&program) {
            Ok(command) => command,
            Err(error) => {
                self.set_status(&error);
                show_error(self.hwnd, &error);
                return;
            }
        };
        let prompt = if program.source == programs::Source::Store {
            tf(
                "InstallScope will ask Windows to uninstall {name} for this user. Any leftover cleanup will be presented for review afterward.\r\n\r\nContinue?",
                &[("name", program.name.clone())],
            )
        } else {
            tf(
                "InstallScope will start {name}'s own uninstaller. Any leftover cleanup will be presented for review afterward.\r\n\r\nContinue?",
                &[("name", program.name.clone())],
            )
        };
        let confirmed = unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                self.hwnd,
                wide(&prompt).as_ptr(),
                wide("Confirm uninstall").as_ptr(),
                MB_YESNO | MB_ICONWARNING,
            ) == 6
        };
        if !confirmed {
            return;
        }
        if self.create_restore_point && self.elevated {
            self.begin_restore_point(PendingUninstall::Single { program, command });
            return;
        }
        if let Err(error) = self.launch_uninstall(program, command) {
            let message = format!("{error:#}");
            self.set_status(&message);
            show_error(self.hwnd, &message);
        }
    }

    fn launch_uninstall(
        &mut self,
        program: InstalledProgram,
        command: Option<String>,
    ) -> Result<()> {
        if program.source == programs::Source::Store {
            let package = program.key.clone();
            let window = self.hwnd as isize;
            let (tx, rx) = mpsc::channel();
            self.store_uninstall_receiver = Some(rx);
            self.uninstall_program = Some(program.clone());
            self.view = View::Uninstalling;
            self.layout_controls();
            self.set_status(&tf("Removing {name}…", &[("name", program.name)]));
            std::thread::spawn(move || {
                let result =
                    programs::remove_store_app(&package).map_err(|error| format!("{error:#}"));
                let _ = tx.send(result);
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                        window as HWND,
                        WM_STORE_UNINSTALL_READY,
                        0,
                        0,
                    )
                };
            });
            return Ok(());
        }
        let command = command.context("The program has no registered uninstaller")?;
        match platform::spawn_command_line(&platform::expand_env(&command)) {
            Ok(child) => {
                self.uninstall_program = Some(program.clone());
                self.uninstall_child = Some(child);
                self.uninstall_exit = None;
                self.view = View::Uninstalling;
                let timer = unsafe {
                    ShowWindow(self.uninstall_button, SW_HIDE);
                    SetTimer(self.hwnd, TIMER_UNINSTALL, 500, None)
                };
                if timer == 0 {
                    let error = std::io::Error::last_os_error();
                    self.uninstall_program = None;
                    self.uninstall_child = None;
                    self.uninstall_exit = None;
                    self.view = View::Programs;
                    self.layout_controls();
                    return Err(anyhow!(
                        "The uninstaller started, but InstallScope could not monitor it ({error}). Check whether it completed before trying again."
                    ));
                }
                self.set_status(&tf(
                    "Waiting for {name}'s uninstaller…",
                    &[("name", program.name)],
                ));
                self.layout_controls();
                Ok(())
            }
            Err(error) => Err(error).context("Could not start the uninstaller"),
        }
    }

    /// Keep the field in sync with the checkbox (BS_AUTOCHECKBOX toggles
    /// itself; the field is the value the uninstall flow reads).
    fn sync_restore_point_check(&mut self) {
        let checked = unsafe { SendMessageW(self.restore_point_check, BM_GETCHECK, 0, 0) };
        self.create_restore_point = checked != 0;
    }

    /// Create the pre-uninstall restore point on a background thread (the
    /// same work the full interface's wizard does before it starts), then
    /// continue with `pending`. Runs at most once per uninstall or batch.
    fn begin_restore_point(&mut self, pending: PendingUninstall) {
        self.pending_uninstall = Some(pending);
        self.view = View::RestorePoint;
        self.layout_controls();
        self.set_status(&t("Creating a restore point…"));
        let window = self.hwnd as isize;
        let (tx, rx) = mpsc::channel();
        self.restore_point_receiver = Some(rx);
        std::thread::spawn(move || {
            let result =
                installscope_core::restore::create_restore_point("Before InstallScope uninstall")
                    .map_err(|error| format!("{error:#}"));
            let _ = tx.send(result);
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                    window as HWND,
                    WM_RESTORE_POINT_READY,
                    0,
                    0,
                )
            };
        });
    }

    fn finish_restore_point(&mut self) {
        let Some(receiver) = self.restore_point_receiver.take() else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(())) => self.resume_after_restore_point(),
            Ok(Err(error)) => self.cancel_after_failed_restore_point(&tf(
                "Restore point not created; uninstall canceled: {error}",
                &[("error", error)],
            )),
            Err(mpsc::TryRecvError::Empty) => self.restore_point_receiver = Some(receiver),
            Err(mpsc::TryRecvError::Disconnected) => self.cancel_after_failed_restore_point(&t(
                "Restore point not created; uninstall canceled.",
            )),
        }
    }

    /// A failed restore point cancels the pending uninstall, matching the
    /// main interface: the user asked for the safety net first, so the
    /// operation does not run without it.
    fn cancel_after_failed_restore_point(&mut self, note: &str) {
        self.pending_uninstall.take();
        if self.batch_active {
            self.abort_batch(note);
        } else {
            self.view = View::Programs;
            self.layout_controls();
            self.set_status(note);
            show_error(self.hwnd, note);
        }
    }

    fn resume_after_restore_point(&mut self) {
        match self.pending_uninstall.take() {
            Some(PendingUninstall::Single { program, command }) => {
                if let Err(error) = self.launch_uninstall(program, command) {
                    let message = format!("{error:#}");
                    self.view = View::Programs;
                    self.layout_controls();
                    self.set_status(&message);
                    show_error(self.hwnd, &message);
                }
            }
            Some(PendingUninstall::Batch) => self.launch_next_batch_program(),
            None => {
                self.view = View::Programs;
                self.layout_controls();
            }
        }
    }

    fn finish_store_uninstall(&mut self) {
        let Some(receiver) = self.store_uninstall_receiver.take() else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(_)) => {
                if let Some(program) = self.uninstall_program.clone() {
                    self.start_leftover_scan(program, 0);
                }
            }
            Ok(Err(error)) => {
                self.view = View::Programs;
                self.uninstall_program = None;
                self.layout_controls();
                self.set_status(&error);
                show_error(self.hwnd, &error);
                if self.batch_active {
                    self.abort_batch(&error);
                }
            }
            Err(mpsc::TryRecvError::Empty) => self.store_uninstall_receiver = Some(receiver),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.view = View::Programs;
                self.uninstall_program = None;
                self.layout_controls();
                let error = t("The Store app uninstall stopped unexpectedly.");
                self.set_status(&error);
                show_error(self.hwnd, &error);
                if self.batch_active {
                    self.abort_batch(&error);
                }
            }
        }
    }

    fn poll_uninstaller(&mut self) {
        let Some(program) = self.uninstall_program.clone() else {
            unsafe { KillTimer(self.hwnd, TIMER_UNINSTALL) };
            return;
        };
        if self.uninstall_exit.is_none() {
            if let Some(code) = self
                .uninstall_child
                .as_ref()
                .and_then(|child| child.try_wait())
            {
                self.uninstall_exit = Some((code, Instant::now()));
                self.uninstall_child = None;
            }
        }
        if self.uninstall_exit.is_some() && !self.registry.exists(&program.key) {
            let exit_code = self
                .uninstall_exit
                .map(|(code, _)| code)
                .unwrap_or_default();
            unsafe { KillTimer(self.hwnd, TIMER_UNINSTALL) };
            self.start_leftover_scan(program, exit_code);
            return;
        }
        if self
            .uninstall_exit
            .is_some_and(|(_, exited)| exited.elapsed() >= std::time::Duration::from_secs(60))
        {
            unsafe { KillTimer(self.hwnd, TIMER_UNINSTALL) };
            self.uninstall_program = None;
            self.uninstall_child = None;
            self.uninstall_exit = None;
            self.view = View::Programs;
            self.layout_controls();
            let error = tf(
                "{name} is still registered; InstallScope did not scan or remove leftovers.",
                &[("name", program.name)],
            );
            self.set_status(&error);
            if self.batch_active {
                self.abort_batch(&error);
            }
            return;
        }
        self.set_status(&tf(
            "Waiting for {name} to finish uninstalling…",
            &[("name", program.name)],
        ));
    }

    fn start_leftover_scan(&mut self, program: InstalledProgram, exit_code: u32) {
        self.system_orphan_scan = false;
        self.view = View::Scanning;
        self.layout_controls();
        self.set_status(&tf(
            "Scanning leftovers for {name}…",
            &[("name", program.name.clone())],
        ));
        let registry = self.registry.clone();
        let logs = self.logs.clone();
        let advanced_mode = self.advanced_mode;
        let window = self.hwnd as isize;
        let (tx, rx) = mpsc::channel();
        self.plan_receiver = Some(rx);
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut plan = build_native_removal_plan(
                    &program,
                    &logs,
                    registry.as_ref(),
                    advanced_mode,
                );
                if exit_code != 0 {
                    plan.info.push(tf(
                        "The uninstaller exited with code {code}; review its results before removing leftovers.",
                        &[("code", exit_code.to_string())],
                    ));
                }
                plan
            }))
            .map_err(|_| t("Could not scan for leftover files and settings."));
            let _ = tx.send(result);
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                    window as HWND,
                    WM_PLAN_READY,
                    0,
                    0,
                )
            };
        });
    }

    fn start_system_orphan_scan(&mut self) {
        if self.view != View::Programs
            || self.plan_receiver.is_some()
            || self.remove_receiver.is_some()
        {
            return;
        }
        self.system_orphan_scan = true;
        self.view = View::Scanning;
        self.layout_controls();
        self.set_status(&t("Scanning for leftovers of removed programs…"));
        let registry = self.registry.clone();
        let window = self.hwnd as isize;
        let (tx, rx) = mpsc::channel();
        self.plan_receiver = Some(rx);
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut plan = installscope_core::orphans::scan_system(registry.as_ref());
                plan.info.push(t(
                    "Uninstall registrations are proposed only when both the install folder and every registered uninstaller are missing. MSI, hidden, protected, and Windows-owned entries are excluded.",
                ));
                plan
            }))
            .map_err(|_| t("Could not scan for leftovers of removed programs."));
            let _ = tx.send(result);
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                    window as HWND,
                    WM_PLAN_READY,
                    0,
                    0,
                )
            };
        });
    }

    fn finish_plan(&mut self) {
        let Some(receiver) = self.plan_receiver.take() else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(plan)) => {
                self.system_orphan_scan = false;
                self.plan = Some(plan);
                self.selected_plan_item = self
                    .plan
                    .as_ref()
                    .and_then(|plan| plan.items.iter().position(|item| item.exists));
                self.view = View::Review;
                self.populate_plan_list();
                self.layout_controls();
                self.set_status(&t(
                    "Review the findings. Nothing will be removed until you choose Remove selected.",
                ));
            }
            Ok(Err(error)) => {
                let scan_error = if self.system_orphan_scan {
                    t("Could not scan for leftovers of removed programs.")
                } else {
                    error
                };
                self.system_orphan_scan = false;
                self.view = View::Programs;
                self.uninstall_program = None;
                self.filter_programs();
                self.layout_controls();
                self.set_status(&scan_error);
                show_error(self.hwnd, &scan_error);
                if self.batch_active {
                    self.abort_batch(&scan_error);
                }
            }
            Err(mpsc::TryRecvError::Empty) => self.plan_receiver = Some(receiver),
            Err(mpsc::TryRecvError::Disconnected) => {
                let error = if self.system_orphan_scan {
                    t("The system-wide leftover scan stopped unexpectedly.")
                } else {
                    t("The leftover scan stopped unexpectedly.")
                };
                self.system_orphan_scan = false;
                self.view = View::Programs;
                self.uninstall_program = None;
                self.filter_programs();
                self.layout_controls();
                self.set_status(&error);
                show_error(self.hwnd, &error);
                if self.batch_active {
                    self.abort_batch(&error);
                }
            }
        }
    }

    fn populate_plan_list(&mut self) {
        let Some(plan) = &self.plan else {
            return;
        };
        self.labels = plan
            .items
            .iter()
            .map(|item| {
                let confidence = revert::confidence(item);
                format!(
                    "{} [{} {}%] {}",
                    if item.selected { "[x]" } else { "[ ]" },
                    t(confidence.band().label()),
                    confidence.score,
                    item.action.describe()
                )
            })
            .collect();
        unsafe {
            SendMessageW(self.list, LB_RESETCONTENT, 0, 0);
            for label in &self.labels {
                let row = wide(label);
                SendMessageW(self.list, LB_ADDSTRING, 0, row.as_ptr() as LPARAM);
            }
            if let Some(index) = self.selected_plan_item {
                SendMessageW(self.list, LB_SETCURSEL, index, 0);
                self.show_plan_item(index);
            } else {
                let info = self
                    .plan
                    .as_ref()
                    .map(|plan| plan.info.join("\r\n"))
                    .unwrap_or_default();
                let text = if info.is_empty() {
                    t("No leftovers were found.")
                } else {
                    format!("{}\r\n\r\n{}", t("No leftovers were found."), info)
                };
                SetWindowTextW(self.details, wide(&text).as_ptr());
            }
        }
        if plan.items.is_empty() {
            self.set_status(&t("No leftovers were found."));
        }
    }

    fn show_plan_item(&self, index: usize) {
        let Some(item) = self.plan.as_ref().and_then(|plan| plan.items.get(index)) else {
            return;
        };
        let confidence = revert::confidence(item);
        let info = self
            .plan
            .as_ref()
            .map(|plan| plan.info.join("\r\n"))
            .unwrap_or_default();
        let details = format!(
            "{}\r\n{}\r\n\r\n{}: {}%\r\n{}: {}\r\n{}",
            item.action.describe(),
            info,
            t(confidence.band().label()),
            confidence.score,
            t("Review note"),
            value_or_unknown(&item.note),
            t("This item is not changed unless selected and explicitly removed.")
        );
        unsafe { SetWindowTextW(self.details, wide(&details).as_ptr()) };
    }

    fn toggle_plan_item(&mut self) {
        let Some(index) = self.selected_plan_item else {
            return;
        };
        if let Some(item) = self
            .plan
            .as_mut()
            .and_then(|plan| plan.items.get_mut(index))
        {
            if item.exists {
                item.selected = !item.selected;
            }
        }
        self.populate_plan_list();
    }

    fn start_remove_selected(&mut self) {
        let Some(plan) = self.plan.as_ref() else {
            return;
        };
        let count = plan.selected();
        if count == 0 {
            self.set_status(&t("Select at least one existing item to remove."));
            return;
        }
        let prompt = tf(
            "Remove {n} selected items? InstallScope will back them up first when backups are enabled.",
            &[("n", count.to_string())],
        );
        let confirmed = unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                self.hwnd,
                wide(&prompt).as_ptr(),
                wide("Confirm leftover removal").as_ptr(),
                MB_YESNO | MB_ICONWARNING,
            ) == 6
        };
        if !confirmed {
            return;
        }
        let plan = plan.clone();
        let registry = self.registry.clone();
        let backup_root = self.backup_root.clone();
        let backup_enabled = self.backup_enabled;
        let options = ExecOptions {
            use_recycle_bin: self.recycle_bin,
            restart_explorer: self.restart_explorer,
            close_programs: self.close_programs,
            ..Default::default()
        };
        let window = self.hwnd as isize;
        let (tx, rx) = mpsc::channel();
        self.remove_receiver = Some(rx);
        self.view = View::Removing;
        self.layout_controls();
        self.set_status(&t("Removing selected leftovers…"));
        std::thread::spawn(move || {
            let result = (|| {
                let mut writer = if backup_enabled {
                    Some(BackupWriter::new(&backup_root, "Native reviewed cleanup")?)
                } else {
                    None
                };
                let results =
                    revert::execute_plan(&plan, registry.as_ref(), writer.as_mut(), &options);
                let mut backup_error = None;
                let backup_id = match writer {
                    Some(writer) if writer.is_empty() => {
                        writer.discard();
                        None
                    }
                    Some(writer) => match writer.finish() {
                        Ok(backup) => Some(backup.id),
                        Err(error) => {
                            backup_error = Some(format!("{error:#}"));
                            None
                        }
                    },
                    None => None,
                };
                Ok((results, backup_id, backup_error))
            })()
            .map_err(|error: anyhow::Error| format!("{error:#}"));
            let _ = tx.send(result);
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                    window as HWND,
                    WM_REMOVE_READY,
                    0,
                    0,
                )
            };
        });
    }

    fn finish_remove(&mut self) {
        let Some(receiver) = self.remove_receiver.take() else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok((results, backup_id, backup_error))) => {
                let failures = results.iter().filter(|result| !result.ok).count();
                self.labels = results
                    .iter()
                    .map(|result| {
                        format!(
                            "{} {}",
                            if result.ok { "[ok]" } else { "[!]" },
                            result.description
                        )
                    })
                    .collect();
                let backup = backup_id
                    .map(|id| tf("Backup created: {id}", &[("id", id)]))
                    .unwrap_or_else(|| {
                        if self.backup_enabled {
                            String::new()
                        } else {
                            t("Backups were disabled in InstallScope settings.")
                        }
                    });
                let backup_warning = backup_error
                    .map(|error| tf("Backup finalization failed: {error}", &[("error", error)]))
                    .unwrap_or_default();
                let result_details = results
                    .iter()
                    .map(|result| format!("{}: {}", result.description, result.message))
                    .collect::<Vec<_>>()
                    .join("\r\n");
                let summary = format!(
                    "{}\r\n\r\n{}\r\n{}\r\n\r\n{}",
                    tf(
                        "Removal finished with {n} problem(s).",
                        &[("n", failures.to_string())]
                    ),
                    backup,
                    backup_warning,
                    result_details
                );
                unsafe {
                    SetWindowTextW(self.details, wide(&summary).as_ptr());
                    SendMessageW(self.list, LB_RESETCONTENT, 0, 0);
                    for label in &self.labels {
                        let row = wide(label);
                        SendMessageW(self.list, LB_ADDSTRING, 0, row.as_ptr() as LPARAM);
                    }
                }
                self.view = View::Results;
                self.layout_controls();
                self.set_status(&tf(
                    "Removal finished with {n} problem(s).",
                    &[("n", failures.to_string())],
                ));
            }
            Ok(Err(error)) => {
                self.view = View::Review;
                self.layout_controls();
                self.set_status(&error);
                show_error(self.hwnd, &error);
            }
            Err(mpsc::TryRecvError::Empty) => self.remove_receiver = Some(receiver),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.view = View::Review;
                self.layout_controls();
                let error = t("Leftover removal stopped unexpectedly.");
                self.set_status(&error);
                show_error(self.hwnd, &error);
            }
        }
    }

    fn leave_review(&mut self) {
        self.plan = None;
        self.selected_plan_item = None;
        self.uninstall_program = None;
        self.view = View::Programs;
        self.layout_controls();
        if self.batch_active {
            self.batch_queue.remove(0);
            self.batch_completed += 1;
            self.launch_next_batch_program();
        } else {
            self.request_refresh();
        }
    }

    fn layout_controls(&self) {
        unsafe {
            ShowWindow(
                self.uninstall_button,
                if self.view == View::Programs {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
            ShowWindow(
                self.add_manual,
                if self.view == View::Programs {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
            ShowWindow(
                self.restore_point_check,
                if self.view == View::Programs {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
            let show_review = matches!(self.view, View::Review | View::Results);
            for control in [self.toggle_item, self.remove_selected, self.back] {
                ShowWindow(control, if show_review { SW_SHOW } else { SW_HIDE });
            }
            if self.view == View::Review {
                SetWindowTextW(self.back, wide(&t("Skip cleanup")).as_ptr());
                if self.batch_active {
                    SetWindowTextW(self.back, wide(&t("Skip cleanup and continue")).as_ptr());
                }
                EnableWindow(
                    self.remove_selected,
                    i32::from(self.plan.as_ref().is_some_and(|plan| plan.selected() > 0)),
                );
            } else if self.view == View::Results {
                let label = if self.batch_active {
                    if self.batch_queue.len() > 1 {
                        t("Next program")
                    } else {
                        t("Finish batch")
                    }
                } else {
                    t("Back to programs")
                };
                SetWindowTextW(self.back, wide(&label).as_ptr());
            }
            EnableWindow(self.refresh, i32::from(self.view == View::Programs));
            EnableWindow(self.hunter, i32::from(self.view == View::Programs));
            EnableWindow(self.add_manual, i32::from(self.view == View::Programs));
            EnableWindow(
                self.uninstall_button,
                i32::from(self.view == View::Programs),
            );
            EnableWindow(
                self.restore_point_check,
                i32::from(self.view == View::Programs && self.elevated),
            );
        }
        unsafe { layout_controls(self) };
    }

    fn set_status(&self, message: &str) {
        unsafe { SetWindowTextW(self.status, wide(message).as_ptr()) };
    }
}

fn build_native_removal_plan(
    program: &InstalledProgram,
    logs: &[InstallLog],
    registry: &dyn Registry,
    advanced_mode: bool,
) -> Plan {
    let log = programs::log_for(logs, program);
    let mut plan = log
        .map(|log| revert::build_plan(&log.changes, registry))
        .unwrap_or_default();
    let mut heuristic = leftovers::find_leftovers(program, registry, &LeftoverScope::default());
    if log.is_some() {
        for item in &mut heuristic.items {
            if matches!(item.action, revert::Action::DeleteTree { .. }) {
                item.selected = false;
                item.note = "Name match".into();
            }
        }
    }
    super::programs::merge_plans(&mut plan, heuristic);
    plan.items
        .sort_by(|left, right| left.action.category().cmp(right.action.category()));
    if !advanced_mode {
        let hidden = revert::simplify(&mut plan);
        if hidden > 0 {
            plan.info.push(tf(
                "{n} higher-risk or uncertain items were left out of this Simple-mode review. Use the full interface's Advanced mode to inspect them.",
                &[("n", hidden.to_string())],
            ));
        }
    }
    plan
}

/// Return matching programs by name or publisher, preserving the list order.
fn filter_programs(programs: &[InstalledProgram], query: &str) -> Vec<InstalledProgram> {
    let query = query.trim().to_lowercase();
    programs
        .iter()
        .filter(|p| {
            query.is_empty()
                || p.name.to_lowercase().contains(&query)
                || p.publisher.to_lowercase().contains(&query)
        })
        .cloned()
        .collect()
}

fn value_or_unknown(value: &str) -> String {
    if value.is_empty() {
        t("Not available")
    } else {
        value.to_string()
    }
}

fn find_program_for_image<'a>(
    programs: &'a [InstalledProgram],
    image_path: &std::path::Path,
) -> Option<&'a InstalledProgram> {
    let image = normalize_windows_path(&image_path.to_string_lossy());
    let mut matches = programs.iter().filter(|program| {
        let location = normalize_windows_path(&program.install_location);
        !location.is_empty()
            && image
                .strip_prefix(&location)
                .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with('\\'))
    });
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

fn normalize_windows_path(path: &str) -> String {
    path.trim()
        .trim_matches('"')
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

fn process_image_path(process_id: u32) -> Result<std::path::PathBuf> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id);
        if process.is_null() {
            return Err(std::io::Error::last_os_error())
                .context("Could not open the selected process");
        }
        let mut path = vec![0u16; 32_768];
        let mut length = path.len() as u32;
        let success = QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut length);
        let error = if success == 0 {
            Some(std::io::Error::last_os_error())
        } else {
            None
        };
        CloseHandle(process);
        if let Some(error) = error {
            return Err(error).context("Could not read the selected process path");
        }
        path.truncate(length as usize);
        Ok(std::path::PathBuf::from(String::from_utf16_lossy(&path)))
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn rgb((r, g, b): (u8, u8, u8)) -> u32 {
    u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16)
}

unsafe fn draw_menu_label(state: &NativePrograms, draw: &DRAWITEMSTRUCT) {
    let label = match draw.itemData as u16 {
        ID_MENU_LABEL_FILE => t("File"),
        ID_MENU_LABEL_TOOLS => t("Utilities"),
        ID_MENU_LABEL_VIEW => t("View"),
        ID_MENU_LABEL_HELP => t("Help"),
        _ => return,
    };
    let brush = if draw.itemState & (0x0001 | 0x0040) != 0 {
        state.brushes[2]
    } else {
        state.brushes[1]
    };
    FillRect(draw.hDC, &draw.rcItem, brush);
    SetBkMode(draw.hDC, TRANSPARENT as i32);
    SetTextColor(draw.hDC, rgb(COLOR_TEXT));
    let mut rect = draw.rcItem;
    rect.left += 8;
    rect.right -= 8;
    let mut text = wide(&label);
    DrawTextW(
        draw.hDC,
        text.as_mut_ptr(),
        -1,
        &mut rect,
        DT_CENTER | DT_SINGLELINE | DT_VCENTER,
    );
}

pub fn run(install_target: Option<String>, _launch_args: Vec<String>) -> Result<()> {
    if install_target.is_some() {
        return Err(anyhow!(
            "The native Programs preview does not yet support Explorer's Install with InstallScope action. Build without --features native-ui-preview to use the complete interface."
        ));
    }

    let store = Store::open().context("Could not open InstallScope's data folder")?;
    let settings = store.load_settings();
    i18n::set_language(&settings.language);
    let registry: Arc<dyn Registry> = Arc::from(platform::system_registry());
    let mut state = Box::new(NativePrograms {
        hwnd: std::ptr::null_mut(),
        brand_mark: std::ptr::null_mut(),
        brand_name: std::ptr::null_mut(),
        nav_programs: std::ptr::null_mut(),
        nav_items: [std::ptr::null_mut(); 17],
        page_title: std::ptr::null_mut(),
        page_subtitle: std::ptr::null_mut(),
        list_heading: std::ptr::null_mut(),
        table_header: std::ptr::null_mut(),
        footer: std::ptr::null_mut(),
        search_label: std::ptr::null_mut(),
        search: std::ptr::null_mut(),
        list: std::ptr::null_mut(),
        details: std::ptr::null_mut(),
        status: std::ptr::null_mut(),
        refresh: std::ptr::null_mut(),
        hunter: std::ptr::null_mut(),
        add_manual: std::ptr::null_mut(),
        uninstall_button: std::ptr::null_mut(),
        restore_point_check: std::ptr::null_mut(),
        toggle_item: std::ptr::null_mut(),
        remove_selected: std::ptr::null_mut(),
        back: std::ptr::null_mut(),
        hunter_armed: false,
        view: View::Programs,
        registry,
        store_root: store.root.clone(),
        backup_root: store.backups_dir(),
        logs: store.list_logs(),
        include_store_apps: settings.show_store_apps,
        advanced_mode: settings.advanced_mode,
        backup_enabled: settings.backup_enabled(),
        recycle_bin: settings.use_recycle_bin && settings.advanced_mode,
        restart_explorer: settings.restart_explorer,
        close_programs: settings.close_programs,
        create_restore_point: settings.create_restore_point,
        elevated: platform::is_elevated(),
        programs: Vec::new(),
        visible: Vec::new(),
        labels: Vec::new(),
        receiver: None,
        uninstall_program: None,
        batch_queue: Vec::new(),
        batch_active: false,
        batch_total: 0,
        batch_completed: 0,
        uninstall_child: None,
        uninstall_exit: None,
        store_uninstall_receiver: None,
        restore_point_receiver: None,
        pending_uninstall: None,
        plan_receiver: None,
        system_orphan_scan: false,
        remove_receiver: None,
        plan: None,
        selected_plan_item: None,
        brushes: [std::ptr::null_mut(); 5],
        fonts: [std::ptr::null_mut(); 3],
        font: 0,
    });

    let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
    if instance.is_null() {
        return Err(std::io::Error::last_os_error())
            .context("Could not get the application module");
    }
    for (i, color) in [
        COLOR_BG,
        COLOR_PANEL,
        COLOR_CARD,
        COLOR_ACCENT,
        (46, 40, 43),
    ]
    .into_iter()
    .enumerate()
    {
        state.brushes[i] = unsafe { CreateSolidBrush(rgb(color)) };
        if state.brushes[i].is_null() {
            return Err(std::io::Error::last_os_error()).context("Could not create the UI palette");
        }
    }
    state.font = unsafe { GetStockObject(DEFAULT_GUI_FONT) as isize };
    for (index, height, weight) in [
        (0, 21, FW_SEMIBOLD as i32),
        (1, 16, FW_SEMIBOLD as i32),
        (2, 13, 400),
    ] {
        state.fonts[index] = unsafe {
            CreateFontW(
                height,
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                0,
                0,
                5,
                DEFAULT_PITCH as u32,
                wide("Segoe UI").as_ptr(),
            )
        };
        if state.fonts[index].is_null() {
            return Err(std::io::Error::last_os_error()).context("Could not create UI typography");
        }
    }

    let class_name = wide(CLASS_NAME);
    let title = wide(&format!("InstallScope - {}", t("Programs preview")));
    let class = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(window_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: std::ptr::null_mut(),
        hCursor: unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW) },
        hbrBackground: std::ptr::null_mut(),
        lpszMenuName: std::ptr::null(),
        lpszClassName: class_name.as_ptr(),
    };
    if unsafe { RegisterClassW(&class) } == 0 {
        return Err(std::io::Error::last_os_error())
            .context("Could not register the native window");
    }

    let state_ptr = &mut *state as *mut NativePrograms;
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            state_ptr.cast::<c_void>(),
        )
    };
    if hwnd.is_null() {
        return Err(std::io::Error::last_os_error()).context("Could not create the native window");
    }
    unsafe { install_menu(hwnd, state.brushes[1])? };
    unsafe {
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
    }
    platform::apply_window_style(COLOR_ACCENT, true);
    let dark_theme = wide("DarkMode_Explorer");
    unsafe {
        let _ = SetWindowTheme(hwnd, dark_theme.as_ptr(), std::ptr::null());
    }

    let mut message: MSG = unsafe { std::mem::zeroed() };
    loop {
        let result = unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) };
        if result == -1 {
            unsafe { DestroyWindow(hwnd) };
            return Err(std::io::Error::last_os_error()).context("The native message loop failed");
        }
        if result == 0 {
            break;
        }
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}

unsafe fn install_menu(hwnd: HWND, background: HBRUSH) -> Result<()> {
    let menu = CreateMenu();
    let file = CreatePopupMenu();
    let tools = CreatePopupMenu();
    let view = CreatePopupMenu();
    let help = CreatePopupMenu();
    if [menu, file, tools, view, help]
        .iter()
        .any(|handle| handle.is_null())
    {
        return Err(std::io::Error::last_os_error()).context("Could not create the menu bar");
    }
    let append_item = |target, id, label: &str, flags| {
        if AppendMenuW(target, flags, id, wide(label).as_ptr()) == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    };
    append_item(file, ID_MENU_EXIT as usize, &t("Exit"), MF_STRING)?;
    append_item(tools, ID_MENU_REFRESH as usize, &t("Refresh"), MF_STRING)?;
    append_item(
        tools,
        ID_MENU_HUNTER as usize,
        &t("Find by window"),
        MF_STRING,
    )?;
    append_item(
        tools,
        ID_MENU_UNINSTALL as usize,
        &t("Uninstall selected"),
        MF_STRING,
    )?;
    append_item(
        tools,
        ID_MENU_SCAN_ORPHANS as usize,
        &t("Find removed-program leftovers"),
        MF_STRING,
    )?;
    append_item(view, 0, &t("Programs (preview)"), MF_STRING | MF_GRAYED)?;
    append_item(
        help,
        ID_MENU_ABOUT as usize,
        &t("About InstallScope"),
        MF_STRING,
    )?;
    append_menu_label(menu, file, ID_MENU_LABEL_FILE)?;
    append_menu_label(menu, tools, ID_MENU_LABEL_TOOLS)?;
    append_menu_label(menu, view, ID_MENU_LABEL_VIEW)?;
    append_menu_label(menu, help, ID_MENU_LABEL_HELP)?;
    let menu_info = MENUINFO {
        cbSize: std::mem::size_of::<MENUINFO>() as u32,
        fMask: MIM_BACKGROUND | MIM_APPLYTOSUBMENUS,
        dwStyle: 0,
        cyMax: 0,
        hbrBack: background,
        dwContextHelpID: 0,
        dwMenuData: 0,
    };
    if SetMenuInfo(menu, &menu_info) == 0 {
        return Err(std::io::Error::last_os_error()).context("Could not style the menu bar");
    }
    if SetMenu(hwnd, menu) == 0 || DrawMenuBar(hwnd) == 0 {
        return Err(std::io::Error::last_os_error()).context("Could not attach the menu bar");
    }
    Ok(())
}

unsafe fn append_menu_label(menu: HMENU, submenu: HMENU, id: u16) -> Result<()> {
    if AppendMenuW(
        menu,
        MF_POPUP | MF_OWNERDRAW,
        submenu as usize,
        id as usize as *const u16,
    ) == 0
    {
        return Err(std::io::Error::last_os_error()).context("Could not add a menu label");
    }
    Ok(())
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = &*(lparam as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut NativePrograms;
    let state = state_ptr.as_mut();

    match message {
        WM_CREATE => {
            let Some(state) = state else {
                return -1;
            };
            state.hwnd = hwnd;
            if let Err(error) = create_controls(state) {
                show_error(hwnd, &format!("{error:#}"));
                return -1;
            }
            state.request_refresh();
            0
        }
        WM_SIZE => {
            if let Some(state) = state {
                layout_controls(state);
            }
            0
        }
        WM_COMMAND => {
            let Some(state) = state else {
                return DefWindowProcW(hwnd, message, wparam, lparam);
            };
            let id = (wparam & 0xffff) as u16;
            let code = ((wparam >> 16) & 0xffff) as u16;
            match (id, code) {
                (ID_MENU_EXIT, _) => {
                    DestroyWindow(hwnd);
                }
                (ID_MENU_REFRESH, _) => state.request_refresh(),
                (ID_MENU_HUNTER, _) => state.arm_window_hunter(),
                (ID_MENU_UNINSTALL, _) => {
                    state.start_selected_action();
                }
                (ID_MENU_SCAN_ORPHANS, _) => state.start_system_orphan_scan(),
                (ID_MENU_ABOUT, _) => show_info(
                    hwnd,
                    &format!(
                        "InstallScope {}\r\nWindows Install, Uninstall & Developer Tools",
                        installscope_core::APP_VERSION
                    ),
                ),
                (ID_SEARCH, SEARCH_CHANGED) if state.view == View::Programs => {
                    state.filter_programs()
                }
                (ID_REFRESH, BUTTON_CLICKED) if state.view == View::Programs => {
                    state.request_refresh()
                }
                (ID_HUNTER, BUTTON_CLICKED) if state.view == View::Programs => {
                    state.arm_window_hunter();
                }
                (ID_UNINSTALL, BUTTON_CLICKED) if state.view == View::Programs => {
                    state.start_selected_action();
                }
                (ID_RESTORE_POINT, BUTTON_CLICKED) if state.view == View::Programs => {
                    state.sync_restore_point_check();
                }
                (ID_ADD_MANUAL, BUTTON_CLICKED) if state.view == View::Programs => {
                    state.add_manual_program();
                }
                (ID_TOGGLE_ITEM, BUTTON_CLICKED) if state.view == View::Review => {
                    state.toggle_plan_item()
                }
                (ID_REMOVE_SELECTED, BUTTON_CLICKED) if state.view == View::Review => {
                    state.start_remove_selected()
                }
                (ID_BACK, BUTTON_CLICKED) if matches!(state.view, View::Review | View::Results) => {
                    state.leave_review()
                }
                (ID_LIST, LIST_SELECTION_CHANGED) => {
                    let index = SendMessageW(state.list, LB_GETCURSEL, 0, 0);
                    if index >= 0 {
                        match state.view {
                            View::Programs => state.show_selected(index as usize),
                            View::Review => {
                                state.selected_plan_item = Some(index as usize);
                                state.show_plan_item(index as usize);
                            }
                            _ => {}
                        }
                    }
                    if state.view == View::Programs {
                        state.update_selection_action();
                    }
                }
                _ => {}
            }
            0
        }
        WM_PROGRAMS_READY => {
            if let Some(state) = state {
                state.finish_refresh();
            }
            0
        }
        WM_PLAN_READY => {
            if let Some(state) = state {
                state.finish_plan();
            }
            0
        }
        WM_REMOVE_READY => {
            if let Some(state) = state {
                state.finish_remove();
            }
            0
        }
        WM_STORE_UNINSTALL_READY => {
            if let Some(state) = state {
                state.finish_store_uninstall();
            }
            0
        }
        WM_RESTORE_POINT_READY => {
            if let Some(state) = state {
                state.finish_restore_point();
            }
            0
        }
        WM_TIMER if wparam == TIMER_UNINSTALL => {
            if let Some(state) = state {
                state.poll_uninstaller();
            }
            0
        }
        WM_LBUTTONUP if state.as_ref().is_some_and(|state| state.hunter_armed) => {
            if let Some(state) = state {
                state.identify_window_target();
            }
            0
        }
        WM_KEYDOWN
            if wparam == VK_ESCAPE as usize
                && state.as_ref().is_some_and(|state| state.hunter_armed) =>
        {
            if let Some(state) = state {
                state.cancel_window_hunter();
            }
            0
        }
        WM_CAPTURECHANGED if state.as_ref().is_some_and(|state| state.hunter_armed) => {
            if let Some(state) = state {
                state.cancel_window_hunter();
            }
            0
        }
        WM_CANCELMODE if state.as_ref().is_some_and(|state| state.hunter_armed) => {
            if let Some(state) = state {
                state.cancel_window_hunter();
            }
            0
        }
        WM_SETCURSOR if state.as_ref().is_some_and(|state| state.hunter_armed) => {
            SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_CROSS));
            1
        }
        WM_MEASUREITEM => {
            let measure = &mut *(lparam as *mut MEASUREITEMSTRUCT);
            if measure.CtlType == 1 {
                measure.itemWidth = match measure.itemData as u16 {
                    ID_MENU_LABEL_FILE => 48,
                    ID_MENU_LABEL_TOOLS => 76,
                    ID_MENU_LABEL_VIEW => 52,
                    ID_MENU_LABEL_HELP => 52,
                    _ => 52,
                };
                measure.itemHeight = 32;
                return 1;
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_DRAWITEM => {
            let Some(state) = state else {
                return DefWindowProcW(hwnd, message, wparam, lparam);
            };
            let draw = &*(lparam as *const DRAWITEMSTRUCT);
            if draw.CtlType == 1 {
                draw_menu_label(state, draw);
                return 1;
            }
            match draw.CtlID as u16 {
                ID_LIST => {
                    if state.view == View::Programs {
                        draw_program_row(state, draw);
                    } else {
                        draw_text_row(state, draw);
                    }
                    return 1;
                }
                id if (ID_NAV_BASE..ID_NAV_BASE + 13).contains(&id)
                    || matches!(
                        id,
                        ID_BRAND_MARK
                            | ID_BRAND_NAME
                            | ID_PAGE_TITLE
                            | ID_PAGE_SUBTITLE
                            | ID_LIST_HEADING
                            | ID_TABLE_HEADER
                            | ID_FOOTER
                    ) =>
                {
                    draw_owned_label(state, draw);
                    return 1;
                }
                ID_NAV_PROGRAMS | ID_REFRESH | ID_HUNTER | ID_ADD_MANUAL | ID_UNINSTALL
                | ID_TOGGLE_ITEM | ID_REMOVE_SELECTED | ID_BACK => {
                    draw_owned_button(state, draw);
                    return 1;
                }
                _ => {}
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_ERASEBKGND => {
            if let Some(state) = state {
                let mut rect: RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut rect);
                FillRect(wparam as HDC, &rect, state.brushes[0]);
                rect.right = 214;
                FillRect(wparam as HDC, &rect, state.brushes[1]);
                return 1;
            }
            0
        }
        0x0132 | 0x0133 | 0x0134 | 0x0135 | 0x0138 => {
            if let Some(state) = state {
                let dc = wparam as HDC;
                let control = lparam as HWND;
                let sidebar = control == state.brand_name
                    || control == state.brand_mark
                    || control == state.nav_programs;
                let (background, brush) = if sidebar {
                    (COLOR_PANEL, state.brushes[1])
                } else if control == state.search || control == state.details {
                    (COLOR_CARD, state.brushes[2])
                } else {
                    (COLOR_BG, state.brushes[0])
                };
                SetBkColor(dc, rgb(background));
                SetTextColor(dc, rgb(COLOR_TEXT));
                SetBkMode(dc, TRANSPARENT as i32);
                return brush as LRESULT;
            }
            0
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        WM_NCDESTROY => {
            let result = DefWindowProcW(hwnd, message, wparam, lparam);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            result
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

unsafe fn create_controls(state: &mut NativePrograms) -> Result<()> {
    let instance = GetModuleHandleW(std::ptr::null());
    let create = |class: &str, text: &str, style: u32, ex_style: u32, id: u16| -> HWND {
        let class = wide(class);
        let text = wide(text);
        CreateWindowExW(
            ex_style,
            class.as_ptr(),
            text.as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            0,
            0,
            0,
            0,
            state.hwnd,
            id as usize as _,
            instance,
            std::ptr::null(),
        )
    };

    const SS_OWNERDRAW: u32 = 0x000D;
    state.brand_mark = create("STATIC", "E", SS_OWNERDRAW, 0, ID_BRAND_MARK);
    state.brand_name = create("STATIC", "InstallScope", SS_OWNERDRAW, 0, ID_BRAND_NAME);
    state.nav_programs = create(
        "BUTTON",
        "□   Programs",
        BS_OWNERDRAW | WS_TABSTOP,
        0,
        ID_NAV_PROGRAMS,
    );
    let nav_labels = [
        "🏠   Dashboard",
        "📡   Monitor",
        "📋   Install Logs",
        "💾   Backups",
        "🚀   Startup",
        "🗑   Cleaner",
        "Utilities",
        "⏪   System Restore",
        "📸   Snapshots",
        "🎒   PortableApps Forge",
        "📦   Setup Package Creator (Planned)",
        "🧪   Developer Sandbox (Planned)",
        "💿   Temporary Scratch Disk (Planned)",
        "🗂   VSS Viewer (Planned)",
        "🛡   Bloatware Shield",
        "☰   Right-Click Menu",
        "⚙   Settings",
    ];
    for (index, label) in nav_labels.iter().enumerate() {
        state.nav_items[index] = create("STATIC", &t(label), 0x000D, 0, ID_NAV_BASE + index as u16);
    }
    state.page_title = create("STATIC", &t("Programs"), SS_OWNERDRAW, 0, ID_PAGE_TITLE);
    state.page_subtitle = create(
        "STATIC",
        &t("Uninstall programs completely, including everything they leave behind"),
        SS_OWNERDRAW,
        0,
        ID_PAGE_SUBTITLE,
    );
    state.list_heading = create(
        "STATIC",
        &t("Installed programs"),
        SS_OWNERDRAW,
        0,
        ID_LIST_HEADING,
    );
    state.table_header = create(
        "STATIC",
        "Name\tPublisher\tVersion\tInstalled\tSize",
        0x000D,
        0,
        ID_TABLE_HEADER,
    );
    state.footer = create(
        "STATIC",
        &format!(
            "InstallScope {} · © {}",
            installscope_core::APP_VERSION,
            installscope_core::APP_AUTHOR
        ),
        0x000D,
        0,
        ID_FOOTER,
    );
    state.search_label = create("STATIC", &t("Search programs"), SS_LEFT | SS_NOPREFIX, 0, 0);
    state.search = create(
        "EDIT",
        "",
        ES_LEFT | ES_AUTOHSCROLL | WS_TABSTOP,
        0,
        ID_SEARCH,
    );
    state.refresh = create(
        "BUTTON",
        &t("Refresh"),
        BS_OWNERDRAW | WS_TABSTOP,
        0,
        ID_REFRESH,
    );
    state.hunter = create(
        "BUTTON",
        &t("Find by window"),
        BS_OWNERDRAW | WS_TABSTOP,
        0,
        ID_HUNTER,
    );
    state.add_manual = create(
        "BUTTON",
        &t("Add program…"),
        BS_OWNERDRAW | WS_TABSTOP,
        0,
        ID_ADD_MANUAL,
    );
    state.uninstall_button = create(
        "BUTTON",
        &t("Uninstall"),
        BS_OWNERDRAW | WS_TABSTOP,
        0,
        ID_UNINSTALL,
    );
    state.restore_point_check = create(
        "BUTTON",
        &t("Create a System Restore point first"),
        BS_AUTOCHECKBOX | WS_TABSTOP,
        0,
        ID_RESTORE_POINT,
    );
    SendMessageW(
        state.restore_point_check,
        BM_SETCHECK,
        if state.create_restore_point { 1 } else { 0 },
        0,
    );
    EnableWindow(state.restore_point_check, i32::from(state.elevated));
    state.toggle_item = create(
        "BUTTON",
        &t("Toggle selection"),
        BS_OWNERDRAW | WS_TABSTOP,
        0,
        ID_TOGGLE_ITEM,
    );
    state.remove_selected = create(
        "BUTTON",
        &t("Remove selected"),
        BS_OWNERDRAW | WS_TABSTOP,
        0,
        ID_REMOVE_SELECTED,
    );
    state.back = create(
        "BUTTON",
        &t("Back to programs"),
        BS_OWNERDRAW | WS_TABSTOP,
        0,
        ID_BACK,
    );
    ShowWindow(state.toggle_item, SW_HIDE);
    ShowWindow(state.remove_selected, SW_HIDE);
    ShowWindow(state.back, SW_HIDE);
    state.list = create(
        "LISTBOX",
        "",
        (LBS_NOTIFY | LBS_HASSTRINGS | LBS_OWNERDRAWFIXED | LBS_EXTENDEDSEL) as u32
            | WS_VSCROLL
            | WS_TABSTOP,
        0,
        ID_LIST,
    );
    state.details = create(
        "EDIT",
        &t("Select a program to see its details."),
        ES_LEFT | ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY | WS_VSCROLL,
        0,
        0,
    );
    state.status = create("STATIC", &t("Loading…"), SS_LEFT | SS_NOPREFIX, 0, 0);

    for control in [
        state.brand_mark,
        state.brand_name,
        state.nav_programs,
        state.nav_items[0],
        state.nav_items[1],
        state.nav_items[2],
        state.nav_items[3],
        state.nav_items[4],
        state.nav_items[5],
        state.nav_items[6],
        state.nav_items[7],
        state.nav_items[8],
        state.nav_items[9],
        state.nav_items[10],
        state.nav_items[11],
        state.nav_items[12],
        state.nav_items[13],
        state.nav_items[14],
        state.nav_items[15],
        state.nav_items[16],
        state.page_title,
        state.page_subtitle,
        state.list_heading,
        state.table_header,
        state.footer,
        state.search_label,
        state.search,
        state.refresh,
        state.hunter,
        state.add_manual,
        state.uninstall_button,
        state.restore_point_check,
        state.toggle_item,
        state.remove_selected,
        state.back,
        state.list,
        state.details,
        state.status,
    ] {
        if control.is_null() {
            return Err(std::io::Error::last_os_error())
                .context("Could not create a native Programs control");
        }
        SendMessageW(control, 0x0030, state.font as WPARAM, 1);
    }
    SendMessageW(state.brand_name, 0x0030, state.fonts[1] as WPARAM, 1);
    SendMessageW(state.page_title, 0x0030, state.fonts[0] as WPARAM, 1);
    SendMessageW(state.page_subtitle, 0x0030, state.fonts[2] as WPARAM, 1);
    SendMessageW(state.list_heading, 0x0030, state.fonts[1] as WPARAM, 1);
    SendMessageW(state.table_header, 0x0030, state.fonts[2] as WPARAM, 1);
    SendMessageW(state.footer, 0x0030, state.fonts[2] as WPARAM, 1);
    state.font = state.fonts[2] as isize;
    for control in [
        state.search,
        state.refresh,
        state.hunter,
        state.add_manual,
        state.uninstall_button,
        state.restore_point_check,
        state.toggle_item,
        state.remove_selected,
        state.back,
        state.list,
        state.details,
        state.status,
    ] {
        SendMessageW(control, 0x0030, state.font as WPARAM, 1);
    }
    SendMessageW(state.list, 0x01A0, 0, 28);
    let dark_controls = wide("DarkMode_Explorer");
    for control in [
        state.search,
        state.list,
        state.details,
        state.restore_point_check,
    ] {
        let _ = SetWindowTheme(control, dark_controls.as_ptr(), std::ptr::null());
    }
    Ok(())
}

unsafe fn layout_controls(state: &NativePrograms) {
    let mut rect: RECT = std::mem::zeroed();
    GetClientRect(state.hwnd, &mut rect);
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    let sidebar_width = 214;
    let margin = 20;
    let content_left = sidebar_width + margin;
    let content_width = (width - content_left - margin).max(420);
    let row_height = 30;
    let status_height = 32;
    let details_width = (content_width * 38 / 100).clamp(230, 340);
    let gutter = 18;
    let list_width = (content_width - details_width - gutter).max(220);
    let list_top = 206;
    let list_height = (height - list_top - status_height - margin).max(120);

    MoveWindow(state.brand_mark, 18, 18, 32, 32, 1);
    MoveWindow(state.brand_name, 58, 18, 132, 32, 1);
    MoveWindow(state.nav_items[0], 10, 72, sidebar_width - 20, 38, 1);
    MoveWindow(state.nav_programs, 10, 110, sidebar_width - 20, 38, 1);
    for (index, y) in [
        148,
        186,
        224,
        262,
        300,
        342,
        368,
        406,
        444,
        482,
        520,
        558,
        596,
        634,
        672,
        height - 48,
    ]
    .into_iter()
    .enumerate()
    {
        MoveWindow(
            state.nav_items[index + 1],
            10,
            y,
            sidebar_width - 20,
            if index == 5 { 20 } else { 38 },
            1,
        );
    }
    MoveWindow(state.page_title, content_left, 18, content_width, 34, 1);
    MoveWindow(state.page_subtitle, content_left, 56, content_width, 24, 1);
    MoveWindow(state.search_label, content_left, 91, 180, 20, 1);
    MoveWindow(
        state.search,
        content_left,
        112,
        (content_width - 330).max(140),
        row_height,
        1,
    );
    MoveWindow(
        state.add_manual,
        content_left + content_width - 302,
        112,
        104,
        row_height,
        1,
    );
    MoveWindow(
        state.hunter,
        content_left + content_width - 190,
        112,
        110,
        row_height,
        1,
    );
    MoveWindow(
        state.refresh,
        content_left + content_width - 72,
        112,
        72,
        row_height,
        1,
    );
    MoveWindow(state.list_heading, content_left, 164, list_width, 24, 1);
    MoveWindow(
        state.table_header,
        content_left + 8,
        188,
        list_width - 24,
        18,
        1,
    );
    MoveWindow(
        state.list,
        content_left,
        list_top,
        list_width,
        list_height,
        1,
    );
    MoveWindow(
        state.details,
        content_left + list_width + gutter,
        list_top,
        details_width,
        (list_height - 116).max(40),
        1,
    );
    let action_x = content_left + list_width + gutter;
    let action_y = list_top + list_height - 82;
    MoveWindow(
        state.restore_point_check,
        action_x,
        action_y - 28,
        details_width,
        24,
        1,
    );
    MoveWindow(
        state.uninstall_button,
        action_x,
        action_y,
        details_width,
        30,
        1,
    );
    MoveWindow(state.toggle_item, action_x, action_y, details_width, 26, 1);
    MoveWindow(
        state.remove_selected,
        action_x,
        action_y + 28,
        details_width,
        26,
        1,
    );
    MoveWindow(state.back, action_x, action_y + 56, details_width, 26, 1);
    MoveWindow(
        state.status,
        content_left,
        height - status_height,
        content_width - 250,
        status_height,
        1,
    );
    MoveWindow(
        state.footer,
        content_left + content_width - 250,
        height - status_height,
        250,
        status_height,
        1,
    );
}

unsafe fn draw_owned_label(state: &NativePrograms, draw: &DRAWITEMSTRUCT) {
    let mut rect = draw.rcItem;
    let id = draw.CtlID as u16;
    let is_nav = (ID_NAV_BASE..ID_NAV_BASE + 13).contains(&id);
    let background = if id == ID_BRAND_MARK {
        state.brushes[3]
    } else if id == ID_BRAND_NAME || is_nav || id == ID_FOOTER {
        state.brushes[1]
    } else {
        state.brushes[0]
    };
    FillRect(draw.hDC, &rect, background);
    let (font, color) = match id {
        ID_BRAND_MARK => (state.fonts[1], COLOR_TEXT),
        ID_BRAND_NAME => (state.fonts[1], COLOR_TEXT),
        ID_PAGE_TITLE => (state.fonts[0], COLOR_TEXT),
        ID_PAGE_SUBTITLE => (state.fonts[2], COLOR_MUTED),
        ID_LIST_HEADING => (state.fonts[1], COLOR_TEXT),
        ID_TABLE_HEADER | ID_FOOTER => (state.fonts[2], COLOR_MUTED),
        ID_NAV_TOOLS => (state.fonts[2], COLOR_MUTED),
        _ if is_nav => (state.fonts[2], COLOR_TEXT),
        _ => (state.fonts[2], COLOR_TEXT),
    };
    let old_font = SelectObject(draw.hDC, font as _);
    SetBkMode(draw.hDC, TRANSPARENT as i32);
    SetTextColor(draw.hDC, rgb(color));
    if id == ID_TABLE_HEADER {
        let columns = ["Name", "Publisher", "Version", "Installed", "Size"];
        let positions = [0.0_f32, 0.30, 0.52, 0.73, 0.88];
        let width = (rect.right - rect.left) as f32;
        for (index, label) in columns.iter().enumerate() {
            let start = rect.left + (width * positions[index]) as i32;
            let end = rect.left + (width * positions.get(index + 1).copied().unwrap_or(1.0)) as i32;
            let mut cell = rect;
            cell.left = start;
            cell.right = end - 6;
            let text = wide(label);
            DrawTextW(
                draw.hDC,
                text.as_ptr(),
                -1,
                &mut cell,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
            );
        }
        if !old_font.is_null() {
            SelectObject(draw.hDC, old_font);
        }
        return;
    }
    let flags = if id == ID_BRAND_MARK {
        DT_CENTER | DT_VCENTER | DT_SINGLELINE
    } else if id == ID_FOOTER {
        DT_RIGHT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS
    } else {
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS
    };
    if is_nav {
        rect.left += 14;
    }
    let text_len = GetWindowTextLengthW(draw.hwndItem).clamp(0, 1024) as usize;
    let mut text = vec![0u16; text_len + 1];
    let read = GetWindowTextW(draw.hwndItem, text.as_mut_ptr(), text.len() as i32);
    DrawTextW(draw.hDC, text.as_ptr(), read, &mut rect, flags);
    if !old_font.is_null() {
        SelectObject(draw.hDC, old_font);
    }
}

unsafe fn draw_owned_button(state: &NativePrograms, draw: &DRAWITEMSTRUCT) {
    let id = draw.CtlID as u16;
    let primary = matches!(id, ID_UNINSTALL | ID_REMOVE_SELECTED);
    let active_nav = id == ID_NAV_PROGRAMS;
    let disabled = draw.itemState & 0x0004 != 0;
    let hovered = draw.itemState & 0x0040 != 0;
    let brush = if disabled {
        state.brushes[1]
    } else if primary || active_nav {
        state.brushes[3]
    } else if hovered {
        state.brushes[4]
    } else {
        state.brushes[2]
    };
    FillRect(draw.hDC, &draw.rcItem, brush);
    SetBkMode(draw.hDC, TRANSPARENT as i32);
    SetTextColor(
        draw.hDC,
        rgb(if disabled { COLOR_MUTED } else { COLOR_TEXT }),
    );
    let text_len = GetWindowTextLengthW(draw.hwndItem).clamp(0, 1024) as usize;
    let mut text = vec![0u16; text_len + 1];
    let read = GetWindowTextW(draw.hwndItem, text.as_mut_ptr(), text.len() as i32);
    let mut rect = draw.rcItem;
    let flags = if active_nav {
        rect.left += 16;
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS
    } else {
        DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS
    };
    DrawTextW(draw.hDC, text.as_ptr(), read, &mut rect, flags);
    if draw.itemState & 0x0010 != 0 {
        DrawFocusRect(draw.hDC, &draw.rcItem);
    }
}

unsafe fn draw_program_row(state: &NativePrograms, draw: &DRAWITEMSTRUCT) {
    if draw.itemID == u32::MAX {
        return;
    }
    let selected = draw.itemState & 1 != 0;
    let brush = if selected {
        state.brushes[3]
    } else {
        state.brushes[2]
    };
    FillRect(draw.hDC, &draw.rcItem, brush);
    SetTextColor(draw.hDC, rgb(COLOR_TEXT));
    SetBkMode(draw.hDC, TRANSPARENT as i32);
    let Some(program) = state.visible.get(draw.itemID as usize) else {
        return;
    };
    let columns = [
        program.name.clone(),
        program.publisher.clone(),
        program.version.clone(),
        program.install_date.clone(),
        (program.size_kb > 0)
            .then(|| installscope_core::model::human_size(program.size_kb.saturating_mul(1024)))
            .unwrap_or_default(),
    ];
    let positions = [0.0_f32, 0.30, 0.52, 0.73, 0.88];
    let width = (draw.rcItem.right - draw.rcItem.left) as f32;
    for (index, value) in columns.iter().enumerate() {
        let mut cell = draw.rcItem;
        cell.left = draw.rcItem.left + 8 + (width * positions[index]) as i32;
        cell.right = draw.rcItem.left
            + (width * positions.get(index + 1).copied().unwrap_or(1.0)) as i32
            - 8;
        let text = wide(value);
        DrawTextW(
            draw.hDC,
            text.as_ptr(),
            -1,
            &mut cell,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
    }
    if draw.itemState & 0x0010 != 0 {
        DrawFocusRect(draw.hDC, &draw.rcItem);
    }
}

unsafe fn draw_text_row(state: &NativePrograms, draw: &DRAWITEMSTRUCT) {
    if draw.itemID == u32::MAX {
        return;
    }
    FillRect(
        draw.hDC,
        &draw.rcItem,
        if draw.itemState & 1 != 0 {
            state.brushes[3]
        } else {
            state.brushes[2]
        },
    );
    SetTextColor(draw.hDC, rgb(COLOR_TEXT));
    SetBkMode(draw.hDC, TRANSPARENT as i32);
    if let Some(label) = state.labels.get(draw.itemID as usize) {
        let label = wide(label);
        let mut text_rect = draw.rcItem;
        text_rect.left += 8;
        DrawTextW(
            draw.hDC,
            label.as_ptr(),
            -1,
            &mut text_rect,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
    }
    if draw.itemState & 0x0010 != 0 {
        DrawFocusRect(draw.hDC, &draw.rcItem);
    }
}

fn show_error(hwnd: HWND, message: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    let title = wide("InstallScope");
    unsafe {
        MessageBoxW(
            hwnd,
            wide(message).as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        )
    };
}

fn show_info(hwnd: HWND, message: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_OK};
    unsafe {
        MessageBoxW(
            hwnd,
            wide(message).as_ptr(),
            wide("About InstallScope").as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        )
    };
}

#[cfg(test)]
mod tests {
    use super::{build_native_removal_plan, filter_programs, find_program_for_image};
    use installscope_core::model::{ChangeSet, InstallLog, KeyData};
    use installscope_core::platform::{MemRegistry, Registry};
    use installscope_core::programs::InstalledProgram;

    fn program(name: &str, publisher: &str) -> InstalledProgram {
        InstalledProgram {
            name: name.into(),
            publisher: publisher.into(),
            ..Default::default()
        }
    }

    #[test]
    fn search_matches_name_and_publisher_case_insensitively() {
        let programs = vec![
            program("Photo Editor", "Acme"),
            program("Archive Tool", "Example Corp"),
        ];
        assert_eq!(filter_programs(&programs, "photo")[0].name, "Photo Editor");
        assert_eq!(
            filter_programs(&programs, "EXAMPLE")[0].name,
            "Archive Tool"
        );
    }

    #[test]
    fn empty_search_returns_all_and_nonmatching_search_returns_none() {
        let programs = vec![program("Photo Editor", "Acme")];
        assert_eq!(filter_programs(&programs, "  ").len(), 1);
        assert!(filter_programs(&programs, "missing").is_empty());
    }

    #[test]
    fn window_matching_requires_a_unique_install_location_prefix() {
        let mut first = program("Editor", "Acme");
        first.install_location = "C:\\Program Files\\Acme".into();
        let mut second = program("Unrelated", "Example");
        second.install_location = "C:\\Tools\\Other".into();
        let programs = vec![first.clone(), second];
        assert_eq!(
            find_program_for_image(
                &programs,
                std::path::Path::new("c:\\PROGRAM FILES\\ACME\\editor.exe")
            )
            .map(|program| program.name.as_str()),
            Some("Editor")
        );
        assert!(find_program_for_image(
            &programs,
            std::path::Path::new("C:\\Program Files\\Acme2\\app.exe")
        )
        .is_none());
        let mut duplicate = first.clone();
        duplicate.name = "Second registration".into();
        assert!(find_program_for_image(
            &[first, duplicate],
            std::path::Path::new("C:\\Program Files\\Acme\\editor.exe")
        )
        .is_none());
    }

    #[test]
    fn monitored_uninstall_review_includes_logged_changes_and_keeps_safe_items_selected() {
        let registry = MemRegistry::new();
        let key = "HKCU\\Software\\Acme\\Editor";
        registry.put(KeyData::new(key));
        let program = InstalledProgram {
            key: "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Editor".into(),
            name: "Editor".into(),
            ..Default::default()
        };
        let logs = vec![InstallLog {
            name: program.name.clone(),
            uninstall_key: program.key.clone(),
            changes: ChangeSet {
                keys_added: vec![KeyData::new(key)],
                ..Default::default()
            },
            ..Default::default()
        }];

        let plan = build_native_removal_plan(&program, &logs, &registry, false);

        assert!(plan.items.iter().any(|item| {
            matches!(&item.action, installscope_core::revert::Action::DeleteKey { path } if path == key)
                && item.selected
        }));
        assert!(!registry.exists(&program.key));
    }
}
