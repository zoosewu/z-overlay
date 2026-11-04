//! Finds the managed application in the foreground and shows its overlays in
//! click-through, layered, topmost windows.

use crate::processes::process_identity;
use resvg::tiny_skia::Pixmap;
use std::collections::HashMap;
use std::mem::zeroed;
use std::path::Path;
use std::ptr::{null, null_mut};
use std::rc::Rc;
use windows_sys::Win32::Foundation::{HWND, POINT, RECT, SIZE};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, ClientToScreen,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetMonitorInfoW,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, SelectObject,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GA_ROOTOWNER, GetAncestor, GetClientRect,
    GetForegroundWindow, GetWindowThreadProcessId, HWND_TOPMOST, IsIconic, RegisterClassExW,
    SW_HIDE, SWP_NOACTIVATE, SWP_NOSIZE, SWP_SHOWWINDOW, SetWindowPos, ShowWindow, ULW_ALPHA,
    UpdateLayeredWindow, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use z_overlay::Result;
use z_overlay::groups::GroupToggles;
use z_overlay::placement::{Rect, place};
use z_overlay::render::{Renderer, to_bgra};
use z_overlay::settings::{Anchor, ManagedApplication, Overlay, Settings};
use z_overlay::system::wide;

const CLASS_NAME: &str = "z-overlay.Overlay";

/// The managed application in the foreground and where it is on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Foreground {
    pub app: usize,
    pub window: Rect,
    pub monitor: Rect,
}

impl Foreground {
    pub fn area(&self, anchor: Anchor) -> Rect {
        match anchor {
            Anchor::Window => self.window,
            Anchor::Monitor => self.monitor,
        }
    }
}

/// Remembers which application owns the foreground window, so the process is
/// only looked up when the foreground window changes.
pub struct ForegroundTracker {
    window: HWND,
    app: Option<usize>,
}

impl Default for ForegroundTracker {
    fn default() -> Self {
        Self {
            window: null_mut(),
            app: None,
        }
    }
}

impl ForegroundTracker {
    /// Call after the managed applications change.
    pub fn forget(&mut self) {
        self.window = null_mut();
    }

    pub fn update(&mut self, apps: &[ManagedApplication]) -> Option<Foreground> {
        let foreground = unsafe { GetForegroundWindow() };
        if foreground.is_null() {
            self.window = null_mut();
            return None;
        }
        // A dialog of the application is placed by the window that owns it.
        let window = match unsafe { GetAncestor(foreground, GA_ROOTOWNER) } {
            root if root.is_null() => foreground,
            root => root,
        };
        if window != self.window {
            self.window = window;
            self.app = managed_app(window, apps);
        }
        let app = self.app?;
        if unsafe { IsIconic(window) } != 0 {
            return None;
        }
        let (window_rect, monitor) =
            unsafe { (client_rect_on_screen(window)?, monitor_rect(window)?) };
        Some(Foreground {
            app,
            window: window_rect,
            monitor,
        })
    }
}

fn managed_app(window: HWND, apps: &[ManagedApplication]) -> Option<usize> {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(window, &mut pid) };
    if pid == 0 || pid == std::process::id() {
        return None;
    }
    let identity = unsafe { process_identity(pid) }?;
    apps.iter()
        .position(|app| app.matches(&identity.name, identity.path.as_deref()))
}

unsafe fn client_rect_on_screen(hwnd: HWND) -> Option<Rect> {
    let mut rect: RECT = zeroed();
    if GetClientRect(hwnd, &mut rect) == 0 {
        return None;
    }
    let mut origin = POINT { x: 0, y: 0 };
    if ClientToScreen(hwnd, &mut origin) == 0 {
        return None;
    }
    Some(Rect {
        x: origin.x,
        y: origin.y,
        width: rect.right - rect.left,
        height: rect.bottom - rect.top,
    })
}

unsafe fn monitor_rect(hwnd: HWND) -> Option<Rect> {
    let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
    if monitor.is_null() {
        return None;
    }
    let mut info: MONITORINFO = zeroed();
    info.cbSize = size_of::<MONITORINFO>() as u32;
    if GetMonitorInfoW(monitor, &mut info) == 0 {
        return None;
    }
    let rect = info.rcMonitor;
    Some(Rect {
        x: rect.left,
        y: rect.top,
        width: rect.right - rect.left,
        height: rect.bottom - rect.top,
    })
}

/// Rendered overlays keyed by what they draw, shared by the overlay windows
/// and the editor preview. A failed render is kept as its error message.
#[derive(Default)]
pub struct RenderCache {
    entries: HashMap<String, std::result::Result<Rc<Pixmap>, String>>,
}

impl RenderCache {
    const LIMIT: usize = 256;

    pub fn get(
        &mut self,
        renderer: &Renderer,
        base_dir: &Path,
        overlay: &Overlay,
    ) -> std::result::Result<Rc<Pixmap>, String> {
        let key = format!(
            "{}|{}",
            serde_json::to_string(&overlay.content).unwrap_or_default(),
            overlay.scale
        );
        if let Some(entry) = self.entries.get(&key) {
            return entry.clone();
        }
        if self.entries.len() >= Self::LIMIT {
            self.entries.clear();
        }
        let entry = renderer
            .render(&overlay.content, base_dir, overlay.scale)
            .map(Rc::new)
            .map_err(|error| error.to_string());
        self.entries.insert(key, entry.clone());
        entry
    }

    /// Drops everything, so images changed on disk are read again.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// One click-through window per overlay of the application in front.
#[derive(Default)]
pub struct OverlayHost {
    windows: Vec<Option<OverlayWindow>>,
    app: Option<usize>,
}

impl OverlayHost {
    /// Closes every window; they are recreated from the settings on the next update.
    pub fn reset(&mut self) {
        self.windows.clear();
        self.app = None;
    }

    pub fn update(
        &mut self,
        foreground: Option<Foreground>,
        settings: &Settings,
        toggles: &GroupToggles,
        cache: &mut RenderCache,
        renderer: &Renderer,
        base_dir: &Path,
    ) {
        let app = foreground
            .and_then(|foreground| Some((foreground, settings.apps.get(foreground.app)?)));
        let Some((foreground, app)) = app else {
            self.hide_all();
            return;
        };
        if self.app != Some(foreground.app) {
            self.reset();
            self.app = Some(foreground.app);
        }
        self.windows.resize_with(app.overlays.len(), || None);

        for (index, overlay) in app.overlays.iter().enumerate() {
            let slot = &mut self.windows[index];
            if !toggles.is_visible(app, overlay) {
                if let Some(window) = slot {
                    window.hide();
                }
                continue;
            }
            if slot.is_none() {
                let Ok(pixmap) = cache.get(renderer, base_dir, overlay) else {
                    continue;
                };
                *slot = OverlayWindow::create(&pixmap, overlay.opacity).ok();
            }
            if let Some(window) = slot {
                let area = foreground.area(overlay.anchor);
                window.show_at(place(area, overlay.align, overlay.offset, window.size));
            }
        }
    }

    fn hide_all(&mut self) {
        for window in self.windows.iter_mut().flatten() {
            window.hide();
        }
    }
}

struct OverlayWindow {
    hwnd: HWND,
    size: (i32, i32),
    shown_at: Option<(i32, i32)>,
}

impl OverlayWindow {
    fn create(pixmap: &Pixmap, opacity: f32) -> Result<Self> {
        register_class();
        let class = wide(CLASS_NAME);
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE
                    | WS_EX_TOPMOST,
                class.as_ptr(),
                class.as_ptr(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            )
        };
        if hwnd.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        let window = Self {
            hwnd,
            size: (pixmap.width() as i32, pixmap.height() as i32),
            shown_at: None,
        };
        window.paint(pixmap, opacity)?;
        Ok(window)
    }

    /// Hands the premultiplied pixels to Windows, which composes them itself.
    fn paint(&self, pixmap: &Pixmap, opacity: f32) -> Result<()> {
        let (width, height) = self.size;
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let mut info: BITMAPINFO = zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..zeroed()
            };
            let mut bits = null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if bitmap.is_null() {
                DeleteDC(dc);
                return Err(std::io::Error::last_os_error().into());
            }
            let pixels = to_bgra(pixmap);
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u8>(), pixels.len());
            let previous = SelectObject(dc, bitmap);
            let size = SIZE {
                cx: width,
                cy: height,
            };
            let source = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: (opacity.clamp(0.0, 1.0) * 255.0).round() as u8,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let painted = UpdateLayeredWindow(
                self.hwnd,
                null_mut(),
                null(),
                &size,
                dc,
                &source,
                0,
                &blend,
                ULW_ALPHA,
            );
            SelectObject(dc, previous);
            DeleteObject(bitmap);
            DeleteDC(dc);
            if painted == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        Ok(())
    }

    fn show_at(&mut self, position: (i32, i32)) {
        if self.shown_at == Some(position) {
            return;
        }
        unsafe {
            SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                position.0,
                position.1,
                0,
                0,
                SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
        };
        self.shown_at = Some(position);
    }

    fn hide(&mut self) {
        if self.shown_at.take().is_some() {
            unsafe { ShowWindow(self.hwnd, SW_HIDE) };
        }
    }
}

impl Drop for OverlayWindow {
    fn drop(&mut self) {
        unsafe { DestroyWindow(self.hwnd) };
    }
}

fn register_class() {
    thread_local! {
        static REGISTERED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    REGISTERED.with(|registered| {
        if registered.get() {
            return;
        }
        let class = wide(CLASS_NAME);
        let info = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(DefWindowProcW),
            hInstance: unsafe { GetModuleHandleW(null()) },
            lpszClassName: class.as_ptr(),
            ..unsafe { zeroed() }
        };
        unsafe { RegisterClassExW(&info) };
        registered.set(true);
    });
}
