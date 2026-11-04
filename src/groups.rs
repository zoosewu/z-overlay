use crate::settings::{Hotkey, ManagedApplication, Overlay};
use std::collections::{BTreeSet, HashMap};

pub const VK_SHIFT: u16 = 0x10;
pub const VK_CONTROL: u16 = 0x11;
pub const VK_MENU: u16 = 0x12;

/// Which groups are shown, per managed application. Nothing is shown at start.
#[derive(Default)]
pub struct GroupToggles {
    shown: HashMap<String, BTreeSet<u32>>,
}

impl GroupToggles {
    pub fn toggle(&mut self, app: &ManagedApplication, group: u32) {
        let shown = self.shown.entry(app.identity.clone()).or_default();
        if !shown.remove(&group) {
            shown.insert(group);
        }
    }

    pub fn is_shown(&self, app: &ManagedApplication, group: u32) -> bool {
        self.shown
            .get(&app.identity)
            .is_some_and(|shown| shown.contains(&group))
    }

    /// Whether an overlay is shown while its application is in front.
    pub fn is_visible(&self, app: &ManagedApplication, overlay: &Overlay) -> bool {
        overlay.enabled
            && match overlay.group {
                None => true,
                // An overlay left pointing at a removed group behaves as ungrouped.
                Some(id) if app.group(id).is_none() => true,
                Some(id) => self.is_shown(app, id),
            }
    }
}

/// Turns held keys into one press per hotkey, so holding a hotkey toggles once.
#[derive(Default)]
pub struct HotkeyPresses {
    held: BTreeSet<(bool, bool, bool, u16)>,
}

impl HotkeyPresses {
    /// Returns the ids of the groups whose hotkey went down since the last poll.
    pub fn poll(&mut self, app: &ManagedApplication, is_down: impl Fn(u16) -> bool) -> Vec<u32> {
        let modifiers = (is_down(VK_CONTROL), is_down(VK_MENU), is_down(VK_SHIFT));
        let mut held = BTreeSet::new();
        let mut pressed = Vec::new();
        for group in &app.groups {
            let Some(hotkey) = group.hotkey else {
                continue;
            };
            if (hotkey.ctrl, hotkey.alt, hotkey.shift) == modifiers && is_down(hotkey.key) {
                let key = key_of(hotkey);
                if !self.held.contains(&key) {
                    pressed.push(group.id);
                }
                held.insert(key);
            }
        }
        self.held = held;
        pressed
    }
}

fn key_of(hotkey: Hotkey) -> (bool, bool, bool, u16) {
    (hotkey.ctrl, hotkey.alt, hotkey.shift, hotkey.key)
}

/// Whether a virtual key can be a hotkey's main key: anything but the
/// modifiers themselves and the primary mouse buttons.
pub fn is_hotkey_key(key: u16) -> bool {
    !matches!(
        key,
        0x01 | 0x02 // left and right mouse buttons
            | VK_SHIFT | VK_CONTROL | VK_MENU
            | 0xA0
            ..=0xA5 // left and right Shift, Ctrl, Alt
            | 0x5B | 0x5C // Windows keys
    ) && (0x01..=0xFE).contains(&key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn app() -> ManagedApplication {
        let mut app = ManagedApplication::new("game.exe".into(), None);
        for (name, ctrl, key) in [
            ("One", false, 0x31),
            ("Two", true, 0x31),
            ("Bare", false, 0),
        ] {
            let index = app.add_group(name.into());
            app.groups[index].hotkey = (key != 0).then_some(Hotkey {
                ctrl,
                alt: false,
                shift: false,
                key,
            });
        }
        app
    }

    fn overlay(group: Option<u32>, enabled: bool) -> Overlay {
        let mut overlay = Overlay::text("x".into(), "x".into());
        overlay.group = group;
        overlay.enabled = enabled;
        overlay
    }

    #[test]
    fn ungrouped_overlays_show_and_grouped_ones_wait_for_their_group() {
        let app = app();
        let mut toggles = GroupToggles::default();
        assert!(toggles.is_visible(&app, &overlay(None, true)));
        assert!(!toggles.is_visible(&app, &overlay(None, false)));
        assert!(!toggles.is_visible(&app, &overlay(Some(1), true)));
        assert!(toggles.is_visible(&app, &overlay(Some(99), true)));

        toggles.toggle(&app, 1);
        assert!(toggles.is_visible(&app, &overlay(Some(1), true)));
        assert!(!toggles.is_visible(&app, &overlay(Some(1), false)));
        assert!(!toggles.is_visible(&app, &overlay(Some(2), true)));

        // Groups toggle independently.
        toggles.toggle(&app, 2);
        assert!(toggles.is_shown(&app, 1) && toggles.is_shown(&app, 2));
        toggles.toggle(&app, 1);
        assert!(!toggles.is_shown(&app, 1) && toggles.is_shown(&app, 2));

        // Toggles belong to one application.
        let other = ManagedApplication::new("other.exe".into(), None);
        assert!(!toggles.is_shown(&other, 2));
    }

    #[test]
    fn a_held_hotkey_presses_once_and_modifiers_must_match_exactly() {
        let app = app();
        let down = RefCell::new(BTreeSet::<u16>::new());
        let is_down = |key| down.borrow().contains(&key);
        let mut presses = HotkeyPresses::default();

        assert!(presses.poll(&app, is_down).is_empty());
        down.borrow_mut().insert(0x31);
        assert_eq!(presses.poll(&app, is_down), [1]);
        assert!(presses.poll(&app, is_down).is_empty());
        down.borrow_mut().remove(&0x31);
        assert!(presses.poll(&app, is_down).is_empty());

        down.borrow_mut().extend([VK_CONTROL, 0x31]);
        assert_eq!(presses.poll(&app, is_down), [2]);

        down.borrow_mut().insert(VK_SHIFT);
        down.borrow_mut().remove(&0x31);
        presses.poll(&app, is_down);
        down.borrow_mut().insert(0x31);
        assert!(presses.poll(&app, is_down).is_empty());
    }

    #[test]
    fn modifiers_and_primary_buttons_are_not_hotkey_keys() {
        assert!(is_hotkey_key(0x31));
        assert!(is_hotkey_key(0x70));
        assert!(is_hotkey_key(0x05)); // mouse side button
        for key in [
            0x00, 0x01, 0x02, VK_SHIFT, VK_CONTROL, VK_MENU, 0xA2, 0x5B, 0xFF,
        ] {
            assert!(!is_hotkey_key(key), "{key:#x}");
        }
    }
}
