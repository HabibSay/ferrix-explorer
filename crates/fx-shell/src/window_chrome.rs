//! Frameless-window chrome: the Win32 half of the app's own caption bar.
//!
//! Slint opens the window with `no-frame: true`, so nothing native draws a title bar.
//! This module keeps the window usable anyway:
//!
//! * it gives the window back the styles that a frameless window loses (a sizing frame,
//!   the system menu, minimise/maximise boxes), so the edges still resize the window;
//! * it answers `WM_NCHITTEST` so the caption band drags the window, double-clicking it
//!   maximises (through the default handling of `HTCAPTION`), and the edges report the
//!   matching `HT*` resize codes;
//! * it reports the maximise state back to the app, so the button can show "restore";
//! * it sets the Windows 11 rounded-corner preference, which a frameless window does not
//!   get by itself (the attribute is ignored on Windows 10).
//!
//! The three caption buttons stay client area: Slint draws their hover state and takes
//! their clicks, so they behave like every other button in the app and are reachable
//! from the keyboard. The trade-off is documented in `docs/tranche-4-validation.md`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_STYLE, GetClientRect, GetWindowLongPtrW, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION,
    HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, IsZoomed, MINMAXINFO, NCCALCSIZE_PARAMS,
    SM_CXPADDEDBORDER, SM_CXSIZEFRAME, SM_CYSIZEFRAME, SetWindowLongPtrW, WM_GETMINMAXINFO,
    WM_NCCALCSIZE, WM_NCHITTEST, WM_SIZE, WS_CAPTION, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU,
    WS_THICKFRAME,
};

/// Identifies this module's subclass when several are installed on one window.
const SUBCLASS_ID: usize = 0x46_58_43_48; // "FXCH"

/// Size of the primary monitor's work area, in physical pixels.
///
/// Used to keep a restored window on screen: a size saved on a larger monitor would
/// otherwise open with its own caption buttons past the edges and no way to drag it back.
pub fn work_area_size(hwnd: HWND) -> Option<(u32, u32)> {
    let work_area = work_area_rect(hwnd)?;
    let width = work_area.right - work_area.left;
    let height = work_area.bottom - work_area.top;
    if width <= 0 || height <= 0 {
        None
    } else {
        Some((width as u32, height as u32))
    }
}

/// The logical layout of the drawn caption bar. Everything is in logical pixels and
/// scaled by the window's DPI when the hit-test runs.
#[derive(Clone, Copy, Debug)]
pub struct ChromeMetrics {
    /// Height of the caption band, measured from the top of the window.
    pub caption_height: f64,
    /// Width of one caption button.
    pub button_width: f64,
    /// How many caption buttons sit at the right end of the band.
    pub button_count: u32,
    /// Smallest size the window may be dragged to (`WM_GETMINMAXINFO`).
    pub min_width: f64,
    pub min_height: f64,
}

impl Default for ChromeMetrics {
    fn default() -> Self {
        Self {
            caption_height: 34.0,
            button_width: 46.0,
            button_count: 3,
            min_width: 400.0,
            min_height: 300.0,
        }
    }
}

/// Why the custom chrome could not be installed. The caller keeps the native frame.
#[derive(Debug)]
pub struct ChromeError(&'static str);

impl fmt::Display for ChromeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for ChromeError {}

struct ChromeState {
    metrics: ChromeMetrics,
    maximized: Cell<bool>,
    on_maximized: Box<dyn Fn(bool)>,
}

thread_local! {
    /// Keyed by HWND. Windows delivers a window's messages on the thread that created it,
    /// so a thread-local map is enough and keeps the callbacks off any `Send` boundary.
    static CHROME: RefCell<HashMap<isize, Rc<ChromeState>>> = RefCell::new(HashMap::new());
}

/// An installed subclass. Dropping it restores the window's previous message handling.
pub struct WindowChrome {
    hwnd: HWND,
}

impl WindowChrome {
    /// Install the custom chrome on `hwnd`. `on_maximized` runs on the UI thread whenever
    /// the window is maximised or restored, including by double-clicking the caption.
    pub fn install(
        hwnd: HWND,
        metrics: ChromeMetrics,
        on_maximized: impl Fn(bool) + 'static,
    ) -> Result<Self, ChromeError> {
        if hwnd.is_invalid() {
            return Err(ChromeError("the window handle is not valid"));
        }
        let state = Rc::new(ChromeState {
            metrics,
            maximized: Cell::new(false),
            on_maximized: Box::new(on_maximized),
        });
        CHROME.with(|map| {
            map.borrow_mut().insert(hwnd.0 as isize, Rc::clone(&state));
        });

        // SAFETY: `hwnd` is a live window on this thread, and the callback keeps the
        // state in the thread-local map for as long as the subclass is installed.
        let installed = unsafe { SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, 0) };
        if !installed.as_bool() {
            CHROME.with(|map| {
                map.borrow_mut().remove(&(hwnd.0 as isize));
            });
            return Err(ChromeError("the window refuses a message subclass"));
        }

        // A frameless window loses the styles that make its edges resize it.
        // SAFETY: reading and writing the style word of a live window on this thread.
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
            let wanted = style
                | WS_THICKFRAME.0
                | WS_CAPTION.0
                | WS_MAXIMIZEBOX.0
                | WS_MINIMIZEBOX.0
                | WS_SYSMENU.0;
            if wanted != style {
                SetWindowLongPtrW(hwnd, GWL_STYLE, wanted as isize);
            }

            // Windows 11 rounds a framed window by itself; this one has to ask. The
            // attribute does not exist on Windows 10, where the call simply fails.
            let preference = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &preference as *const _ as *const std::ffi::c_void,
                std::mem::size_of_val(&preference) as u32,
            );
        }

        let chrome = Self { hwnd };
        let maximized = chrome.is_maximized();
        CHROME.with(|map| {
            if let Some(state) = map.borrow().get(&(hwnd.0 as isize)) {
                state.maximized.set(maximized);
            }
        });
        Ok(chrome)
    }

    /// Whether the window is currently maximised.
    pub fn is_maximized(&self) -> bool {
        // SAFETY: the handle belongs to a live window on this thread.
        unsafe { IsZoomed(self.hwnd).as_bool() }
    }
}

impl Drop for WindowChrome {
    fn drop(&mut self) {
        // SAFETY: same handle and subclass id used by `install`.
        unsafe {
            let _ = RemoveWindowSubclass(self.hwnd, Some(subclass_proc), SUBCLASS_ID);
        }
        CHROME.with(|map| {
            map.borrow_mut().remove(&(self.hwnd.0 as isize));
        });
    }
}

fn state_of(hwnd: HWND) -> Option<Rc<ChromeState>> {
    CHROME.with(|map| map.borrow().get(&(hwnd.0 as isize)).cloned())
}

/// Declares the process per-monitor (v2) DPI aware.
///
/// Call it once, before any window exists. Without it Windows virtualises the process at
/// the system DPI while the window is laid out at the real one, and the two disagree about
/// how wide the window is: the UI then stops short of the window's right edge (the last
/// detail column ends in the middle of a maximised window). A process that is already
/// aware, as winit may make it, gets `FALSE` here and keeps its existing mode.
pub fn enable_per_monitor_dpi_awareness() {
    use windows::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
    };
    // SAFETY: the call sets a process-wide mode, takes no pointer and only fails (FALSE)
    // when the process or thread already has a DPI awareness set.
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
}

/// Size of the window's client area, in physical pixels, as Win32 reports it.
pub fn window_client_size(hwnd: HWND) -> Option<(i32, i32)> {
    let mut client = RECT::default();
    // SAFETY: the handle belongs to a live window and `client` is a valid RECT.
    unsafe { GetClientRect(hwnd, &mut client) }.ok()?;
    Some((client.right - client.left, client.bottom - client.top))
}

/// The DPI scale of the monitor the window is on (1.0 at 96 DPI).
pub fn window_scale(hwnd: HWND) -> f64 {
    scale_of(hwnd)
}

fn scale_of(hwnd: HWND) -> f64 {
    // SAFETY: the handle belongs to a live window.
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 { 1.0 } else { f64::from(dpi) / 96.0 }
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match message {
        // The client area covers the whole window: that is what removes the native
        // caption while keeping the sizing frame and the drop shadow.
        //
        // A maximised window is the exception. Windows places it so the resize border sits
        // off the screen edge, which with "client = window" pushed the top of the caption
        // (and the buttons' hover square) past the edge: the band looked cut off. Handing a
        // maximised window exactly its monitor's work area keeps the whole band on screen
        // and gives it the same geometry Windows itself would use.
        WM_NCCALCSIZE if wparam.0 != 0 => {
            if unsafe { IsZoomed(hwnd) }.as_bool() {
                let params = lparam.0 as *mut NCCALCSIZE_PARAMS;
                if !params.is_null() {
                    let (border_x, border_y) = frame_border(hwnd);
                    // SAFETY: WM_NCCALCSIZE with wParam TRUE passes a valid
                    // NCCALCSIZE_PARAMS whose rgrc[0] is the proposed client rectangle.
                    unsafe {
                        let client = &mut (*params).rgrc[0];
                        client.left += border_x;
                        client.right -= border_x;
                        client.top += border_y;
                        client.bottom -= border_y;
                    }
                }
            }
            LRESULT(0)
        }

        WM_NCHITTEST => {
            if let Some(state) = state_of(hwnd)
                && let Some(hit) = hit_test(hwnd, &state, lparam)
            {
                return LRESULT(hit as isize);
            }
            unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
        }

        WM_GETMINMAXINFO => {
            let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
            if let Some(state) = state_of(hwnd) {
                let info = lparam.0 as *mut MINMAXINFO;
                if !info.is_null() {
                    let scale = scale_of(hwnd);
                    unsafe {
                        (*info).ptMinTrackSize.x = (state.metrics.min_width * scale).round() as i32;
                        (*info).ptMinTrackSize.y =
                            (state.metrics.min_height * scale).round() as i32;
                    }
                }
            }
            result
        }

        WM_SIZE => {
            let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
            if let Some(state) = state_of(hwnd) {
                let maximized = unsafe { IsZoomed(hwnd).as_bool() };
                if state.maximized.replace(maximized) != maximized {
                    (state.on_maximized)(maximized);
                }
            }
            result
        }

        _ => unsafe { DefSubclassProc(hwnd, message, wparam, lparam) },
    }
}

/// Thickness of the resize border Windows keeps around a resizable window, in physical
/// pixels (frame plus the padded border).
fn frame_border(hwnd: HWND) -> (i32, i32) {
    let dpi = (scale_of(hwnd) * 96.0) as u32;
    // SAFETY: GetSystemMetricsForDpi is a pure query for the given DPI.
    let border_x = unsafe {
        GetSystemMetricsForDpi(SM_CXSIZEFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
    };
    // SAFETY: as above, queried for the vertical frame metric.
    let border_y = unsafe {
        GetSystemMetricsForDpi(SM_CYSIZEFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
    };
    (border_x, border_y)
}

/// The work area of the monitor that owns the window, in screen coordinates.
fn work_area_rect(hwnd: HWND) -> Option<RECT> {
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
    };
    // SAFETY: the handle is a live window; MonitorFromWindow only looks it up.
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_invalid() {
        return None;
    }
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info.cbSize` is set to the size of the struct the API expects to write.
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return None;
    }
    Some(info.rcWork)
}

/// Which non-client region the screen point falls in, or `None` when it is client area
/// (the file list, the tab strip, the caption buttons: everything Slint owns).
fn hit_test(hwnd: HWND, state: &ChromeState, lparam: LPARAM) -> Option<u32> {
    let scale = scale_of(hwnd);
    let maximized = unsafe { IsZoomed(hwnd).as_bool() };

    let mut client = RECT::default();
    // SAFETY: the handle belongs to a live window.
    unsafe {
        GetClientRect(hwnd, &mut client).ok()?;
    }
    let width = client.right - client.left;
    let height = client.bottom - client.top;

    let mut point = POINT {
        x: (lparam.0 & 0xffff) as i16 as i32,
        y: ((lparam.0 >> 16) & 0xffff) as i16 as i32,
    };
    // SAFETY: converting the screen point of a live window into its client space.
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut point);
    }
    let (x, y) = (point.x, point.y);

    // Resize band. A maximised window has no edges to drag: Windows would restore it on
    // the first pixel of movement instead of resizing.
    if !maximized {
        let border = unsafe {
            GetSystemMetricsForDpi(SM_CXSIZEFRAME, (scale * 96.0) as u32)
                + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, (scale * 96.0) as u32)
        };
        let border_y = unsafe {
            GetSystemMetricsForDpi(SM_CYSIZEFRAME, (scale * 96.0) as u32)
                + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, (scale * 96.0) as u32)
        };
        let left = x < border;
        let right = x >= width - border;
        let top = y < border_y;
        let bottom = y >= height - border_y;
        match (left, right, top, bottom) {
            (true, _, true, _) => return Some(HTTOPLEFT),
            (_, true, true, _) => return Some(HTTOPRIGHT),
            (true, _, _, true) => return Some(HTBOTTOMLEFT),
            (_, true, _, true) => return Some(HTBOTTOMRIGHT),
            (true, _, _, _) => return Some(HTLEFT),
            (_, true, _, _) => return Some(HTRIGHT),
            (_, _, true, _) => return Some(HTTOP),
            (_, _, _, true) => return Some(HTBOTTOM),
            _ => {}
        }
    }

    // The caption band drags the window, except over the buttons: those are Slint's and
    // keep their hover state and click handling.
    let caption = (state.metrics.caption_height * scale).round() as i32;
    if y < caption {
        let buttons =
            (state.metrics.button_width * scale).round() as i32 * state.metrics.button_count as i32;
        if x < width - buttons {
            return Some(HTCAPTION);
        }
    }

    None
}
