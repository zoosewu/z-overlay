# z-overlay

A small, portable Windows 10/11 x64 utility that shows text and images over an application while that application is in the foreground. Overlays are click-through, can be grouped, and each group is shown and hidden with its own hotkey.

z-overlay runs without administrator rights and does not inject into or hook other applications: overlays are ordinary topmost windows, and hotkeys are read without consuming the key. It covers windowed and borderless full-screen applications, not exclusive full-screen ones. The interface is available in English and Traditional Chinese and follows the Windows display language by default.

<a href="https://slint.dev"><picture><source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-dark.svg"><img alt="#MadeWithSlint" src="https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-light.svg" height="60"></picture></a>

## Download

Download `z-overlay.exe` from the [latest release](https://github.com/zoosewu/z-overlay/releases/latest) and put it in a folder of its own. To verify it, compare the output below with `z-overlay.exe.sha256` from the same release:

```powershell
(Get-FileHash .\z-overlay.exe -Algorithm SHA256).Hash
```

## Use

1. Launch `z-overlay.exe`.
2. On the **Applications** tab, select **Add…** and pick a running application or enter its path or executable name. Optionally enter a display name. Select **Save** to open its overlays.
3. In the overlay editor, the list on the left holds the overlays: first those in no group, then each group with its overlays. The preview on the right shows how they sit in a window of the chosen size.
   - **+ Text** or **+ Image…** (PNG, JPEG, WebP or SVG). [`samples/crosshair.svg`](samples/crosshair.svg) is a crosshair whose length, thickness, centre gap and colour are set at the top of the file.
   - Each overlay's row has a check box to turn it on or off, **Copy** and **Delete**. Drag a row to move the overlay into another group or change its order.
   - Right-click an overlay, in the list, the preview or on screen, for its details: text and colours, **Anchor** (**Application window** or **Monitor showing the application**), **Position** (the point of that area it is placed from), **Image point** (the point of the overlay placed there, which stays put when the overlay changes size), **Offset**, **Scale** and **Opacity**.
   - **Edit on screen** shows every overlay over the application itself; drag them into place, right-click for details, and press Esc or **Done** to finish. Dragging in the preview works too.
   - **+ Group** adds a group. Each group's row has a check box to turn it on or off, its number, its name, the hotkey of its number, a button that switches between **Toggle** (each press shows or hides it; groups start hidden) and **Hold** (shown while the hotkey is held), and **Delete**. Overlays in no group are always shown.
   - Image files in use are checked every second, so an image saved again shows at once.
4. Select **Overlays…**, **Edit…** or **Remove** on the **Applications** tab to change an application later. Double-click an application to open its overlays.
5. On the **Settings** tab, set the **Group hotkeys** for numbers 1 to 9 (Ctrl+1 to Ctrl+9 at first; they apply to every application), enable **Start with Windows** and optionally **Start minimized to the tray**, choose a **Language**, or **Open** the settings folder.

Settings are saved immediately to `settings.json` beside `z-overlay.exe`, and added images are copied into the `images` folder beside it, so the folder can be moved or copied as a whole. The folder must be writable. Moving it while **Start with Windows** is on is fine: the startup entry is updated the next time z-overlay starts.

z-overlay checks GitHub for a newer release shortly after it starts and every six hours. When one is out, a dot appears on the **Settings** tab and the bottom right of that tab offers **Update and restart**, which downloads the new `z-overlay.exe`, verifies its SHA-256 checksum, replaces the running copy and starts it again. Settings and images are kept. Nothing pops up, and a failed check is simply retried later.

The tray icon remains visible while z-overlay is running. Minimize or close the window to hide it; left-click the tray icon to restore it, or right-click and choose **Exit** to quit. Only one instance can run at a time; launching it again restores the existing window.

## Build

```powershell
cargo build --release
```

The executable is written to `target\release\z-overlay.exe`.

Interface strings are marked with `@tr` in `ui/*.slint`. Traditional Chinese translations live in `lang/zh-TW/LC_MESSAGES/z-overlay.po` and are bundled into the executable. To list the strings, run:

```powershell
slint-tr-extractor --no-default-translation-context -o z-overlay.pot ui/app.slint ui/editor.slint ui/tray.slint
```

The icons are drawn by `tools/make-icons.py` (requires Pillow):

```powershell
python tools/make-icons.py assets
```
