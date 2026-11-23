//! A window's accessibility tree as plain data.
//!
//! This is the hand-off between the platform code that reads Windows UI Automation
//! (`platform/uia`) and the pure code that turns a tree into something to read aloud
//! (`context/page`). It holds no COM types, so it can cross threads, be built by hand in
//! tests, and be saved as a JSON fixture.
//!
//! Privacy: a tree holds text from another application's window. It lives in memory for one
//! request and must never be logged or stored. The reader that builds it never records the
//! value of an editable field, and never descends into a password field.

use serde::{Deserialize, Serialize};

/// Everything read from one window in one go.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RawTree {
    pub window: WindowInfo,
    /// The content root: the window's document element when it has one (a web page, a PDF, a
    /// document editor), otherwise the window itself. `None` when nothing could be read.
    pub root: Option<RawNode>,
    /// The keyboard focus was in a password field when the tree was read.
    pub focused_is_password: bool,
    /// The node or depth limit was reached, so the tree is incomplete.
    pub truncated: bool,
    /// Nodes in the tree.
    pub node_count: u32,
    /// How long the read took, for diagnostics.
    pub elapsed_ms: u64,
}

/// Facts about the window the tree came from. Used to decide whether reading it is allowed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowInfo {
    /// File name of the owning program in lower case, for example `msedge.exe`. Empty when it
    /// could not be determined.
    pub process_name: String,
    pub title: String,
    /// Win32 window class, for example `Chrome_WidgetWin_1`.
    pub class_name: String,
    /// Address shown by a browser's document, when the document exposes one. May be empty.
    pub url: String,
}

/// A screen rectangle in physical pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn width(&self) -> i32 {
        (self.right - self.left).max(0)
    }

    pub fn height(&self) -> i32 {
        (self.bottom - self.top).max(0)
    }

    pub fn is_empty(&self) -> bool {
        self.width() == 0 || self.height() == 0
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
    }
}

/// The UI Automation control type of a node, reduced to the kinds that matter for reading.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlKind {
    Document,
    /// A run of text.
    Text,
    Hyperlink,
    /// A generic container: `div`, `section`, `article`, `main`, `nav`, `figure` and so on.
    Group,
    List,
    ListItem,
    Table,
    /// A table row or cell.
    DataItem,
    Header,
    HeaderItem,
    Image,
    /// An editable text field. Its value is never recorded.
    Edit,
    Button,
    ComboBox,
    CheckBox,
    RadioButton,
    Menu,
    MenuBar,
    MenuItem,
    ToolBar,
    Tab,
    TabItem,
    Tree,
    TreeItem,
    Pane,
    Window,
    TitleBar,
    ScrollBar,
    StatusBar,
    Separator,
    /// Anything else.
    #[default]
    Other,
}

/// The landmark role of a node (`main`, `nav`, ...), from UI Automation's landmark type.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Landmark {
    #[default]
    None,
    Main,
    Navigation,
    Search,
    Form,
    /// A landmark UI Automation has no code for (banner, complementary, contentinfo,
    /// region). `localized_landmark` on the node says which.
    Custom,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RawNode {
    pub control: ControlKind,
    /// The accessible name: the text of a text run, a link's label, an image's description,
    /// a heading's text. Empty for an editable field's contents, which are never read.
    pub name: String,
    /// Localized control type, for example `heading`, `article`, `figure`.
    pub localized_type: String,
    /// The ARIA role when the provider reports one, for example `heading`, `main`,
    /// `navigation`, `banner`, `dialog`, `alert`.
    pub aria_role: String,
    /// The provider's ARIA properties string, `key=value;key=value`.
    pub aria_properties: String,
    pub landmark: Landmark,
    /// Localized landmark name for [`Landmark::Custom`], for example `banner`.
    pub localized_landmark: String,
    /// Heading level 1 to 9, or 0 when the node is not a heading.
    pub heading_level: u8,
    /// Where the node is on screen, when known.
    pub rect: Option<Rect>,
    /// The node is scrolled out of view or otherwise not visible.
    pub is_offscreen: bool,
    /// The node is a password field. Readers stop here: such a node has no name and no
    /// children in a tree.
    pub is_password: bool,
    pub children: Vec<RawNode>,
}

impl RawNode {
    /// A text run, for building trees in tests.
    pub fn text(name: &str) -> Self {
        Self {
            control: ControlKind::Text,
            name: name.to_string(),
            ..Self::default()
        }
    }

    /// A container of `control` kind with `children`, for building trees in tests.
    pub fn container(control: ControlKind, children: Vec<RawNode>) -> Self {
        Self {
            control,
            children,
            ..Self::default()
        }
    }

    /// Nodes in this subtree, including this one.
    pub fn count(&self) -> u32 {
        1 + self.children.iter().map(RawNode::count).sum::<u32>()
    }
}
