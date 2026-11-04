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
3. In the overlay editor:
   - **Add text** or **Add image…** (PNG, JPEG, WebP or SVG). Drag an overlay on the preview to place it, or set its **Position** and **Offset**. The offset is measured from the chosen corner, edge or centre, so the overlay keeps its place when the window is resized.
   - **Anchor** places the overlay in the **Application window** or on the **Monitor showing the application**.
   - Clear an overlay's check box to turn it off. **Duplicate** copies the selected overlay.
   - **Add group**, name it and **Record…** a hotkey. Put overlays in the group with **Group**. While the application is in front, the hotkey shows or hides the group's overlays; each group toggles on its own, and all groups start hidden. Overlays in no group are always shown.
   - The preview size at the top right only changes the preview.
4. Select **Overlays…**, **Edit…** or **Remove** on the **Applications** tab to change an application later. Double-click an application to open its overlays.
5. On the **Settings** tab, enable **Start with Windows** and optionally **Start minimized to the tray**, choose a **Language**, or **Open** the settings folder.

Settings are saved immediately to `settings.json` beside `z-overlay.exe`, and added images are copied into the `images` folder beside it, so the folder can be moved or copied as a whole. The folder must be writable. Moving it while **Start with Windows** is on is fine: the startup entry is updated the next time z-overlay starts.

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
