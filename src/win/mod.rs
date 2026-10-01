//! Windows shell: a widget embedded in the taskbar, a tray icon as fallback, and the
//! polling loop that feeds them.
//!
//! The widget is a layered child window of `Shell_TrayWnd`, so it shares an input queue
//! with Explorer: nothing on the UI thread may block. Network requests run on a worker
//! thread and come back through `WM_APP_FETCHED`.

mod autostart;
mod http;
mod popup;
mod render;

use std::cell::RefCell;
use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetLastInputInfo, ReleaseCapture, SetCapture, LASTINPUTINFO, VK_ESCAPE,
};
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::alert::{self, Alert};
use crate::cache::{self, Cache, Entry, Stored};
use crate::model::{FetchError, Provider, Thresholds, Usage};
use crate::panel::{self, Panel};
use crate::providers::{self, Env};
use crate::schedule;
use crate::settings::Settings;
use crate::view::{self, Row, Severity, View};
use render::{draw_text, Align, Canvas, Color, Font, Surface};

const WM_APP_TRAY: u32 = WM_APP + 1;
const WM_APP_FETCHED: u32 = WM_APP + 2;
const PBT_APMRESUMEAUTOMATIC: usize = 0x12;
const TICK_TIMER: usize = 1;
const TICK_MS: u32 = 2000;
const TRAY_ID: u32 = 1;

const CMD_CLAUDE: u32 = 1;
const CMD_CODEX: u32 = 2;
const CMD_USED: u32 = 3;
const CMD_REMAINING: u32 = 4;
const CMD_REFRESH: u32 = 5;
const CMD_AUTOSTART: u32 = 6;
const CMD_NOTIFY: u32 = 7;
const CMD_EXIT: u32 = 9;

/// Seconds to wait after a credential problem: re-checking is only a file read.
const CREDENTIAL_RETRY_SECS: i64 = 15;
const MAX_BACKOFF_SECS: i64 = 960;
/// Shortest pause after the service says we are asking too often. Retrying sooner was
/// seen to keep the limit in place for ten minutes and more.
const RATE_LIMIT_MIN_SECS: i64 = 300;
/// Longest pause honoured from a Retry-After header.
const RATE_LIMIT_MAX_SECS: i64 = 3600;
/// The HTTP timeouts add up to well under this.
const FETCH_STUCK_SECS: i64 = 120;
/// With no keyboard or mouse input for this long, nobody is looking at the widget.
const IDLE_AFTER_SECS: i64 = 300;

// Widget layout in 96-dpi pixels, left to right: grip, provider badge, row label, bar, text.
const PAD_L: f32 = 2.0;
const GRIP_W: f32 = 8.0;
const BADGE_X: f32 = 14.0;
const BADGE_W: f32 = 12.0;
const LABEL_X: f32 = 30.0;
const LABEL_W: f32 = 18.0;
const BAR_X: f32 = 49.0;
const BAR_W: f32 = 96.0;
const TEXT_X: f32 = 151.0;
const TEXT_W: f32 = 54.0;
const WIDTH: f32 = 207.0;
const BAR_H: f32 = 9.0;
const BAR_RADIUS: f32 = 1.5;
const ROW_GAP: f32 = 6.0;
const FONT_PX: f32 = 10.0;
const BADGE_FONT_PX: f32 = 12.0;

const FILL_OK: Color = Color::rgb(34, 197, 94);
const FILL_WARN: Color = Color::rgb(251, 191, 36);
const FILL_CRIT: Color = Color::rgb(239, 68, 68);
/// How much stale numbers are dimmed.
const STALE_FADE: f32 = 0.55;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}
/// Result handed from the worker thread to the UI thread.
static FETCHED: Mutex<Option<(Provider, Result<Usage, FetchError>)>> = Mutex::new(None);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Copies `text` into a fixed-size UTF-16 field, truncating and keeping the terminator.
fn copy_wide(field: &mut [u16], text: &str) {
    let limit = field.len() - 1;
    for (slot, unit) in field.iter_mut().zip(text.encode_utf16().take(limit)) {
        *slot = unit;
    }
}

fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default())
}

fn app_data_dir() -> PathBuf {
    std::env::var_os("APPDATA").map_or_else(home_dir, PathBuf::from)
}

fn provider_env() -> Arc<Env> {
    Arc::new(Env { home: home_dir(), claude_code_dir: app_data_dir().join("Claude").join("claude-code") })
}

/// Seconds since the last keyboard or mouse input anywhere on the desktop.
fn idle_secs() -> i64 {
    let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    unsafe {
        if GetLastInputInfo(&mut info) == 0 {
            return 0;
        }
        // Both are 32-bit millisecond tick counts that wrap together.
        (GetTickCount().wrapping_sub(info.dwTime) / 1000) as i64
    }
}

/// Runs `f` on the app state. Returns `None` when the state is already borrowed, which
/// happens when a Win32 call made from inside a handler re-enters a window procedure.
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| cell.try_borrow_mut().ok()?.as_mut().map(f))
}

/// What is currently on screen; repainting is skipped while this is unchanged.
#[derive(Clone, PartialEq)]
struct Shown {
    view: View,
    light: bool,
}

#[derive(Clone, Copy, PartialEq, Default)]
struct Geom {
    /// Widget position in taskbar client coordinates.
    x: i32,
    w: i32,
    h: i32,
    /// X at which the widget sits flush against the tray; the saved offset counts back from it.
    anchor: i32,
    scale: f32,
}

struct Drag {
    grip: bool,
    moved: bool,
    start_cursor: i32,
    start_x: i32,
}

/// The detail panel while it is open.
struct Popup {
    hwnd: HWND,
    surface: Surface,
    scratch: Surface,
    scale: f32,
    /// What the surface currently holds; repainting is skipped while this is unchanged.
    shown: Option<(Panel, bool)>,
}

struct App {
    main: HWND,
    widget: HWND,
    geom: Geom,
    surface: Option<Surface>,
    scratch: Option<Surface>,
    settings: Settings,
    settings_path: PathBuf,
    cache_path: PathBuf,
    error_path: PathBuf,
    env: Arc<Env>,
    // Per provider, indexed by `Provider::index`: last good reading, last error,
    // earliest time of the next scheduled request, the current error backoff, and
    // when the service last answered at all.
    cache: Cache,
    error: [Option<FetchError>; 2],
    next_poll: [i64; 2],
    backoff: [i64; 2],
    last_response: [i64; 2],
    /// End of a wait the service imposed (rate limit, rejected token); 0 when none.
    /// Kept apart from `next_poll` because nothing may override it, and saved to disk
    /// so that a restart does not cut it short.
    blocked_until: [i64; 2],
    fetching: bool,
    fetch_started: i64,
    light: bool,
    shown: Option<Shown>,
    drag: Option<Drag>,
    icon: HICON,
    popup: Option<Popup>,
    popup_closed_at: Option<Instant>,
}

/// Fetches both providers once and prints the result. `iusage --check > out.txt`
pub fn check() {
    use std::io::Write;
    let (env, now) = (provider_env(), unix_now());
    let mut out = std::io::stdout();
    for provider in Provider::ALL {
        let line = match providers::fetch(provider, &env, now, http::get) {
            Ok(usage) => {
                let v = view::build(provider, Some(&usage), None, false, Thresholds::default(), now);
                format!("{}: 5h {} | 7d {}", provider.name(), v.rows[0].text, v.rows[1].text)
            }
            Err(error) => format!("{}: {error:?}", provider.name()),
        };
        let _ = writeln!(out, "{line}");
    }
}

pub fn run() {
    unsafe {
        let mutex = CreateMutexW(null(), 1, wide("Local\\IUsage.SingleInstance").as_ptr());
        if mutex.is_null() || GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }

        let instance = GetModuleHandleW(null());
        let main_class = wide("IUsageMain");
        let widget_class = wide("IUsageWidget");
        let popup_class = wide("IUsagePopup");
        let classes: [(&Vec<u16>, WNDPROC); 3] =
            [(&main_class, Some(main_proc)), (&widget_class, Some(widget_proc)), (&popup_class, Some(popup_proc))];
        for (name, proc) in classes {
            let mut class: WNDCLASSW = std::mem::zeroed();
            class.lpfnWndProc = proc;
            class.hInstance = instance;
            class.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
            class.lpszClassName = name.as_ptr();
            RegisterClassW(&class);
        }

        // A hidden top-level window: broadcasts such as TaskbarCreated only reach those.
        let main = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            main_class.as_ptr(),
            wide("IUsage").as_ptr(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if main.is_null() {
            return;
        }
        TASKBAR_CREATED.store(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()), Ordering::Relaxed);

        let settings_dir = app_data_dir().join("IUsage");
        let settings_path = settings_dir.join("settings.json");
        let cache_path = settings_dir.join("cache.json");
        let settings = Settings::load(&settings_path);
        let Stored { cache, blocked_until } = cache::load(&cache_path);
        // A reading that is still fresh is shown as is; nothing is requested until it
        // ages out. Times in the future are capped so a clock that was set back cannot
        // stall us. A wait the service imposed on an earlier run carries over as loaded.
        let now = unix_now();
        let poll = settings.poll_secs as i64;
        let mut next_poll = [0i64; 2];
        let mut last_response = [0i64; 2];
        for i in 0..2 {
            if let Some(entry) = cache[i] {
                next_poll[i] = (entry.fetched_at + poll).min(now + poll);
                last_response[i] = entry.fetched_at.min(now);
            }
        }
        let app = App {
            main,
            widget: null_mut(),
            geom: Geom::default(),
            surface: None,
            scratch: None,
            settings,
            settings_path,
            cache_path,
            error_path: settings_dir.join("last-error.txt"),
            env: provider_env(),
            cache,
            error: [None, None],
            next_poll,
            backoff: [0, 0],
            last_response,
            blocked_until,
            fetching: false,
            fetch_started: 0,
            light: system_uses_light_theme(),
            shown: None,
            drag: None,
            icon: null_mut(),
            popup: None,
            popup_closed_at: None,
        };
        APP.with(|cell| *cell.borrow_mut() = Some(app));
        with_app(|app| {
            app.set_tray(NIM_ADD);
            app.tick();
        });
        SetTimer(main, TICK_TIMER, TICK_MS, None);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        with_app(|app| app.shutdown());
    }
}

unsafe extern "system" fn main_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER if wparam == TICK_TIMER => {
            with_app(|app| app.tick());
            0
        }
        WM_APP_FETCHED => {
            with_app(|app| app.on_fetched());
            0
        }
        WM_APP_TRAY => {
            match lparam as u32 & 0xFFFF {
                WM_LBUTTONUP => {
                    with_app(|app| app.toggle_popup());
                }
                WM_RBUTTONUP => menu(hwnd),
                _ => {}
            }
            0
        }
        WM_POWERBROADCAST => {
            if wparam == PBT_APMRESUMEAUTOMATIC {
                // Timers do not count time spent asleep: fetch as soon as we are back.
                // The minimum gap and any wait the service imposed still apply.
                with_app(|app| app.next_poll = [0, 0]);
            }
            1
        }
        WM_SETTINGCHANGE => {
            with_app(|app| {
                app.light = system_uses_light_theme();
                app.refresh();
            });
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ if msg != 0 && msg == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            // Explorer restarted: the tray icon has to be registered again, and the old
            // widget died with the old taskbar. `tick` notices a dead widget by itself;
            // forgetting a live one here would leave a second widget on the taskbar,
            // because this broadcast also arrives when the taskbar was not replaced.
            with_app(|app| {
                app.set_tray(NIM_ADD);
                app.tick();
            });
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe extern "system" fn widget_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let client_x = (lparam & 0xFFFF) as i16 as i32;
    match msg {
        WM_LBUTTONDOWN => {
            with_app(|app| app.on_down(client_x));
            0
        }
        WM_MOUSEMOVE => {
            with_app(|app| app.on_move());
            0
        }
        WM_LBUTTONUP => {
            with_app(|app| app.on_up(client_x));
            0
        }
        WM_CAPTURECHANGED => {
            with_app(|app| app.on_capture_lost());
            0
        }
        WM_RBUTTONUP => {
            if let Some(main) = with_app(|app| app.main) {
                menu(main);
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe extern "system" fn popup_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            if with_app(|app| app.blit_popup(hwnd)).is_none() {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            }
            0
        }
        WM_ERASEBKGND => 1,
        // Clicking anywhere else, or Escape, dismisses the panel. Both are ignored while
        // the app state is borrowed, which is when we are the ones closing it.
        WM_ACTIVATE if wparam & 0xFFFF == WA_INACTIVE as usize => {
            with_app(|app| app.close_popup());
            0
        }
        WM_KEYDOWN if wparam == VK_ESCAPE as usize => {
            with_app(|app| app.close_popup());
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Seconds east of UTC for the local time zone, daylight saving included.
unsafe fn utc_offset_secs() -> i64 {
    let mut zone: TIME_ZONE_INFORMATION = std::mem::zeroed();
    let bias = match GetTimeZoneInformation(&mut zone) {
        1 => zone.Bias + zone.StandardBias,
        2 => zone.Bias + zone.DaylightBias,
        _ => zone.Bias,
    };
    -(bias as i64) * 60
}

/// Shows the context menu at the cursor and applies the choice. The menu runs its own
/// message loop, so the app state must not be borrowed while it is open.
unsafe fn menu(owner: HWND) {
    let Some((provider, show_remaining, notify, can_refresh)) = with_app(|app| {
        (app.settings.provider, app.settings.show_remaining, app.settings.notify, app.can_refresh())
    }) else {
        return;
    };
    let checked = |on: bool| if on { MF_CHECKED } else { MF_UNCHECKED };
    let menu = CreatePopupMenu();
    let item = |id: u32, label: &str| {
        AppendMenuW(menu, MF_STRING, id as usize, wide(label).as_ptr());
    };
    item(CMD_CLAUDE, "Claude");
    item(CMD_CODEX, "Codex");
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    item(CMD_USED, "显示已用");
    item(CMD_REMAINING, "显示剩余");
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    // Greyed out right after a refresh and while the service has told us to wait.
    let refresh = if can_refresh { MF_ENABLED } else { MF_GRAYED };
    AppendMenuW(menu, MF_STRING | refresh, CMD_REFRESH as usize, wide("立即刷新").as_ptr());
    AppendMenuW(menu, MF_STRING | checked(notify), CMD_NOTIFY as usize, wide("低额度提醒").as_ptr());
    AppendMenuW(menu, MF_STRING | checked(autostart::is_enabled()), CMD_AUTOSTART as usize, wide("开机自启").as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    item(CMD_EXIT, "退出");
    let current = if provider == Provider::Claude { CMD_CLAUDE } else { CMD_CODEX };
    CheckMenuRadioItem(menu, CMD_CLAUDE, CMD_CODEX, current, MF_BYCOMMAND);
    let mode = if show_remaining { CMD_REMAINING } else { CMD_USED };
    CheckMenuRadioItem(menu, CMD_USED, CMD_REMAINING, mode, MF_BYCOMMAND);

    let mut cursor = POINT { x: 0, y: 0 };
    GetCursorPos(&mut cursor);
    // Without a foreground owner the menu would not close when clicking elsewhere.
    SetForegroundWindow(owner);
    let flags = TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN;
    let command = TrackPopupMenu(menu, flags, cursor.x, cursor.y, 0, owner, null()) as u32;
    PostMessageW(owner, WM_NULL, 0, 0);
    DestroyMenu(menu);

    if command == CMD_EXIT {
        DestroyWindow(owner);
    } else if command != 0 {
        with_app(|app| app.on_command(command));
    }
}

unsafe fn system_uses_light_theme() -> bool {
    let key = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    let name = wide("SystemUsesLightTheme");
    let mut value: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = RegGetValueW(
        HKEY_CURRENT_USER,
        key.as_ptr(),
        name.as_ptr(),
        RRF_RT_REG_DWORD,
        null_mut(),
        &mut value as *mut u32 as *mut c_void,
        &mut size,
    );
    status == ERROR_SUCCESS && value != 0
}

fn fill_color(severity: Severity) -> Color {
    match severity {
        Severity::Ok => FILL_OK,
        Severity::Warn => FILL_WARN,
        Severity::Crit => FILL_CRIT,
    }
}

fn badge(provider: Provider) -> (&'static str, Color) {
    match provider {
        Provider::Claude => ("C", Color::rgb(217, 119, 87)),
        Provider::Codex => ("X", Color::rgb(122, 162, 247)),
    }
}

/// Text and track colours for the taskbar's theme.
fn theme_colors(light: bool) -> (Color, Color) {
    if light {
        (Color::rgba(0, 0, 0, 190), Color::rgba(0, 0, 0, 50))
    } else {
        (Color::rgba(255, 255, 255, 200), Color::rgba(255, 255, 255, 56))
    }
}

/// Two bars filling a small square, for the tray.
unsafe fn make_icon(rows: &[Row; 2], light: bool, stale: bool) -> HICON {
    let n = GetSystemMetrics(SM_CXSMICON).max(16);
    let size = n as f32;
    let mut px = vec![0u32; (n * n) as usize];
    {
        let mut canvas = Canvas { px: &mut px, w: n, h: n };
        let bar_h = (size * 0.30).round();
        let gap = (size * 0.16).round();
        let top = ((size - bar_h * 2.0 - gap) / 2.0).floor();
        let track = if light { Color::rgba(0, 0, 0, 80) } else { Color::rgba(255, 255, 255, 90) };
        for (i, row) in rows.iter().enumerate() {
            let y = top + i as f32 * (bar_h + gap);
            canvas.round_rect(0.0, y, size, bar_h, size * 0.08, f32::MAX, track);
            if let Some(fill) = row.fill.filter(|f| *f > 0.0) {
                let color = fill_color(row.severity).faded(if stale { STALE_FADE } else { 1.0 });
                canvas.round_rect(0.0, y, size, bar_h, size * 0.08, size * fill, color);
            }
        }
    }
    // Icon bitmaps carry straight alpha, the canvas is premultiplied.
    for p in px.iter_mut() {
        let a = *p >> 24;
        if a != 0 && a != 255 {
            let straight = |shift: u32| (((*p >> shift) & 0xFF) * 255 / a).min(255) << shift;
            *p = (a << 24) | straight(16) | straight(8) | straight(0);
        }
    }
    let mask = vec![0u8; (((n + 15) / 16) * 2 * n) as usize];
    let color = CreateBitmap(n, n, 1, 32, px.as_ptr() as *const c_void);
    let mask = CreateBitmap(n, n, 1, 1, mask.as_ptr() as *const c_void);
    let info = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
    let icon = CreateIconIndirect(&info);
    DeleteObject(color);
    DeleteObject(mask);
    icon
}

impl App {
    fn save_settings(&self) {
        // Losing a preference is not worth interrupting the user for.
        let _ = self.settings.save(&self.settings_path);
    }

    unsafe fn tick(&mut self) {
        self.ensure_widget();
        self.fetch_if_due();
        self.refresh();
    }

    /// Asks for the displayed provider once its reading has aged out, and for the other
    /// one as well while the detail panel is open. One request runs at a time; the next
    /// is picked up when the first one returns.
    fn fetch_if_due(&mut self) {
        let now = unix_now();
        // A request that has not come back long after every timeout has passed is
        // written off, so one stuck worker cannot stop all later updates.
        if self.fetching && now - self.fetch_started > FETCH_STUCK_SECS {
            self.fetching = false;
        }
        let current = self.settings.provider;
        let wanted = [Some(current), self.popup.is_some().then_some(current.other())];
        // An open panel means someone is looking, whatever the input timer says.
        let away = self.popup.is_none() && idle_secs() >= IDLE_AFTER_SECS;
        for provider in wanted.into_iter().flatten() {
            let i = provider.index();
            let last_success = self.cache[i].map(|entry| entry.fetched_at);
            if !schedule::told_to_wait(self.blocked_until[i], now)
                && schedule::poll_due(self.next_poll[i], self.last_response[i], last_success, away, now)
            {
                self.start_fetch(provider);
            }
        }
    }

    /// Whether "refresh now" may send a request for the displayed provider.
    fn can_refresh(&self) -> bool {
        let i = self.settings.provider.index();
        let now = unix_now();
        !self.fetching
            && !schedule::told_to_wait(self.blocked_until[i], now)
            && schedule::gap_elapsed(self.last_response[i], now)
    }

    fn save_cache(&self) {
        // Best effort: without the file a restart just fetches again.
        let _ = cache::save(&self.cache_path, &Stored { cache: self.cache, blocked_until: self.blocked_until });
    }

    unsafe fn toggle_popup(&mut self) {
        if self.popup.is_some() {
            self.close_popup();
            return;
        }
        // A click on the widget or the tray icon first takes focus from an open panel,
        // which closes it; that same click must not open it again.
        if self.popup_closed_at.is_some_and(|at| at.elapsed() < Duration::from_millis(300)) {
            return;
        }
        self.open_popup();
    }

    unsafe fn open_popup(&mut self) {
        let scale = if self.geom.scale > 0.0 {
            self.geom.scale
        } else {
            GetDpiForWindow(self.main).max(96) as f32 / 96.0
        };
        let (w, h) = popup::size(scale);
        let (Some(surface), Some(scratch)) = (Surface::new(w, h), Surface::new(w, h)) else {
            return;
        };

        // Centred on the click, just above the taskbar, kept inside the monitor's work area.
        let mut cursor = POINT { x: 0, y: 0 };
        GetCursorPos(&mut cursor);
        let mut monitor: MONITORINFO = std::mem::zeroed();
        monitor.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST), &mut monitor);
        let work = monitor.rcWork;
        let margin = (8.0 * scale).round() as i32;
        let x = (cursor.x - w / 2).min(work.right - w - margin).max(work.left + margin);
        let y = work.bottom - h - margin;

        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            wide("IUsagePopup").as_ptr(),
            wide("IUsage").as_ptr(),
            WS_POPUP,
            x,
            y,
            w,
            h,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        );
        if hwnd.is_null() {
            return;
        }
        // Windows 11 rounded corners and shadow; a dark frame in dark mode.
        let round: u32 = 2; // DWMWCP_ROUND
        DwmSetWindowAttribute(hwnd, 33, &round as *const u32 as *const c_void, 4); // DWMWA_WINDOW_CORNER_PREFERENCE
        let dark: i32 = (!self.light).into();
        DwmSetWindowAttribute(hwnd, 20, &dark as *const i32 as *const c_void, 4); // DWMWA_USE_IMMERSIVE_DARK_MODE

        self.popup = Some(Popup { hwnd, surface, scratch, scale, shown: None });
        self.refresh_popup();
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        self.fetch_if_due();
    }

    unsafe fn close_popup(&mut self) {
        if let Some(popup) = self.popup.take() {
            DestroyWindow(popup.hwnd);
            self.popup_closed_at = Some(Instant::now());
        }
    }

    /// Redraws the panel's surface if its content has changed.
    unsafe fn refresh_popup(&mut self) {
        let Some(popup) = self.popup.as_mut() else {
            return;
        };
        let context = panel::Context {
            current: self.settings.provider,
            show_remaining: self.settings.show_remaining,
            thresholds: self.settings.thresholds,
            now: unix_now(),
            utc_offset: utc_offset_secs(),
        };
        let content = (panel::build(&self.cache, &self.error, &context), self.light);
        if popup.shown.as_ref() == Some(&content) {
            return;
        }
        popup::paint(&mut popup.surface, &mut popup.scratch, &content.0, content.1, popup.scale);
        popup.shown = Some(content);
        InvalidateRect(popup.hwnd, null(), 0);
    }

    unsafe fn blit_popup(&mut self, hwnd: HWND) {
        let mut paint: PAINTSTRUCT = std::mem::zeroed();
        let dc = BeginPaint(hwnd, &mut paint);
        if let Some(popup) = self.popup.as_ref().filter(|popup| popup.hwnd == hwnd) {
            BitBlt(dc, 0, 0, popup.surface.w, popup.surface.h, popup.surface.dc, 0, 0, SRCCOPY);
        }
        EndPaint(hwnd, &paint);
    }

    /// Creates the widget if needed and keeps it sized and placed against the tray.
    /// Leaves `widget` null when the taskbar cannot host it; the tray icon still works.
    unsafe fn ensure_widget(&mut self) {
        if self.drag.is_some() {
            return;
        }
        let taskbar = FindWindowW(wide("Shell_TrayWnd").as_ptr(), null());
        let tray = if taskbar.is_null() {
            null_mut()
        } else {
            FindWindowExW(taskbar, null_mut(), wide("TrayNotifyWnd").as_ptr(), null())
        };
        let mut bar = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        let mut notify = bar;
        if tray.is_null() || GetWindowRect(taskbar, &mut bar) == 0 || GetWindowRect(tray, &mut notify) == 0 {
            self.drop_widget();
            return;
        }

        let dpi = GetDpiForWindow(taskbar);
        let scale = if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 };
        let w = (WIDTH * scale).round() as i32;
        let h = bar.bottom - bar.top;
        let anchor = (notify.left - bar.left - w).max(0);
        let x = (anchor - (self.settings.offset_x as f32 * scale).round() as i32).clamp(0, anchor);
        let geom = Geom { x, w, h, anchor, scale };

        if self.widget.is_null() || IsWindow(self.widget) == 0 || GetParent(self.widget) != taskbar {
            self.drop_widget();
            self.widget = CreateWindowExW(
                WS_EX_LAYERED,
                wide("IUsageWidget").as_ptr(),
                null(),
                WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
                x,
                0,
                w,
                h,
                taskbar,
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            );
            if self.widget.is_null() {
                return;
            }
            self.geom = Geom::default();
        }
        if geom != self.geom {
            if (geom.w, geom.h) != (self.geom.w, self.geom.h) {
                self.surface = Surface::new(w, h);
                self.scratch = Surface::new(w, h);
            }
            SetWindowPos(self.widget, null_mut(), x, 0, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
            self.geom = geom;
            self.shown = None;
        }
    }

    unsafe fn drop_widget(&mut self) {
        if !self.widget.is_null() && IsWindow(self.widget) != 0 {
            DestroyWindow(self.widget);
        }
        self.widget = null_mut();
        self.geom = Geom::default();
    }

    /// Repaints the widget, the tray icon and the detail panel if what they show has changed.
    unsafe fn refresh(&mut self) {
        self.refresh_popup();
        let provider = self.settings.provider;
        let i = provider.index();
        let view = view::build(
            provider,
            self.cache[i].as_ref().map(|entry| &entry.usage),
            self.error[i].as_ref(),
            self.settings.show_remaining,
            self.settings.thresholds,
            unix_now(),
        );
        let shown = Shown { view, light: self.light };
        if self.shown.as_ref() == Some(&shown) {
            return;
        }
        self.paint(&shown);
        let old_icon = self.icon;
        self.icon = make_icon(&shown.view.rows, shown.light, shown.view.stale);
        self.shown = Some(shown);
        self.set_tray(NIM_MODIFY);
        if !old_icon.is_null() {
            DestroyIcon(old_icon);
        }
    }

    unsafe fn paint(&mut self, shown: &Shown) {
        if self.widget.is_null() {
            return;
        }
        let (Some(surface), Some(scratch)) = (self.surface.as_mut(), self.scratch.as_mut()) else {
            return;
        };
        let s = self.geom.scale;
        let (w, h) = (surface.w, surface.h);
        let fade = if shown.view.stale { STALE_FADE } else { 1.0 };
        let (text, track) = theme_colors(shown.light);
        let text = text.faded(fade);

        let bar_h = (BAR_H * s).round();
        let gap = (ROW_GAP * s).round();
        let top = ((h as f32 - bar_h * 2.0 - gap) / 2.0).floor();
        let row_y = [top, top + bar_h + gap];

        // Alpha 1 everywhere: invisible, but the whole rectangle receives mouse input.
        surface.pixels().fill(0x0100_0000);
        {
            let mut canvas = surface.canvas();
            let middle = h as f32 / 2.0;
            for column in [2.0, 6.0] {
                for dy in [-7.5, -2.5, 2.5, 7.5] {
                    canvas.circle((PAD_L + column) * s, middle + dy * s, 0.85 * s, text.faded(0.45));
                }
            }
            for (row, y) in shown.view.rows.iter().zip(row_y) {
                let track = if row.fill.is_some() { track } else { track.faded(0.5) };
                canvas.round_rect(BAR_X * s, y, BAR_W * s, bar_h, BAR_RADIUS * s, f32::MAX, track);
                if let Some(fill) = row.fill.filter(|f| *f > 0.0) {
                    let right = (BAR_X + BAR_W * fill) * s;
                    let color = fill_color(row.severity).faded(fade);
                    canvas.round_rect(BAR_X * s, y, BAR_W * s, bar_h, BAR_RADIUS * s, right, color);
                }
            }
        }

        let regular = Font::new((FONT_PX * s).round() as i32, false);
        let bold = Font::new((FONT_PX * s).round() as i32, true);
        let badge_font = Font::new((BADGE_FONT_PX * s).round() as i32, true);
        let rect = |x: f32, width: f32, top: f32, bottom: f32| RECT {
            left: (x * s).round() as i32,
            top: top as i32,
            right: ((x + width) * s).round() as i32,
            bottom: bottom as i32,
        };
        let (letter, badge_color) = badge(shown.view.provider);
        draw_text(surface, scratch, &badge_font, letter, rect(BADGE_X, BADGE_W, 0.0, h as f32), Align::Center, badge_color);
        for (row, y) in shown.view.rows.iter().zip(row_y) {
            // A band taller than the bar, sharing its centre, so glyphs are not clipped.
            let (band_top, band_bottom) = (y - bar_h, y + bar_h * 2.0);
            let label = if row.fill.is_some() { text } else { text.faded(0.5) };
            draw_text(surface, scratch, &bold, row.label, rect(LABEL_X, LABEL_W, band_top, band_bottom), Align::Left, label);
            draw_text(surface, scratch, &regular, &row.text, rect(TEXT_X, TEXT_W, band_top, band_bottom), Align::Left, text);
        }

        let size = SIZE { cx: w, cy: h };
        let origin = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION { BlendOp: 0, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: 1 };
        // No destination point: the position stays whatever SetWindowPos made it.
        UpdateLayeredWindow(self.widget, null_mut(), null(), &size, surface.dc, &origin, 0, &blend, ULW_ALPHA);
    }

    /// A notify-icon record that identifies our tray icon and nothing else.
    fn tray_data(&self) -> NOTIFYICONDATAW {
        let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = self.main;
        data.uID = TRAY_ID;
        data
    }

    unsafe fn set_tray(&mut self, message: NOTIFY_ICON_MESSAGE) {
        if self.icon.is_null() {
            let blank = view::build(self.settings.provider, None, None, false, self.settings.thresholds, 0);
            self.icon = make_icon(&blank.rows, self.light, false);
        }
        let mut data = self.tray_data();
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.uCallbackMessage = WM_APP_TRAY;
        data.hIcon = self.icon;
        let tip = self.shown.as_ref().map_or("IUsage", |shown| shown.view.tip.as_str());
        copy_wide(&mut data.szTip, tip);
        Shell_NotifyIconW(message, &data);
    }

    /// Raises a system notification through the tray icon.
    unsafe fn notify(&self, provider: Provider, alert: &Alert) {
        let mut data = self.tray_data();
        data.uFlags = NIF_INFO;
        copy_wide(&mut data.szInfoTitle, &format!("{} 用量提醒", provider.name()));
        copy_wide(&mut data.szInfo, &alert.lines.join("\n"));
        data.dwInfoFlags = if alert.critical { NIIF_WARNING } else { NIIF_INFO };
        Shell_NotifyIconW(NIM_MODIFY, &data);
    }

    unsafe fn shutdown(&mut self) {
        let data = self.tray_data();
        Shell_NotifyIconW(NIM_DELETE, &data);
        self.close_popup();
        self.drop_widget();
        if !self.icon.is_null() {
            DestroyIcon(self.icon);
            self.icon = null_mut();
        }
    }

    fn start_fetch(&mut self, provider: Provider) {
        if self.fetching {
            return;
        }
        self.fetching = true;
        self.fetch_started = unix_now();
        let env = Arc::clone(&self.env);
        let main = self.main as usize;
        let spawned = std::thread::Builder::new().stack_size(256 * 1024).spawn(move || {
            let result = providers::fetch(provider, &env, unix_now(), http::get);
            if let Ok(mut slot) = FETCHED.lock() {
                *slot = Some((provider, result));
            }
            unsafe { PostMessageW(main as HWND, WM_APP_FETCHED, 0, 0) };
        });
        if spawned.is_err() {
            self.fetching = false;
            self.next_poll[provider.index()] = unix_now() + CREDENTIAL_RETRY_SECS;
        }
    }

    unsafe fn on_fetched(&mut self) {
        let Some((provider, result)) = FETCHED.lock().ok().and_then(|mut slot| slot.take()) else {
            return;
        };
        self.fetching = false;
        let now = unix_now();
        let i = provider.index();
        match result {
            Ok(usage) => {
                // While notifications are off nothing counts as announced, so turning
                // them on reports the current state at the next reading.
                let mut announced = [0, 0];
                let mut alert = None;
                if self.settings.notify {
                    announced = self.cache[i].map_or(announced, |entry| entry.announced);
                    alert = alert::check(&mut announced, &usage, self.settings.thresholds, now);
                }
                self.cache[i] = Some(Entry { usage, fetched_at: now, announced });
                self.error[i] = None;
                self.backoff[i] = 0;
                self.next_poll[i] = now + self.settings.poll_secs as i64;
                self.last_response[i] = now;
                self.blocked_until[i] = 0;
                self.save_cache();
                if let Some(alert) = alert {
                    self.notify(provider, &alert);
                }
            }
            Err(error) => {
                // Problems on this machine never reached the service, so they do not
                // count towards the gap between requests and can be re-checked quickly.
                let local = matches!(error, FetchError::NoCredentials | FetchError::Expired | FetchError::Network(_));
                let wait = match error {
                    FetchError::NoCredentials | FetchError::Expired => CREDENTIAL_RETRY_SECS,
                    _ => {
                        self.backoff[i] = if self.backoff[i] == 0 { 30 } else { (self.backoff[i] * 2).min(MAX_BACKOFF_SECS) };
                        match error {
                            FetchError::RateLimited(retry_after) => self.backoff[i]
                                .max(RATE_LIMIT_MIN_SECS)
                                .max((retry_after.unwrap_or(0) as i64).min(RATE_LIMIT_MAX_SECS)),
                            // A token the service rejects stays rejected until the
                            // provider's own app replaces it; hammering does not help.
                            FetchError::Unauthorized => self.backoff[i].max(RATE_LIMIT_MIN_SECS),
                            _ => self.backoff[i],
                        }
                    }
                };
                // One line for troubleshooting; the file's own timestamp says when.
                let _ = std::fs::write(&self.error_path, format!("{}: {error:?}\n", provider.name()));
                self.error[i] = Some(error);
                self.next_poll[i] = now + wait;
                if !local {
                    self.last_response[i] = now;
                }
                if matches!(self.error[i], Some(FetchError::RateLimited(_) | FetchError::Unauthorized)) {
                    self.blocked_until[i] = now + wait;
                    self.save_cache();
                }
            }
        }
        // The user may have switched while this request was in flight.
        self.fetch_if_due();
        self.refresh();
    }

    unsafe fn on_command(&mut self, command: u32) {
        match command {
            CMD_CLAUDE => self.set_provider(Provider::Claude),
            CMD_CODEX => self.set_provider(Provider::Codex),
            CMD_USED | CMD_REMAINING => {
                self.settings.show_remaining = command == CMD_REMAINING;
                self.save_settings();
                self.refresh();
            }
            CMD_REFRESH => {
                // The menu greys this out when it is not allowed; checked again here
                // because things may have changed while the menu was open.
                if self.can_refresh() {
                    let provider = self.settings.provider;
                    self.backoff[provider.index()] = 0;
                    self.start_fetch(provider);
                }
            }
            CMD_NOTIFY => {
                self.settings.notify = !self.settings.notify;
                self.save_settings();
            }
            CMD_AUTOSTART => {
                // The registry is the only state: the menu reads it back next time it opens.
                autostart::set_enabled(!autostart::is_enabled());
            }
            _ => {}
        }
    }

    unsafe fn set_provider(&mut self, provider: Provider) {
        if provider == self.settings.provider {
            return;
        }
        self.settings.provider = provider;
        self.save_settings();
        // Switching shows the last reading for that provider and only asks the
        // service if that reading has aged out.
        self.fetch_if_due();
        self.refresh();
    }

    unsafe fn on_down(&mut self, client_x: i32) {
        let mut cursor = POINT { x: 0, y: 0 };
        GetCursorPos(&mut cursor);
        SetCapture(self.widget);
        self.drag = Some(Drag {
            grip: (client_x as f32) < (PAD_L + GRIP_W + 2.0) * self.geom.scale,
            moved: false,
            start_cursor: cursor.x,
            start_x: self.geom.x,
        });
    }

    unsafe fn on_move(&mut self) {
        let Some(drag) = self.drag.as_mut().filter(|drag| drag.grip) else {
            return;
        };
        let mut cursor = POINT { x: 0, y: 0 };
        GetCursorPos(&mut cursor);
        let dx = cursor.x - drag.start_cursor;
        if dx.abs() > 3 {
            drag.moved = true;
        }
        if !drag.moved {
            return;
        }
        let x = (drag.start_x + dx).clamp(0, self.geom.anchor);
        if x != self.geom.x {
            self.geom.x = x;
            SetWindowPos(self.widget, null_mut(), x, 0, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }

    unsafe fn on_up(&mut self, client_x: i32) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        ReleaseCapture();
        if drag.grip {
            if drag.moved {
                self.commit_offset();
            }
            return;
        }
        // A click on the badge switches between the two assistants; anywhere else
        // opens or closes the detail panel.
        let x = client_x as f32 / self.geom.scale;
        if (BADGE_X - 2.0..BADGE_X + BADGE_W + 2.0).contains(&x) {
            self.set_provider(self.settings.provider.other());
        } else {
            self.toggle_popup();
        }
    }

    /// Capture was taken away mid-drag (Alt-Tab, UAC prompt): keep the position, never click.
    fn on_capture_lost(&mut self) {
        if let Some(drag) = self.drag.take() {
            if drag.grip && drag.moved {
                self.commit_offset();
            }
        }
    }

    fn commit_offset(&mut self) {
        self.settings.offset_x = ((self.geom.anchor - self.geom.x) as f32 / self.geom.scale).round() as i32;
        self.save_settings();
    }
}
