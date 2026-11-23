//! UI Automation's numeric ids turned into the plain enums of `context::page::raw`.
//!
//! The ids are written out here instead of taken from the `windows` crate so the mappings can
//! be tested, and read, without any COM types in sight. They are part of the documented UI
//! Automation contract and do not change between Windows versions.

use crate::context::page::raw::{ControlKind, Landmark, Rect};

pub(crate) const CONTROL_TYPE_DOCUMENT: i32 = 50030;

const HEADING_LEVEL_1: i32 = 80051;
const HEADING_LEVEL_9: i32 = 80059;

pub(crate) fn control_kind(control_type: i32) -> ControlKind {
    match control_type {
        50000 => ControlKind::Button,
        50002 => ControlKind::CheckBox,
        50003 => ControlKind::ComboBox,
        50004 => ControlKind::Edit,
        50005 => ControlKind::Hyperlink,
        50006 => ControlKind::Image,
        50007 => ControlKind::ListItem,
        50008 => ControlKind::List,
        50009 => ControlKind::Menu,
        50010 => ControlKind::MenuBar,
        50011 => ControlKind::MenuItem,
        50013 => ControlKind::RadioButton,
        50014 => ControlKind::ScrollBar,
        50017 => ControlKind::StatusBar,
        50018 => ControlKind::Tab,
        50019 => ControlKind::TabItem,
        50020 => ControlKind::Text,
        50021 => ControlKind::ToolBar,
        50023 => ControlKind::Tree,
        50024 => ControlKind::TreeItem,
        50026 => ControlKind::Group,
        // A data grid is a table whose cells can be selected; for reading it is a table.
        50028 | 50036 => ControlKind::Table,
        50029 => ControlKind::DataItem,
        CONTROL_TYPE_DOCUMENT => ControlKind::Document,
        // A split button is a button with a drop-down attached.
        50031 => ControlKind::Button,
        50032 => ControlKind::Window,
        50033 => ControlKind::Pane,
        50034 => ControlKind::Header,
        50035 => ControlKind::HeaderItem,
        50037 => ControlKind::TitleBar,
        50038 => ControlKind::Separator,
        _ => ControlKind::Other,
    }
}

pub(crate) fn landmark(landmark_type: i32) -> Landmark {
    match landmark_type {
        80000 => Landmark::Custom,
        80001 => Landmark::Form,
        80002 => Landmark::Main,
        80003 => Landmark::Navigation,
        80004 => Landmark::Search,
        _ => Landmark::None,
    }
}

/// 1 to 9 for a heading, 0 for anything else (including `UIA_HeadingLevel_None`, 80050).
pub(crate) fn heading_level(heading_level_id: i32) -> u8 {
    if (HEADING_LEVEL_1..=HEADING_LEVEL_9).contains(&heading_level_id) {
        (heading_level_id - HEADING_LEVEL_1 + 1) as u8
    } else {
        0
    }
}

/// Providers report an element without a place on screen as an all-zero or inverted
/// rectangle; that is "unknown", not a rectangle at the origin.
pub(crate) fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Option<Rect> {
    let rect = Rect {
        left,
        top,
        right,
        bottom,
    };
    (!rect.is_empty()).then_some(rect)
}

/// Whether the reader may look inside a node.
///
/// A password field is never entered. Neither is an editable field or a combo box: what lies
/// under them is the text the user typed or the current choice, which is the control's value
/// in all but name.
pub(crate) fn may_descend(kind: ControlKind, is_password: bool) -> bool {
    !is_password && !matches!(kind, ControlKind::Edit | ControlKind::ComboBox)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_control_kind_with_an_id_is_mapped() {
        let expected = [
            (50000, ControlKind::Button),
            (50002, ControlKind::CheckBox),
            (50003, ControlKind::ComboBox),
            (50004, ControlKind::Edit),
            (50005, ControlKind::Hyperlink),
            (50006, ControlKind::Image),
            (50007, ControlKind::ListItem),
            (50008, ControlKind::List),
            (50009, ControlKind::Menu),
            (50010, ControlKind::MenuBar),
            (50011, ControlKind::MenuItem),
            (50013, ControlKind::RadioButton),
            (50014, ControlKind::ScrollBar),
            (50017, ControlKind::StatusBar),
            (50018, ControlKind::Tab),
            (50019, ControlKind::TabItem),
            (50020, ControlKind::Text),
            (50021, ControlKind::ToolBar),
            (50023, ControlKind::Tree),
            (50024, ControlKind::TreeItem),
            (50026, ControlKind::Group),
            (50028, ControlKind::Table),
            (50029, ControlKind::DataItem),
            (50030, ControlKind::Document),
            (50031, ControlKind::Button),
            (50032, ControlKind::Window),
            (50033, ControlKind::Pane),
            (50034, ControlKind::Header),
            (50035, ControlKind::HeaderItem),
            (50036, ControlKind::Table),
            (50037, ControlKind::TitleBar),
            (50038, ControlKind::Separator),
        ];

        for (id, kind) in expected {
            assert_eq!(control_kind(id), kind, "control type {id}");
        }
    }

    #[test]
    fn control_types_without_a_kind_are_other() {
        // Calendar, progress bar, slider, spinner, tooltip, custom, thumb, semantic zoom, app
        // bar, an id past the end of the table, and no id at all.
        for id in [
            50001, 50012, 50015, 50016, 50022, 50025, 50027, 50039, 50040, 50999, 0, -1,
        ] {
            assert_eq!(control_kind(id), ControlKind::Other, "control type {id}");
        }
    }

    #[test]
    fn landmark_ids_map_to_landmarks() {
        assert_eq!(landmark(80000), Landmark::Custom);
        assert_eq!(landmark(80001), Landmark::Form);
        assert_eq!(landmark(80002), Landmark::Main);
        assert_eq!(landmark(80003), Landmark::Navigation);
        assert_eq!(landmark(80004), Landmark::Search);
        assert_eq!(landmark(0), Landmark::None);
        assert_eq!(landmark(80005), Landmark::None);
    }

    #[test]
    fn heading_ids_map_to_levels_one_to_nine() {
        for level in 1..=9u8 {
            assert_eq!(heading_level(80050 + i32::from(level)), level);
        }
        assert_eq!(heading_level(80050), 0);
        assert_eq!(heading_level(80060), 0);
        assert_eq!(heading_level(0), 0);
    }

    #[test]
    fn a_rectangle_without_area_is_unknown() {
        assert_eq!(
            rect(10, 20, 110, 70),
            Some(Rect {
                left: 10,
                top: 20,
                right: 110,
                bottom: 70
            })
        );
        assert_eq!(rect(0, 0, 0, 0), None);
        assert_eq!(rect(10, 20, 10, 70), None);
        assert_eq!(rect(10, 20, 5, 70), None);
        // Off the left edge of the screen is still a real rectangle.
        assert!(rect(-500, -40, -100, -10).is_some());
    }

    #[test]
    fn fields_that_hold_what_the_user_typed_are_not_entered() {
        assert!(may_descend(ControlKind::Group, false));
        assert!(may_descend(ControlKind::Document, false));
        assert!(!may_descend(ControlKind::Edit, false));
        assert!(!may_descend(ControlKind::ComboBox, false));
        assert!(!may_descend(ControlKind::Group, true));
        assert!(!may_descend(ControlKind::Edit, true));
    }
}
