use crate::settings::{GroupMode, Hotkey, ManagedApplication, Overlay};
use std::collections::{BTreeSet, HashMap};

pub const VK_SHIFT: u16 = 0x10;
pub const VK_CONTROL: u16 = 0x11;
pub const VK_MENU: u16 = 0x12;

/// Which groups are shown, per managed application. Toggled groups start
/// hidden; held groups show while their number's hotkey is held.
#[derive(Default)]
pub struct GroupToggles {
    shown: HashMap<String, BTreeSet<u32>>,
    held: BTreeSet<u8>,
}

impl GroupToggles {
    /// Applies a press of a number's hotkey to the application's toggled groups.
    pub fn press(&mut self, app: &ManagedApplication, number: u8) {
        let shown = self.shown.entry(app.identity.clone()).or_default();
        for group in &app.groups {
            if group.number == number && group.mode == GroupMode::Toggle && !shown.remove(&group.id)
            {
                shown.insert(group.id);
            }
        }
    }

    /// The numbers whose hotkeys are held down right now.
    pub fn set_held(&mut self, held: BTreeSet<u8>) {
        self.held = held;
    }

    pub fn is_shown(&self, app: &ManagedApplication, group: u32) -> bool {
        let Some(group) = app.group(group) else {
            return false;
        };
        group.enabled
            && match group.mode {
                GroupMode::Toggle => self
                    .shown
                    .get(&app.identity)
                    .is_some_and(|shown| shown.contains(&group.id)),
                GroupMode::Hold => self.held.contains(&group.number),
            }
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

/// What the number hotkeys did since the last poll.
#[derive(Debug, Default, PartialEq)]
pub struct HotkeyState {
    /// Numbers whose hotkey went down; holding it presses once.
    pub pressed: Vec<u8>,
    /// Numbers whose hotkey is down now.
    pub held: BTreeSet<u8>,
}

#[derive(Default)]
pub struct HotkeyPresses {
    held: BTreeSet<u8>,
}

impl HotkeyPresses {
    /// Reads the hotkey of each number, 1 first. A hotkey needs exactly its
    /// own modifiers: Ctrl+1 is not down while Ctrl+Shift+1 is.
    pub fn poll(
        &mut self,
        hotkeys: &[Option<Hotkey>],
        is_down: impl Fn(u16) -> bool,
    ) -> HotkeyState {
        let modifiers = (is_down(VK_CONTROL), is_down(VK_MENU), is_down(VK_SHIFT));
        let held: BTreeSet<u8> = hotkeys
            .iter()
            .zip(1u8..)
            .filter_map(|(hotkey, number)| {
                let hotkey = (*hotkey)?;
                ((hotkey.ctrl, hotkey.alt, hotkey.shift) == modifiers && is_down(hotkey.key))
                    .then_some(number)
            })
            .collect();
        let pressed = held.difference(&self.held).copied().collect();
        self.held = held.clone();
        HotkeyState { pressed, held }
    }
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

    /// Groups 1 and 2 toggle with numbers 1 and 2; group 3 is held with number 1.
    fn app() -> ManagedApplication {
        let mut app = ManagedApplication::new("game.exe".into(), None);
        for (name, number, mode) in [
            ("One", 1, GroupMode::Toggle),
            ("Two", 2, GroupMode::Toggle),
            ("Peek", 1, GroupMode::Hold),
        ] {
            let index = app.add_group(name.into());
            app.groups[index].number = number;
            app.groups[index].mode = mode;
        }
        app
    }

    fn overlay(group: Option<u32>, enabled: bool) -> Overlay {
        let mut overlay = Overlay::text("x".into(), "x".into());
        overlay.group = group;
        overlay.enabled = enabled;
        overlay
    }

    fn hotkey(ctrl: bool, key: u16) -> Option<Hotkey> {
        Some(Hotkey {
            ctrl,
            alt: false,
            shift: false,
            key,
        })
    }

    #[test]
    fn ungrouped_overlays_show_and_grouped_ones_wait_for_their_group() {
        let mut app = app();
        let mut toggles = GroupToggles::default();
        assert!(toggles.is_visible(&app, &overlay(None, true)));
        assert!(!toggles.is_visible(&app, &overlay(None, false)));
        assert!(!toggles.is_visible(&app, &overlay(Some(1), true)));
        assert!(toggles.is_visible(&app, &overlay(Some(99), true)));

        // A press toggles only the toggled groups with that number.
        toggles.press(&app, 1);
        assert!(toggles.is_visible(&app, &overlay(Some(1), true)));
        assert!(!toggles.is_visible(&app, &overlay(Some(1), false)));
        assert!(!toggles.is_visible(&app, &overlay(Some(2), true)));
        assert!(!toggles.is_visible(&app, &overlay(Some(3), true)));

        // Toggled groups are independent.
        toggles.press(&app, 2);
        assert!(toggles.is_shown(&app, 1) && toggles.is_shown(&app, 2));
        toggles.press(&app, 1);
        assert!(!toggles.is_shown(&app, 1) && toggles.is_shown(&app, 2));

        // A held group shows only while its number is held.
        toggles.set_held([1].into());
        assert!(toggles.is_shown(&app, 3));
        toggles.set_held(BTreeSet::new());
        assert!(!toggles.is_shown(&app, 3));

        // A disabled group never shows.
        app.groups[1].enabled = false;
        assert!(!toggles.is_shown(&app, 2));

        // Toggles belong to one application.
        let other = ManagedApplication {
            identity: "other.exe".into(),
            ..app.clone()
        };
        app.groups[1].enabled = true;
        assert!(!toggles.is_shown(&other, 2));
    }

    #[test]
    fn a_held_hotkey_presses_once_and_modifiers_must_match_exactly() {
        let hotkeys = [hotkey(false, 0x31), hotkey(true, 0x31), None];
        let down = RefCell::new(BTreeSet::<u16>::new());
        let is_down = |key| down.borrow().contains(&key);
        let mut presses = HotkeyPresses::default();

        assert_eq!(presses.poll(&hotkeys, is_down), HotkeyState::default());
        down.borrow_mut().insert(0x31);
        let state = presses.poll(&hotkeys, is_down);
        assert_eq!((state.pressed, state.held), (vec![1], [1].into()));
        let state = presses.poll(&hotkeys, is_down);
        assert_eq!((state.pressed, state.held), (vec![], [1].into()));
        down.borrow_mut().remove(&0x31);
        assert_eq!(presses.poll(&hotkeys, is_down), HotkeyState::default());

        down.borrow_mut().extend([VK_CONTROL, 0x31]);
        assert_eq!(presses.poll(&hotkeys, is_down).pressed, [2]);

        down.borrow_mut().insert(VK_SHIFT);
        assert_eq!(presses.poll(&hotkeys, is_down), HotkeyState::default());
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
