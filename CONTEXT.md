# Glossary

## z-overlay

The Windows application that shows text and images over a managed application's window while that application is in the foreground.

## Managed application

An application configured by selecting a running process or entering an executable name or path. Its executable path identifies it when available; its process name is the fallback identity. These are two forms of the same setting, not separate kinds of managed application. An optional display name is shown instead of the identity; it does not affect matching. Each managed application has its own overlays and groups.

## Overlay

A piece of text or an image drawn over a managed application. It never takes input; clicks go through to the application. It is shown only while its managed application is in the foreground, it is enabled, and it is either in no group or in a shown group.

## Anchor

The area an overlay is positioned in: the content area of the application window, or the monitor showing it.

## Alignment

The point of the anchor area and of the overlay that are lined up — a corner, an edge centre or the centre. The overlay's offset is measured from that point, so an overlay aligned to a corner stays in that corner when the window is resized.

## Group

A set of a managed application's overlays shown and hidden together by the group's hotkey. Each press toggles the group; groups toggle independently, and all groups are hidden when z-overlay starts.

## Hotkey

A key, optionally with Ctrl, Alt and Shift, that toggles a group while its managed application is in the foreground. z-overlay reads key state without hooking or consuming the key, so the application still receives it.

## Portable folder

The folder containing `z-overlay.exe`. It holds `settings.json` and the `images/` folder that imported images are copied into, so the whole folder can be moved or copied as one.

## Overlay editor

The page in z-overlay's window that lists a managed application's overlays and groups and shows a scaled preview canvas on which overlays are dragged into place.

## Update

A newer release found on GitHub. It is offered only at the bottom right of the Settings tab; installing it replaces `z-overlay.exe` in the portable folder, keeping the previous one as `z-overlay.exe.old` until the new one starts and removes it.
