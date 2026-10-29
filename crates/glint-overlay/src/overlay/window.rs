//! Win32 side of the overlay: one borderless topmost popup per monitor, all on the
//! calling thread, sharing one [`Interaction`] through a thread-local, plus a private
//! message loop that ends when the interaction finishes.
//!
//! Re-entrancy: Win32 sends some messages synchronously from inside API calls. The
//! shared state is a `RefCell`; a message that arrives while a handler holds it goes to
//! `DefWindowProcW`. Handlers therefore never call APIs that must reach our own code
//! synchronously; capture changes and cursor moves are applied after the borrow ends.

use std::cell::RefCell;
use std::mem::size_of;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, ERROR_CLASS_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_CLOAK, DWMWA_COLOR_NONE, DWMWA_TRANSITIONS_FORCEDISABLED,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWINDOWATTRIBUTE,
};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, InvalidateRect, UpdateWindow, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_CONTROL};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos,
    GetForegroundWindow, GetMessageW, GetWindowRect, GetWindowThreadProcessId, IsHungAppWindow, IsWindow, LoadCursorW,
    PostMessageW,
    PostQuitMessage, RegisterClassExW, SetCursor, SetCursorPos, SetForegroundWindow, SetWindowPos, TranslateMessage,
    UnregisterClassW, HCURSOR, HTCLIENT, HWND_TOPMOST, IDC_CROSS, MSG, SC_KEYMENU, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    WM_CAPTURECHANGED, WM_CLOSE, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_NULL, WM_PAINT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR, WM_SYSCHAR,
    WM_SYSCOMMAND, WM_SYSKEYDOWN, WNDCLASSEXW, WNDCLASS_STYLES, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use super::input::{self, Capture, Finish, Interaction, Response, View};
use super::render::{Drawn, Gfx, Renderer};
use super::{build_selection, snap, validate_frames};
use crate::geom::{Point, Rect};
use crate::win::{last_error, ThreadDpiScope};
use crate::{MonitorFrame, Outcome, OverlayError, OverlayOptions, Result};

const CLASS_NAME: PCWSTR = w!("GlintSelectionOverlay");
/// Consecutive `D2DERR_RECREATE_TARGET`s tolerated before giving up.
const MAX_DEVICE_LOSSES: u32 = 4;

/// Only one overlay at a time: the window class and thread-local state are global.
static ACTIVE: AtomicBool = AtomicBool::new(false);

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

pub fn select_region(frames: Vec<MonitorFrame>, opts: &OverlayOptions) -> Result<Outcome> {
    validate_frames(&frames)?;
    let _busy = Busy::acquire()?;
    let _dpi = ThreadDpiScope::per_monitor_v2();

    let monitors: Vec<Rect> = frames.iter().map(|f| Rect::from_phys(f.rect)).collect();
    // Taken before our windows exist, so they can never be snap targets.
    let targets = if opts.snap_windows { snap::snapshot() } else { Vec::new() };
    // SAFETY: no preconditions.
    let previous_foreground = unsafe { GetForegroundWindow() };

    let instance = module_instance()?;
    let class = WindowClass::register(instance)?;
    let gfx = Gfx::new().map_err(|e| OverlayError::Window(format!("Direct2D/DirectWrite init: {e}")))?;
    let windows = WindowSet::create(instance, &monitors)?;

    let mut surfaces = Vec::with_capacity(frames.len());
    for (&hwnd, frame) in windows.0.iter().zip(&frames) {
        let renderer = Renderer::new(&gfx, hwnd, frame.scale, opts.accent)
            .map_err(|e| OverlayError::Window(format!("text format: {e}")))?;
        let origin = Point::new(frame.rect.x, frame.rect.y);
        surfaces.push(Surface { hwnd, origin, renderer, requested: View::default() });
    }

    let mut interaction = Interaction::new(monitors.clone(), targets, opts.default_action, opts.show_magnifier);
    if let Some(p) = cursor_pos() {
        // Seed the hover state so the very first frame already shows snap + magnifier.
        interaction.pointer_moved(p);
    }
    let focus = interaction.cursor().and_then(|c| interaction.monitor_at(c)).unwrap_or(0);
    // SAFETY: loads a predefined system cursor (shared; never destroyed).
    let cross = unsafe { LoadCursorW(None, IDC_CROSS) }.ok();

    // From here on the App owns the renderers; `slot` drops before `windows`, so render
    // targets are always released before their windows are destroyed.
    let slot = AppSlot::install(App {
        surfaces,
        gfx,
        frames,
        interaction,
        cross,
        finish: None,
        error: None,
        device_losses: 0,
    });

    windows.show_and_paint(&monitors);
    if !app_done() {
        windows.uncloak();
        if let Some(&hwnd) = windows.0.get(focus) {
            force_foreground(hwnd);
        }
        if let Some(c) = cross {
            // SAFETY: shared system cursor; we are now the foreground thread.
            unsafe { SetCursor(Some(c)) };
        }
    }

    let exit = run_message_loop();

    let app = slot.take();
    // Release render targets, then destroy windows, then the class.
    let App { surfaces, frames, finish, error, .. } = app.ok_or_else(|| OverlayError::Window("overlay state lost".into()))?;
    drop(surfaces);
    drop(windows);
    drop(class);
    restore_foreground(previous_foreground);

    let error = match exit {
        LoopExit::Done => error,
        LoopExit::Failed(e) => Some(error.unwrap_or(e)),
        LoopExit::Quit(code) => {
            // WM_QUIT belongs to the caller's loop: hand it back now that we are clean.
            // SAFETY: no preconditions.
            unsafe { PostQuitMessage(code) };
            error
        }
    };
    if let Some(e) = error {
        return Err(e);
    }
    match finish {
        Some(Finish::Select { rect, monitor, action }) => {
            build_selection(&frames, rect, monitor, action).map(Outcome::Selected)
        }
        Some(Finish::Cancel) | None => Ok(Outcome::Cancelled),
    }
}

// ---------------------------------------------------------------------------------
// Shared state

struct Surface {
    hwnd: HWND,
    /// Monitor top-left in virtual-desktop px; client (0, 0) of `hwnd`.
    origin: Point,
    renderer: Renderer,
    /// Last view this window was invalidated for.
    requested: View,
}

struct App {
    // Field order matters for drop order: surfaces (render targets) go first.
    surfaces: Vec<Surface>,
    gfx: Gfx,
    frames: Vec<MonitorFrame>,
    interaction: Interaction,
    cross: Option<HCURSOR>,
    finish: Option<Finish>,
    error: Option<OverlayError>,
    device_losses: u32,
}

/// Side effects to run after the state borrow is released, because they make Win32
/// send messages back to us synchronously.
#[derive(Default)]
struct Deferred {
    capture: Option<Capture>,
    cursor_to: Option<Point>,
    wake: bool,
}

impl Deferred {
    fn apply(self, hwnd: HWND) {
        // SAFETY (whole block): `hwnd` is the live window whose message we are handling;
        // all calls run on its thread.
        unsafe {
            match self.capture {
                Some(Capture::Set) => {
                    SetCapture(hwnd);
                }
                Some(Capture::Release) => {
                    let _ = ReleaseCapture();
                }
                None => {}
            }
            if let Some(p) = self.cursor_to {
                // Generates a WM_MOUSEMOVE, which updates the state like a real move.
                if let Err(e) = SetCursorPos(p.x, p.y) {
                    log::debug!("SetCursorPos failed: {e}");
                }
            }
            if self.wake {
                // GetMessageW may be waiting inside a sent message; make sure the loop
                // wakes up and sees that we are done.
                let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            }
        }
    }
}

impl App {
    fn done(&self) -> bool {
        self.finish.is_some() || self.error.is_some()
    }

    fn fail(&mut self, e: OverlayError) {
        log::error!("overlay: {e}");
        if self.error.is_none() {
            self.error = Some(e);
        }
    }

    fn finish_with(&mut self, f: Finish) -> Deferred {
        if self.finish.is_none() {
            self.finish = Some(f);
        }
        Deferred { wake: true, ..Deferred::default() }
    }

    /// Invalidates every window whose view changed, then turns the response into
    /// deferred side effects.
    fn respond(&mut self, r: Response) -> Deferred {
        for (i, s) in self.surfaces.iter_mut().enumerate() {
            let v = self.interaction.view(i);
            if v != s.requested {
                s.requested = v;
                // SAFETY: our live window; InvalidateRect only queues a WM_PAINT.
                unsafe {
                    let _ = InvalidateRect(Some(s.hwnd), None, false);
                }
            }
        }
        let mut d = Deferred { capture: r.capture, cursor_to: r.move_cursor, wake: false };
        if let Some(f) = r.finish {
            d.wake = self.finish_with(f).wake;
        }
        d
    }

    fn paint(&mut self, idx: usize) {
        let view = self.interaction.view(idx);
        let (Some(s), Some(frame)) = (self.surfaces.get_mut(idx), self.frames.get(idx)) else {
            return;
        };
        s.requested = view;
        let hwnd = s.hwnd;
        let result = s.renderer.draw(&self.gfx, frame, &view.to_local(s.origin));
        match result {
            Ok(Drawn::Presented) => self.device_losses = 0,
            Ok(Drawn::DeviceLost) => {
                self.device_losses += 1;
                if self.device_losses > MAX_DEVICE_LOSSES {
                    self.fail(OverlayError::Window("graphics device keeps getting lost".into()));
                } else {
                    // SAFETY: our live window; queues a repaint with a fresh device.
                    unsafe {
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                }
            }
            Err(e) => self.fail(OverlayError::Window(format!("render: {e}"))),
        }
    }

    fn handle(&mut self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> (Option<LRESULT>, Deferred) {
        let Some(idx) = self.surfaces.iter().position(|s| s.hwnd == hwnd) else {
            return (None, Deferred::default());
        };
        let point = || {
            let (x, y) = input::split_lparam(lp.0);
            self.surfaces[idx].origin.offset(x, y)
        };
        let handled = Some(LRESULT(0));
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                // SAFETY: `hwnd` is our window and `ps` a valid out-pointer; EndPaint
                // below pairs the call. D2D presents on its own; this only validates.
                unsafe { BeginPaint(hwnd, &mut ps) };
                self.paint(idx);
                // SAFETY: pairs the BeginPaint above.
                unsafe {
                    let _ = EndPaint(hwnd, &ps);
                }
                (handled, Deferred::default())
            }
            WM_SETCURSOR if (lp.0 & 0xFFFF) as u32 == HTCLIENT => match self.cross {
                Some(c) => {
                    // SAFETY: shared system cursor.
                    unsafe { SetCursor(Some(c)) };
                    (Some(LRESULT(1)), Deferred::default())
                }
                None => (None, Deferred::default()),
            },
            WM_MOUSEMOVE => {
                let p = point();
                let r = self.interaction.pointer_moved(p);
                (handled, self.respond(r))
            }
            WM_LBUTTONDOWN => {
                let p = point();
                let r = self.interaction.left_down(p);
                (handled, self.respond(r))
            }
            WM_LBUTTONUP => {
                let p = point();
                let r = self.interaction.left_up(p);
                (handled, self.respond(r))
            }
            WM_RBUTTONDOWN => {
                let p = point();
                let r = self.interaction.right_down(p);
                (handled, self.respond(r))
            }
            WM_RBUTTONUP => (handled, Deferred::default()),
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                // SAFETY: reads this thread's keyboard state.
                let ctrl = unsafe { GetKeyState(i32::from(VK_CONTROL.0)) } < 0;
                let repeat = lp.0 & (1 << 30) != 0;
                match input::map_key(wp.0 as u16, ctrl, msg == WM_SYSKEYDOWN, repeat) {
                    Some(key) => {
                        let r = self.interaction.key(key);
                        (handled, self.respond(r))
                    }
                    None => (None, Deferred::default()),
                }
            }
            // No menu: swallow Alt+letter so DefWindowProc does not beep.
            WM_SYSCHAR => (handled, Deferred::default()),
            WM_CAPTURECHANGED => {
                let gaining = HWND(lp.0 as *mut _);
                if self.surfaces.iter().any(|s| s.hwnd == gaining) {
                    (handled, Deferred::default())
                } else {
                    let r = self.interaction.capture_lost();
                    (handled, self.respond(r))
                }
            }
            // Alt+F4 / SC_CLOSE, and monitor layout changes (the frames are stale).
            WM_CLOSE | WM_DISPLAYCHANGE => (handled, self.finish_with(Finish::Cancel)),
            // Each window sits exactly on its monitor; never resize to a suggested rect.
            WM_DPICHANGED => (handled, Deferred::default()),
            _ => (None, Deferred::default()),
        }
    }
}

/// Runs `f` on the installed App, unless there is none or it is already borrowed.
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.try_with(|slot| slot.try_borrow_mut().ok().and_then(|mut guard| guard.as_mut().map(f)))
        .ok()
        .flatten()
}

fn app_done() -> bool {
    with_app(|app| app.done()).unwrap_or(true)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_ERASEBKGND {
        // Direct2D paints every pixel; never let GDI erase (that is the white flash).
        return LRESULT(1);
    }
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        with_app(|app| app.handle(hwnd, msg, wp, lp)).unwrap_or((None, Deferred::default()))
    }));
    let (result, deferred) = outcome.unwrap_or_else(|_| {
        with_app(|app| app.fail(OverlayError::Window("panic in overlay window procedure".into())));
        (None, Deferred { wake: true, ..Deferred::default() })
    });
    deferred.apply(hwnd);
    match result {
        Some(r) => r,
        // Never let DefWindowProc destroy a window (WM_CLOSE) or enter the system-menu
        // loop (Alt / F10 -> SC_KEYMENU), which would swallow the next Esc.
        None if msg == WM_CLOSE => LRESULT(0),
        None if msg == WM_SYSCOMMAND && (wp.0 & 0xFFF0) as u32 == SC_KEYMENU => LRESULT(0),
        // SAFETY: forwards the unmodified message to the default procedure.
        None => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

enum LoopExit {
    Done,
    Quit(i32),
    Failed(OverlayError),
}

fn run_message_loop() -> LoopExit {
    let mut msg = MSG::default();
    loop {
        if app_done() {
            return LoopExit::Done;
        }
        // SAFETY: `msg` is a valid out-pointer; NULL hwnd = all windows of this thread.
        let r = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        match r.0 {
            0 => return LoopExit::Quit(msg.wParam.0 as i32),
            -1 => return LoopExit::Failed(OverlayError::Window(last_error("GetMessageW"))),
            _ => {
                // SAFETY: dispatching a message GetMessageW just filled in.
                unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------
// RAII guards

struct Busy;

impl Busy {
    fn acquire() -> Result<Self> {
        ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Busy)
            .map_err(|_| OverlayError::Window("an overlay is already open".into()))
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::Release);
    }
}

/// Owns the thread-local App; clears it on drop so nothing outlives the call.
struct AppSlot;

impl AppSlot {
    fn install(app: App) -> Self {
        APP.with(|slot| *slot.borrow_mut() = Some(app));
        AppSlot
    }

    fn take(self) -> Option<App> {
        APP.try_with(|slot| slot.try_borrow_mut().ok().and_then(|mut g| g.take())).ok().flatten()
    }
}

impl Drop for AppSlot {
    fn drop(&mut self) {
        let _ = APP.try_with(|slot| slot.try_borrow_mut().map(|mut g| g.take()));
    }
}

struct WindowClass {
    instance: HINSTANCE,
}

impl WindowClass {
    fn register(instance: HINSTANCE) -> Result<Self> {
        // SAFETY: loads a predefined system cursor.
        let cursor = unsafe { LoadCursorW(None, IDC_CROSS) }.unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            // No CS_HREDRAW/CS_VREDRAW (never resized) and no background brush, so GDI
            // has nothing to paint before Direct2D does.
            style: WNDCLASS_STYLES(0),
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            hCursor: cursor,
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        // SAFETY: `wc` is fully initialized and its strings are static.
        if unsafe { RegisterClassExW(&wc) } == 0 {
            // SAFETY: no preconditions.
            let err = unsafe { GetLastError() };
            // Left over from a call whose windows outlived it; same wndproc, reuse it.
            if err != ERROR_CLASS_ALREADY_EXISTS {
                return Err(OverlayError::Window(format!("RegisterClassExW: {err:?}")));
            }
            log::warn!("overlay window class already registered; reusing it");
        }
        Ok(Self { instance })
    }
}

impl Drop for WindowClass {
    fn drop(&mut self) {
        // SAFETY: all windows of the class were destroyed before this runs.
        if let Err(e) = unsafe { UnregisterClassW(CLASS_NAME, Some(self.instance)) } {
            log::warn!("UnregisterClassW: {e}");
        }
    }
}

/// The per-monitor popups, destroyed on drop.
struct WindowSet(Vec<HWND>);

impl WindowSet {
    fn create(instance: HINSTANCE, monitors: &[Rect]) -> Result<Self> {
        let mut set = WindowSet(Vec::with_capacity(monitors.len()));
        for m in monitors {
            // SAFETY: the class is registered; created hidden at the monitor's exact
            // physical rect (the thread is PMv2, so no coordinate virtualization).
            let hwnd = unsafe {
                CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                    CLASS_NAME,
                    w!("Glint"),
                    WS_POPUP,
                    m.left,
                    m.top,
                    m.width(),
                    m.height(),
                    None,
                    None,
                    Some(instance),
                    None,
                )
            }
            .map_err(|e| OverlayError::Window(format!("CreateWindowExW: {e}")))?;
            set.0.push(hwnd);
            // Instant show/hide, and square corners with no Windows 11 border, so the
            // window covers its monitor exactly. These fail harmlessly on older builds.
            dwm_set(hwnd, DWMWA_TRANSITIONS_FORCEDISABLED, &BOOL::from(true));
            dwm_set(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &DWMWCP_DONOTROUND);
            dwm_set(hwnd, DWMWA_BORDER_COLOR, &DWMWA_COLOR_NONE);
        }
        Ok(set)
    }

    /// Shows every window cloaked (DWM keeps it invisible), paints it synchronously,
    /// and fixes its rect if Windows adjusted it. Nothing is visible yet afterwards.
    fn show_and_paint(&self, monitors: &[Rect]) {
        for (&hwnd, m) in self.0.iter().zip(monitors) {
            if !dwm_set(hwnd, DWMWA_CLOAK, &BOOL::from(true)) {
                log::debug!("DWM cloak unavailable; window may flash once");
            }
            place(hwnd, *m, SWP_SHOWWINDOW | SWP_NOACTIVATE);
            let mut r = RECT::default();
            // SAFETY: our live window; `r` is a valid out-pointer.
            if unsafe { GetWindowRect(hwnd, &mut r) }.is_ok() {
                let actual = Rect::new(r.left, r.top, r.right, r.bottom);
                if actual != *m {
                    log::warn!("overlay window is {actual:?}, expected {m:?}; fixing");
                    place(hwnd, *m, SWP_NOACTIVATE);
                }
            }
        }
        for &hwnd in &self.0 {
            // SAFETY: our live window. Sends WM_PAINT synchronously; the App is
            // installed and not borrowed here, so the first frame is drawn now.
            unsafe {
                let _ = UpdateWindow(hwnd);
            }
        }
    }

    fn uncloak(&self) {
        for &hwnd in &self.0 {
            dwm_set(hwnd, DWMWA_CLOAK, &BOOL::from(false));
        }
    }
}

impl Drop for WindowSet {
    fn drop(&mut self) {
        for &hwnd in &self.0 {
            // SAFETY: destroys a window this thread created (also releases capture).
            if let Err(e) = unsafe { DestroyWindow(hwnd) } {
                log::warn!("DestroyWindow: {e}");
            }
        }
    }
}

fn place(hwnd: HWND, m: Rect, flags: windows::Win32::UI::WindowsAndMessaging::SET_WINDOW_POS_FLAGS) {
    // SAFETY: our live window; physical coordinates under PMv2.
    if let Err(e) = unsafe { SetWindowPos(hwnd, Some(HWND_TOPMOST), m.left, m.top, m.width(), m.height(), flags) } {
        log::warn!("SetWindowPos: {e}");
    }
}

/// Sets a DWM window attribute. `T` must be the attribute's documented value type
/// (BOOL, a 32-bit enum or a COLORREF for the ones used here).
fn dwm_set<T: Copy>(hwnd: HWND, attr: DWMWINDOWATTRIBUTE, value: &T) -> bool {
    // SAFETY: the pointer and size describe `value`, which outlives the call; DWM
    // copies it and rejects mismatched sizes with an error.
    unsafe { DwmSetWindowAttribute(hwnd, attr, (value as *const T).cast(), size_of::<T>() as u32) }.is_ok()
}

fn module_instance() -> Result<HINSTANCE> {
    // SAFETY: NULL = the executable this crate is linked into.
    unsafe { GetModuleHandleW(None) }
        .map(HINSTANCE::from)
        .map_err(|e| OverlayError::Window(format!("GetModuleHandleW: {e}")))
}

fn cursor_pos() -> Option<Point> {
    let mut p = POINT::default();
    // SAFETY: valid out-pointer.
    unsafe { GetCursorPos(&mut p) }.ok()?;
    Some(Point::new(p.x, p.y))
}

/// Brings `hwnd` to the foreground so keys work without a click first. Windows only
/// lets the foreground thread do that, so briefly share input state with it.
fn force_foreground(hwnd: HWND) {
    // SAFETY (whole block): thread ids come from the system; `hwnd` is our live window;
    // AttachThreadInput is undone before returning.
    unsafe {
        let me = GetCurrentThreadId();
        let fg = GetForegroundWindow();
        // Attached input makes focus changes send messages to the other thread
        // synchronously; never attach to a hung one or the overlay would hang with it.
        let fg_thread = if fg.0.is_null() || IsHungAppWindow(fg).as_bool() {
            0
        } else {
            GetWindowThreadProcessId(fg, None)
        };
        let attached = fg_thread != 0 && fg_thread != me && AttachThreadInput(me, fg_thread, true).as_bool();
        let _ = BringWindowToTop(hwnd);
        let ok = SetForegroundWindow(hwnd).as_bool();
        let _ = SetFocus(Some(hwnd));
        if attached {
            let _ = AttachThreadInput(me, fg_thread, false);
        }
        if !ok {
            log::warn!("SetForegroundWindow failed; keyboard works after the first click");
        }
    }
}

/// Gives focus back to the window that had it before the overlay (we are still the
/// foreground thread, so Windows allows it).
fn restore_foreground(previous: HWND) {
    if previous.0.is_null() {
        return;
    }
    // SAFETY: `IsWindow` guards against a stale handle.
    unsafe {
        if IsWindow(Some(previous)).as_bool() {
            let _ = SetForegroundWindow(previous);
        }
    }
}
