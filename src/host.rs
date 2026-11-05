//! Finds the managed application in the foreground and shows its overlays in
//! layered, topmost windows: click-through normally, draggable while editing.

use crate::processes::process_identity;
use resvg::tiny_skia::{
    Color as SkiaColor, Paint, Pixmap, PixmapPaint, Rect as SkiaRect, Transform,
};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::mem::zeroed;
use std::path::Path;
use std::ptr::{null, null_mut};
use std::rc::Rc;
use std::time::SystemTime;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, ClientToScreen,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetMonitorInfoW,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, SelectObject,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, EnumWindows, GA_ROOTOWNER, GW_OWNER,
    GetAncestor, GetClientRect, GetCursorPos, GetForegroundWindow, GetWindow, GetWindowRect,
    GetWindowThreadProcessId, HWND_TOPMOST, IDC_SIZEALL, IsIconic, IsWindow, IsWindowVisible,
    LoadCursorW, MA_NOACTIVATE, RegisterClassExW, SW_HIDE, SWP_NOACTIVATE, SWP_NOSIZE,
    SWP_NOZORDER, SWP_SHOWWINDOW, SetCursor, SetWindowPos, ShowWindow, ULW_ALPHA,
    UpdateLayeredWindow, WM_CAPTURECHANGED, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE,
    WM_MOUSEMOVE, WM_RBUTTONUP, WM_SETCURSOR, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use z_overlay::Result;
use z_overlay::groups::GroupToggles;
use z_overlay::placement::{Rect, place};
use z_overlay::render::{Renderer, to_bgra};
use z_overlay::settings::{Anchor, Content, ManagedApplication, Overlay, Settings};
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
        foreground_of(window, self.app?)
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

/// The window of `app` the user most likely means: its largest visible
/// top-level window. Used to edit overlays on screen while z-overlay is in front.
pub fn find_app_window(app: &ManagedApplication) -> Option<HWND> {
    struct Search<'a> {
        app: &'a ManagedApplication,
        best: Option<(HWND, i64)>,
    }
    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> i32 {
        let search = unsafe { &mut *(lparam as *mut Search) };
        let candidate = unsafe {
            IsWindowVisible(hwnd) != 0
                && GetWindow(hwnd, GW_OWNER).is_null()
                && managed_app(hwnd, std::slice::from_ref(search.app)).is_some()
        };
        if candidate && let Some(rect) = unsafe { client_rect_on_screen(hwnd) } {
            let area = i64::from(rect.width) * i64::from(rect.height);
            if search.best.is_none_or(|(_, best)| area > best) {
                search.best = Some((hwnd, area));
            }
        }
        1
    }
    let mut search = Search { app, best: None };
    unsafe { EnumWindows(Some(visit), &mut search as *mut Search as LPARAM) };
    search.best.map(|(hwnd, _)| hwnd)
}

/// Where a window of managed application `app` is, unless it is gone or minimized.
pub fn foreground_of(window: HWND, app: usize) -> Option<Foreground> {
    unsafe {
        if IsWindow(window) == 0 || IsIconic(window) != 0 {
            return None;
        }
        Some(Foreground {
            app,
            window: client_rect_on_screen(window)?,
            monitor: monitor_rect(window)?,
        })
    }
}

/// When and how large an overlay's image file is, so an edited file is noticed.
pub fn image_stamp(base_dir: &Path, overlay: &Overlay) -> Option<(SystemTime, u64)> {
    let Content::Image { path } = &overlay.content else {
        return None;
    };
    let meta = std::fs::metadata(base_dir.join(path)).ok()?;
    Some((meta.modified().ok()?, meta.len()))
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
        // An image file saved again gets a new key, and so is drawn again.
        let key = format!(
            "{}|{}|{:?}",
            serde_json::to_string(&overlay.content).unwrap_or_default(),
            overlay.scale,
            image_stamp(base_dir, overlay),
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
}

/// How the overlays of the application are shown.
pub enum Mode<'a> {
    /// Visible overlays only, click-through.
    Normal(&'a GroupToggles),
    /// Every overlay, outlined and draggable, under a banner that says how to finish.
    Editing { banner: &'a Pixmap },
}

/// What the user did to an overlay window while editing on screen.
#[derive(Clone, Copy, Debug)]
pub enum EditEvent {
    /// The left button went down on the overlay.
    Pressed(usize),
    /// The overlay was dropped with its top-left corner here.
    Dropped(usize, (i32, i32)),
    /// The right button was clicked on the overlay, at this screen point.
    Menu(usize, (i32, i32)),
}

#[derive(Clone, Copy)]
struct Drag {
    hwnd: HWND,
    grab: (i32, i32),
}

/// What happened to a window, as the window procedure saw it.
#[derive(Clone, Copy)]
enum WindowEvent {
    Pressed(HWND),
    Dropped(HWND, (i32, i32)),
    Menu(HWND, (i32, i32)),
}

thread_local! {
    static EVENTS: RefCell<Vec<WindowEvent>> = const { RefCell::new(Vec::new()) };
    static DRAG: Cell<Option<Drag>> = const { Cell::new(None) };
}

/// One window per overlay of the application in front.
#[derive(Default)]
pub struct OverlayHost {
    windows: Vec<Option<OverlayWindow>>,
    app: Option<usize>,
    editing: bool,
    banner: Option<OverlayWindow>,
}

impl OverlayHost {
    /// Closes every window; they are recreated from the settings on the next update.
    pub fn reset(&mut self) {
        self.windows.clear();
        self.banner = None;
        self.app = None;
    }

    pub fn update(
        &mut self,
        foreground: Option<Foreground>,
        settings: &Settings,
        mode: Mode,
        cache: &mut RenderCache,
        renderer: &Renderer,
        base_dir: &Path,
    ) {
        let editing = matches!(mode, Mode::Editing { .. });
        if editing != self.editing {
            self.reset();
            self.editing = editing;
        }
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
        let dragged = DRAG.get().map(|drag| drag.hwnd);

        for (index, overlay) in app.overlays.iter().enumerate() {
            let slot = &mut self.windows[index];
            let visible = match mode {
                Mode::Normal(toggles) => toggles.is_visible(app, overlay),
                Mode::Editing { .. } => true,
            };
            if !visible {
                if let Some(window) = slot {
                    window.hide();
                }
                continue;
            }
            if slot.is_none() {
                let Ok(pixmap) = cache.get(renderer, base_dir, overlay) else {
                    continue;
                };
                *slot = if editing {
                    // Turned-off overlays stay in reach, faded.
                    let opacity = overlay.opacity * if overlay.enabled { 1.0 } else { 0.35 };
                    OverlayWindow::create(&outlined(&pixmap), opacity.max(0.2), true)
                } else {
                    OverlayWindow::create(&pixmap, overlay.opacity, false)
                }
                .ok();
            }
            if let Some(window) = slot
                && Some(window.hwnd) != dragged
            {
                let area = foreground.area(overlay.anchor);
                window.show_at(place(
                    area,
                    overlay.align,
                    overlay.pivot(),
                    overlay.offset,
                    window.size,
                ));
            }
        }

        if let Mode::Editing { banner } = mode {
            if self.banner.is_none() {
                self.banner = OverlayWindow::create(banner, 1.0, false).ok();
            }
            if let Some(window) = &mut self.banner {
                let area = foreground.window;
                window.show_at((area.x + (area.width - window.size.0) / 2, area.y + 12));
            }
        }
    }

    /// The size of an overlay's window, as last drawn.
    pub fn size_of(&self, index: usize) -> Option<(i32, i32)> {
        Some(self.windows.get(index)?.as_ref()?.size)
    }

    /// What happened to the overlay windows since the last call.
    pub fn take_events(&self) -> Vec<EditEvent> {
        let index_of = |hwnd: HWND| {
            self.windows
                .iter()
                .position(|window| window.as_ref().is_some_and(|window| window.hwnd == hwnd))
        };
        EVENTS
            .with(|events| std::mem::take(&mut *events.borrow_mut()))
            .into_iter()
            .filter_map(|event| match event {
                WindowEvent::Pressed(hwnd) => Some(EditEvent::Pressed(index_of(hwnd)?)),
                WindowEvent::Dropped(hwnd, at) => Some(EditEvent::Dropped(index_of(hwnd)?, at)),
                WindowEvent::Menu(hwnd, at) => Some(EditEvent::Menu(index_of(hwnd)?, at)),
            })
            .collect()
    }

    fn hide_all(&mut self) {
        for window in self
            .windows
            .iter_mut()
            .flatten()
            .chain(self.banner.as_mut())
        {
            window.hide();
        }
    }
}

/// The overlay over a faint tint inside a thin frame, so every part of it can
/// be seen and grabbed, transparent or not.
fn outlined(content: &Pixmap) -> Pixmap {
    let (width, height) = (content.width(), content.height());
    let mut pixmap = Pixmap::new(width, height).expect("the overlay has a size");
    pixmap.fill(SkiaColor::from_rgba8(0, 120, 215, 40));
    pixmap.draw_pixmap(
        0,
        0,
        content.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        None,
    );
    let mut paint = Paint::default();
    paint.set_color_rgba8(0, 120, 215, 255);
    let (w, h) = (width as f32, height as f32);
    for (x, y, rect_width, rect_height) in [
        (0.0, 0.0, w, 1.0),
        (0.0, h - 1.0, w, 1.0),
        (0.0, 0.0, 1.0, h),
        (w - 1.0, 0.0, 1.0, h),
    ] {
        if let Some(rect) = SkiaRect::from_xywh(x, y, rect_width, rect_height) {
            pixmap.fill_rect(rect, &paint, Transform::identity(), None);
        }
    }
    pixmap
}

struct OverlayWindow {
    hwnd: HWND,
    size: (i32, i32),
    shown_at: Option<(i32, i32)>,
}

impl OverlayWindow {
    /// An `interactive` window can be dragged and right-clicked; any other
    /// lets every click through to the window below.
    fn create(pixmap: &Pixmap, opacity: f32, interactive: bool) -> Result<Self> {
        register_class();
        let class = wide(CLASS_NAME);
        let click_through = if interactive { 0 } else { WS_EX_TRANSPARENT };
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | click_through | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
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
        if DRAG.get().is_some_and(|drag| drag.hwnd == self.hwnd) {
            DRAG.set(None);
        }
        unsafe { DestroyWindow(self.hwnd) };
    }
}

fn register_class() {
    thread_local! {
        static REGISTERED: Cell<bool> = const { Cell::new(false) };
    }
    REGISTERED.with(|registered| {
        if registered.get() {
            return;
        }
        let class = wide(CLASS_NAME);
        let info = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: unsafe { GetModuleHandleW(null()) },
            lpszClassName: class.as_ptr(),
            ..unsafe { zeroed() }
        };
        unsafe { RegisterClassExW(&info) };
        registered.set(true);
    });
}

fn push(event: WindowEvent) {
    EVENTS.with(|events| events.borrow_mut().push(event));
}

fn cursor() -> (i32, i32) {
    let mut point = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut point) };
    (point.x, point.y)
}

fn top_left(hwnd: HWND) -> (i32, i32) {
    let mut rect: RECT = unsafe { zeroed() };
    unsafe { GetWindowRect(hwnd, &mut rect) };
    (rect.left, rect.top)
}

/// Only windows made interactive for editing get mouse input; they move with
/// the cursor while the left button is held, without ever taking the focus.
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_MOUSEACTIVATE => return MA_NOACTIVATE as LRESULT,
        WM_SETCURSOR => {
            unsafe { SetCursor(LoadCursorW(null_mut(), IDC_SIZEALL)) };
            return 1;
        }
        WM_LBUTTONDOWN => {
            let (cursor, origin) = (cursor(), top_left(hwnd));
            DRAG.set(Some(Drag {
                hwnd,
                grab: (cursor.0 - origin.0, cursor.1 - origin.1),
            }));
            unsafe { SetCapture(hwnd) };
            push(WindowEvent::Pressed(hwnd));
            return 0;
        }
        WM_MOUSEMOVE => {
            if let Some(drag) = DRAG.get()
                && drag.hwnd == hwnd
            {
                let cursor = cursor();
                unsafe {
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        cursor.0 - drag.grab.0,
                        cursor.1 - drag.grab.1,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                    )
                };
            }
            return 0;
        }
        // Releasing the capture ends the drag in WM_CAPTURECHANGED, which also
        // covers the capture being taken away.
        WM_LBUTTONUP => {
            unsafe { ReleaseCapture() };
            return 0;
        }
        WM_CAPTURECHANGED => {
            if DRAG.get().is_some_and(|drag| drag.hwnd == hwnd) {
                DRAG.set(None);
                push(WindowEvent::Dropped(hwnd, top_left(hwnd)));
            }
            return 0;
        }
        WM_RBUTTONUP => {
            push(WindowEvent::Menu(hwnd, cursor()));
            return 0;
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}
