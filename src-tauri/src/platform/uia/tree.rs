//! Turns whatever the accessibility interface hands back into a [`RawNode`] tree, within
//! limits.
//!
//! The walk knows nothing about COM: it sees a [`TreeSource`], which the real reader
//! implements over UI Automation elements and the tests implement over a hand-built tree.
//! Everything that decides what ends up in a tree lives here: the node and depth limits, the
//! time budget, and the rules about password and editable fields.

use crate::context::page::raw::RawNode;

use super::map;

/// One element's properties exactly as UI Automation reports them, ids still numeric.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct NodeProps {
    pub control_type: i32,
    pub name: String,
    pub localized_type: String,
    pub aria_role: String,
    pub aria_properties: String,
    pub landmark_type: i32,
    pub localized_landmark: String,
    pub heading_level: i32,
    /// Left, top, right, bottom.
    pub rect: [i32; 4],
    pub is_offscreen: bool,
    pub is_password: bool,
    pub has_keyboard_focus: bool,
}

/// The children of a node could not be fetched (the element went away, or the application
/// stopped answering). The walk carries on without them and reports the tree as incomplete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChildrenUnavailable;

pub(crate) trait TreeSource {
    type Node;

    fn props(&mut self, node: &Self::Node) -> NodeProps;

    fn children(&mut self, node: &Self::Node) -> Result<Vec<Self::Node>, ChildrenUnavailable>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limits {
    /// Most nodes a tree may hold, the root included. At least the root is always kept.
    pub max_nodes: u32,
    /// Most levels below the root.
    pub max_depth: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BuiltTree {
    pub root: RawNode,
    pub node_count: u32,
    pub truncated: bool,
    /// How many nodes' children could not be fetched.
    pub failed_fetches: u32,
    /// A password field in the tree holds the keyboard focus.
    pub password_has_focus: bool,
}

struct Frame<N> {
    node: RawNode,
    pending: std::vec::IntoIter<N>,
    depth: u32,
}

/// Walk `root` in document order and build its tree.
///
/// `should_stop` is asked before each node; once it answers yes the walk keeps what it has
/// and marks the tree truncated. An explicit stack is used instead of recursion so a
/// pathologically deep tree cannot overflow the reader thread's stack.
pub(crate) fn build<S: TreeSource>(
    source: &mut S,
    root: S::Node,
    limits: Limits,
    should_stop: &dyn Fn() -> bool,
) -> BuiltTree {
    let mut walk = Walk {
        source,
        limits,
        node_count: 1,
        truncated: false,
        failed_fetches: 0,
        password_has_focus: false,
    };
    let node = walk.read(&root);
    let pending = walk.children_to_visit(&root, &node, 0);
    let mut stack = vec![Frame {
        node,
        pending,
        depth: 0,
    }];
    let mut halted = false;

    loop {
        let top = stack
            .last_mut()
            .expect("the root frame is popped only on return");
        let next = if halted { None } else { top.pending.next() };
        match next {
            Some(child) => {
                if walk.node_count >= walk.limits.max_nodes || should_stop() {
                    walk.truncated = true;
                    halted = true;
                    continue;
                }
                walk.node_count += 1;
                let depth = top.depth + 1;
                let node = walk.read(&child);
                let pending = walk.children_to_visit(&child, &node, depth);
                stack.push(Frame {
                    node,
                    pending,
                    depth,
                });
            }
            None => {
                let finished = stack.pop().expect("checked above");
                match stack.last_mut() {
                    Some(parent) => parent.node.children.push(finished.node),
                    None => {
                        return BuiltTree {
                            root: finished.node,
                            node_count: walk.node_count,
                            truncated: walk.truncated,
                            failed_fetches: walk.failed_fetches,
                            password_has_focus: walk.password_has_focus,
                        }
                    }
                }
            }
        }
    }
}

struct Walk<'a, S: TreeSource> {
    source: &'a mut S,
    limits: Limits,
    node_count: u32,
    truncated: bool,
    failed_fetches: u32,
    password_has_focus: bool,
}

impl<S: TreeSource> Walk<'_, S> {
    fn read(&mut self, element: &S::Node) -> RawNode {
        let props = self.source.props(element);
        self.password_has_focus |= props.is_password && props.has_keyboard_focus;
        to_raw_node(props)
    }

    fn children_to_visit(
        &mut self,
        element: &S::Node,
        node: &RawNode,
        depth: u32,
    ) -> std::vec::IntoIter<S::Node> {
        if !map::may_descend(node.control, node.is_password) {
            return Vec::new().into_iter();
        }
        let children = match self.source.children(element) {
            Ok(children) => children,
            Err(ChildrenUnavailable) => {
                self.failed_fetches += 1;
                self.truncated = true;
                Vec::new()
            }
        };
        if depth >= self.limits.max_depth && !children.is_empty() {
            self.truncated = true;
            return Vec::new().into_iter();
        }
        children.into_iter()
    }
}

fn to_raw_node(props: NodeProps) -> RawNode {
    let [left, top, right, bottom] = props.rect;
    let mut node = RawNode {
        control: map::control_kind(props.control_type),
        name: props.name,
        localized_type: props.localized_type,
        aria_role: props.aria_role,
        aria_properties: props.aria_properties,
        landmark: map::landmark(props.landmark_type),
        localized_landmark: props.localized_landmark,
        heading_level: map::heading_level(props.heading_level),
        rect: map::rect(left, top, right, bottom),
        is_offscreen: props.is_offscreen,
        is_password: props.is_password,
        children: Vec::new(),
    };
    if node.is_password {
        // The real source never reads these for a password field; clearing them here as well
        // means no source can put a password field's text into a tree. The ARIA properties
        // go too because providers may put value-like attributes in them.
        node.name.clear();
        node.aria_properties.clear();
    }
    node
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::context::page::raw::{ControlKind, Landmark, Rect};

    const GROUP: i32 = 50026;
    const TEXT: i32 = 50020;
    const EDIT: i32 = 50004;
    const COMBO_BOX: i32 = 50003;
    const DOCUMENT: i32 = 50030;

    #[derive(Debug, Clone, Default)]
    struct Mock {
        props: NodeProps,
        children: Vec<Mock>,
        children_fail: bool,
    }

    fn node(control_type: i32, name: &str, children: Vec<Mock>) -> Mock {
        Mock {
            props: NodeProps {
                control_type,
                name: name.to_string(),
                ..NodeProps::default()
            },
            children,
            children_fail: false,
        }
    }

    fn text(name: &str) -> Mock {
        node(TEXT, name, Vec::new())
    }

    #[derive(Default)]
    struct MockSource {
        children_calls: u32,
        entered: Vec<String>,
    }

    impl TreeSource for MockSource {
        type Node = Mock;

        fn props(&mut self, node: &Mock) -> NodeProps {
            node.props.clone()
        }

        fn children(&mut self, node: &Mock) -> Result<Vec<Mock>, ChildrenUnavailable> {
            self.children_calls += 1;
            self.entered.push(node.props.name.clone());
            if node.children_fail {
                Err(ChildrenUnavailable)
            } else {
                Ok(node.children.clone())
            }
        }
    }

    const ROOMY: Limits = Limits {
        max_nodes: 1_000,
        max_depth: 50,
    };

    fn build_all(root: Mock, limits: Limits) -> BuiltTree {
        build(&mut MockSource::default(), root, limits, &|| false)
    }

    fn names(node: &RawNode, out: &mut Vec<String>) {
        out.push(node.name.clone());
        node.children.iter().for_each(|child| names(child, out));
    }

    fn depth(node: &RawNode) -> u32 {
        node.children
            .iter()
            .map(|child| 1 + depth(child))
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn a_tree_is_copied_in_document_order() {
        let root = node(
            DOCUMENT,
            "doc",
            vec![
                node(GROUP, "a", vec![text("a1"), text("a2")]),
                text("b"),
                node(GROUP, "c", vec![node(GROUP, "c1", vec![text("c1x")])]),
            ],
        );

        let built = build_all(root, ROOMY);

        let mut seen = Vec::new();
        names(&built.root, &mut seen);
        assert_eq!(seen, ["doc", "a", "a1", "a2", "b", "c", "c1", "c1x"]);
        assert_eq!(built.node_count, 8);
        assert_eq!(built.node_count, built.root.count());
        assert!(!built.truncated);
        assert_eq!(built.failed_fetches, 0);
    }

    #[test]
    fn properties_are_mapped_onto_the_node() {
        let root = Mock {
            props: NodeProps {
                control_type: GROUP,
                name: "Main story".to_string(),
                localized_type: "heading".to_string(),
                aria_role: "heading".to_string(),
                aria_properties: "level=2".to_string(),
                landmark_type: 80002,
                localized_landmark: "main".to_string(),
                heading_level: 80052,
                rect: [1, 2, 31, 42],
                is_offscreen: true,
                is_password: false,
                has_keyboard_focus: false,
            },
            ..Mock::default()
        };

        let built = build_all(root, ROOMY);

        assert_eq!(
            built.root,
            RawNode {
                control: ControlKind::Group,
                name: "Main story".to_string(),
                localized_type: "heading".to_string(),
                aria_role: "heading".to_string(),
                aria_properties: "level=2".to_string(),
                landmark: Landmark::Main,
                localized_landmark: "main".to_string(),
                heading_level: 2,
                rect: Some(Rect {
                    left: 1,
                    top: 2,
                    right: 31,
                    bottom: 42
                }),
                is_offscreen: true,
                is_password: false,
                children: Vec::new(),
            }
        );
    }

    #[test]
    fn the_node_limit_keeps_the_start_of_the_page_and_marks_the_tree_truncated() {
        let paragraphs = (0..100).map(|index| text(&format!("p{index}"))).collect();
        let root = node(
            DOCUMENT,
            "doc",
            vec![node(GROUP, "body", paragraphs), text("footer")],
        );

        let built = build_all(
            root,
            Limits {
                max_nodes: 12,
                max_depth: 50,
            },
        );

        assert!(built.truncated);
        assert_eq!(built.node_count, 12);
        assert_eq!(built.root.count(), 12);
        let mut seen = Vec::new();
        names(&built.root, &mut seen);
        assert_eq!(&seen[..4], ["doc", "body", "p0", "p1"]);
        assert_eq!(seen.last().map(String::as_str), Some("p9"));
    }

    #[test]
    fn a_tree_exactly_at_the_node_limit_is_complete() {
        let root = node(DOCUMENT, "doc", vec![text("a"), text("b")]);

        let built = build_all(
            root,
            Limits {
                max_nodes: 3,
                max_depth: 50,
            },
        );

        assert_eq!(built.node_count, 3);
        assert!(!built.truncated);
    }

    #[test]
    fn the_root_is_kept_even_with_a_node_limit_of_zero() {
        let root = node(DOCUMENT, "doc", vec![text("a")]);

        let built = build_all(
            root,
            Limits {
                max_nodes: 0,
                max_depth: 50,
            },
        );

        assert_eq!(built.node_count, 1);
        assert!(built.root.children.is_empty());
        assert!(built.truncated);
    }

    #[test]
    fn the_depth_limit_cuts_below_it_and_marks_the_tree_truncated() {
        let mut chain = text("leaf");
        for level in (0..10).rev() {
            chain = node(GROUP, &format!("level{level}"), vec![chain]);
        }

        let built = build_all(
            chain.clone(),
            Limits {
                max_nodes: 1_000,
                max_depth: 4,
            },
        );
        assert_eq!(depth(&built.root), 4);
        assert_eq!(built.node_count, 5);
        assert!(built.truncated);

        // Ten groups and a leaf are ten levels below the root: a limit of ten keeps it all.
        let whole = build_all(
            chain,
            Limits {
                max_nodes: 1_000,
                max_depth: 10,
            },
        );
        assert_eq!(depth(&whole.root), 10);
        assert!(!whole.truncated);
    }

    #[test]
    fn a_very_deep_tree_does_not_overflow_the_stack() {
        /// Every node has exactly one child, for ever.
        struct Endless;
        impl TreeSource for Endless {
            type Node = ();
            fn props(&mut self, _node: &()) -> NodeProps {
                NodeProps {
                    control_type: GROUP,
                    ..NodeProps::default()
                }
            }
            fn children(&mut self, _node: &()) -> Result<Vec<()>, ChildrenUnavailable> {
                Ok(vec![()])
            }
        }

        let built = build(
            &mut Endless,
            (),
            Limits {
                max_nodes: 1_000_000,
                max_depth: 100_000,
            },
            &|| false,
        );

        assert_eq!(built.node_count, 100_001);
        assert!(built.truncated);
        // Dropping a tree this deep would recurse; take it apart from the top instead.
        let mut node = built.root;
        while let Some(child) = node.children.pop() {
            node = child;
        }
    }

    #[test]
    fn a_password_field_has_no_name_and_is_not_entered() {
        let mut password = node(EDIT, "hunter2", vec![text("hunter2")]);
        password.props.is_password = true;
        password.props.aria_properties = "valuetext=hunter2".to_string();
        let root = node(
            DOCUMENT,
            "doc",
            vec![text("Sign in"), password, text("after")],
        );
        let mut source = MockSource::default();

        let built = build(&mut source, root, ROOMY, &|| false);

        let field = &built.root.children[1];
        assert!(field.is_password);
        assert_eq!(field.control, ControlKind::Edit);
        assert!(field.name.is_empty());
        assert!(field.aria_properties.is_empty());
        assert!(field.children.is_empty());
        assert_eq!(built.node_count, 4);
        assert!(!built.truncated);
        // The password field's children were never even asked for.
        assert_eq!(source.entered, ["doc", "Sign in", "after"]);
        assert!(!format!("{built:?}").contains("hunter2"));
    }

    #[test]
    fn keyboard_focus_in_a_password_field_is_noticed() {
        let page = |password_focused: bool, other_focused: bool| {
            let mut password = node(EDIT, "", Vec::new());
            password.props.is_password = true;
            password.props.has_keyboard_focus = password_focused;
            let mut other = node(EDIT, "Email", Vec::new());
            other.props.has_keyboard_focus = other_focused;
            node(
                DOCUMENT,
                "doc",
                vec![node(GROUP, "form", vec![other, password])],
            )
        };

        assert!(build_all(page(true, false), ROOMY).password_has_focus);
        assert!(!build_all(page(false, true), ROOMY).password_has_focus);
        assert!(!build_all(page(false, false), ROOMY).password_has_focus);
    }

    #[test]
    fn a_password_container_of_any_kind_is_not_entered() {
        let mut wrapper = node(GROUP, "secret label", vec![text("secret")]);
        wrapper.props.is_password = true;
        let root = node(DOCUMENT, "doc", vec![wrapper]);

        let built = build_all(root, ROOMY);

        assert!(!format!("{built:?}").contains("secret"));
        assert_eq!(built.node_count, 2);
    }

    #[test]
    fn editable_fields_keep_their_label_but_their_contents_are_not_read() {
        let root = node(
            DOCUMENT,
            "doc",
            vec![
                node(EDIT, "Email", vec![text("someone@example.com")]),
                node(COMBO_BOX, "Country", vec![text("Iceland")]),
            ],
        );
        let mut source = MockSource::default();

        let built = build(&mut source, root, ROOMY, &|| false);

        assert_eq!(built.root.children[0].name, "Email");
        assert_eq!(built.root.children[1].name, "Country");
        assert!(built
            .root
            .children
            .iter()
            .all(|field| field.children.is_empty()));
        assert_eq!(built.node_count, 3);
        assert!(!built.truncated);
        assert_eq!(source.children_calls, 1);
        let shown = format!("{built:?}");
        assert!(!shown.contains("example.com") && !shown.contains("Iceland"));
    }

    #[test]
    fn a_failed_fetch_loses_only_that_subtree() {
        let mut broken = node(GROUP, "broken", vec![text("lost")]);
        broken.children_fail = true;
        let root = node(DOCUMENT, "doc", vec![text("before"), broken, text("after")]);

        let built = build_all(root, ROOMY);

        let mut seen = Vec::new();
        names(&built.root, &mut seen);
        assert_eq!(seen, ["doc", "before", "broken", "after"]);
        assert!(built.truncated);
        assert_eq!(built.failed_fetches, 1);
    }

    #[test]
    fn running_out_of_time_keeps_what_was_read() {
        let paragraphs = (0..50).map(|index| text(&format!("p{index}"))).collect();
        let root = node(DOCUMENT, "doc", paragraphs);
        let asked = Cell::new(0u32);
        let out_of_time = || {
            asked.set(asked.get() + 1);
            asked.get() > 5
        };

        let built = build(&mut MockSource::default(), root, ROOMY, &out_of_time);

        assert_eq!(built.node_count, 6);
        assert_eq!(built.root.children.len(), 5);
        assert!(built.truncated);
        // Once stopped, the walk does not keep asking.
        assert_eq!(asked.get(), 6);
    }
}
