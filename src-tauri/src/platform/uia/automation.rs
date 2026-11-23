//! The UI Automation calls themselves. Everything here runs on the reader's worker thread.
//!
//! Reading a tree one property at a time costs a cross-process call per property per
//! element. A cache request instead names the properties up front and gets them, for a whole
//! subtree, in one call. That is how a page is read here. Should that call be refused, the
//! tree is walked with one call per parent instead: many times slower, but it reads the top
//! of the page in the time allowed. A page so large that the one call times out is reported
//! as a timeout; see `read_window` for why walking cannot rescue it.
//!
//! Privacy: no text read from a window is logged or put in an error. Log lines carry counts,
//! timings and HRESULT codes only. The `Value` of a control is read in exactly one place:
//! the content document's, which is where browsers expose the page address.

use std::{
    thread,
    time::{Duration, Instant},
};

use windows::{
    core::{Interface, BSTR},
    Win32::{
        Foundation::{E_ACCESSDENIED, HWND},
        System::{
            Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
                COINIT_MULTITHREADED,
            },
            Variant::{VARIANT, VT_I4},
        },
        UI::Accessibility::{
            AutomationElementMode, AutomationElementMode_Full, AutomationElementMode_None,
            CUIAutomation, CUIAutomation8, IUIAutomation, IUIAutomation2,
            IUIAutomationCacheRequest, IUIAutomationCondition, IUIAutomationElement,
            IUIAutomationElement5, IUIAutomationElement8, IUIAutomationElementArray,
            IUIAutomationValuePattern, TreeScope, TreeScope_Children, TreeScope_Descendants,
            TreeScope_Element, TreeScope_Subtree, UIA_AriaPropertiesPropertyId,
            UIA_AriaRolePropertyId, UIA_BoundingRectanglePropertyId, UIA_ControlTypePropertyId,
            UIA_HasKeyboardFocusPropertyId, UIA_HeadingLevelPropertyId, UIA_IsOffscreenPropertyId,
            UIA_IsPasswordPropertyId, UIA_LandmarkTypePropertyId,
            UIA_LocalizedControlTypePropertyId, UIA_LocalizedLandmarkTypePropertyId,
            UIA_NamePropertyId, UIA_ValuePatternId, UIA_ValueValuePropertyId,
            UIA_E_ELEMENTNOTAVAILABLE, UIA_E_TIMEOUT, UIA_PROPERTY_ID,
        },
    },
};

use crate::context::page::{raw::RawTree, types::PageError};

use super::{
    map,
    tree::{self, ChildrenUnavailable, Limits, NodeProps, TreeSource},
    window,
    worker::Cancel,
    SnapshotOptions,
};

/// How long UI Automation waits for an application that is not answering before the call
/// fails. The defaults are 20 seconds and more, far longer than anyone waits to be read to.
const PROVIDER_TIMEOUT_MS: u32 = 1_500;
/// Pause between looks at a browser window whose page is not in the tree yet.
const DOCUMENT_POLL: Duration = Duration::from_millis(40);
/// Share of the caller's deadline the walk may use. The rest is left for handing the tree
/// back, so a slow page comes back cut short instead of not at all.
const WALK_BUDGET: f32 = 0.85;
/// Elements examined when looking for documents level by level. A browser's own interface
/// (tabs, toolbar, side panels) is a few hundred elements; pages are not entered.
const SHALLOW_SEARCH_ELEMENTS: usize = 600;
/// Share of the remaining time the one-call fetch of a whole page may take.
const SUBTREE_FETCH_SHARE: f32 = 0.75;

/// Properties every node of a tree needs.
const NODE_PROPERTIES: [UIA_PROPERTY_ID; 8] = [
    UIA_ControlTypePropertyId,
    UIA_NamePropertyId,
    UIA_LocalizedControlTypePropertyId,
    UIA_AriaRolePropertyId,
    UIA_AriaPropertiesPropertyId,
    UIA_BoundingRectanglePropertyId,
    UIA_IsOffscreenPropertyId,
    UIA_HasKeyboardFocusPropertyId,
];
/// Added in later Windows 10 releases. A system without them simply reports no landmarks
/// and no heading levels.
const NEWER_NODE_PROPERTIES: [UIA_PROPERTY_ID; 3] = [
    UIA_LandmarkTypePropertyId,
    UIA_LocalizedLandmarkTypePropertyId,
    UIA_HeadingLevelPropertyId,
];

/// A UI Automation client, bound to the thread that created it.
pub(crate) struct Session {
    automation: IUIAutomation,
    /// The same client, when it supports timeouts.
    timeouts: Option<IUIAutomation2>,
    everything: IUIAutomationCondition,
    documents: IUIAutomationCondition,
    /// A whole subtree, properties only: the elements cannot be used for further calls.
    subtree_request: IUIAutomationCacheRequest,
    /// An element and its children, usable for further calls.
    step_request: IUIAutomationCacheRequest,
    /// What is needed to choose between documents, and the address of the chosen one.
    document_request: IUIAutomationCacheRequest,
    focus_request: IUIAutomationCacheRequest,
    // Declared last: fields drop in order, and COM must outlive every interface above.
    _com: ComApartment,
}

struct ComApartment;

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

type Com<T> = windows::core::Result<T>;

impl Session {
    pub fn open() -> Result<Self, PageError> {
        // Multithreaded on purpose: this thread owns no windows and runs no message loop,
        // which a single-threaded apartment would need to receive answers.
        if unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_err() {
            return Err(PageError::Unavailable(
                "COM could not be started for reading windows.".to_string(),
            ));
        }
        let com = ComApartment;
        Self::create(com).map_err(|error| {
            tracing::warn!(code = error.code().0, "UI Automation could not be started");
            PageError::Unavailable("UI Automation is not available on this system.".to_string())
        })
    }

    fn create(com: ComApartment) -> Com<Self> {
        unsafe {
            // CUIAutomation8 is the client that knows about timeouts and the newer
            // properties; the original class is the fallback on a system without it.
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)
                    .or_else(|_| CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER))?;
            let timeouts = automation.cast::<IUIAutomation2>().ok();
            if let Some(timeouts) = &timeouts {
                let _ = timeouts.SetConnectionTimeout(PROVIDER_TIMEOUT_MS);
                let _ = timeouts.SetTransactionTimeout(PROVIDER_TIMEOUT_MS);
            }

            let mut document_type = VARIANT::default();
            {
                let fields = &mut *document_type.Anonymous.Anonymous;
                fields.vt = VT_I4;
                fields.Anonymous.lVal = map::CONTROL_TYPE_DOCUMENT;
            }
            let documents =
                automation.CreatePropertyCondition(UIA_ControlTypePropertyId, &document_type)?;

            let subtree_request =
                node_request(&automation, TreeScope_Subtree, AutomationElementMode_None)?;
            let step_request = node_request(
                &automation,
                TreeScope(TreeScope_Element.0 | TreeScope_Children.0),
                AutomationElementMode_Full,
            )?;

            let document_request = automation.CreateCacheRequest()?;
            for property in [
                UIA_ControlTypePropertyId,
                UIA_BoundingRectanglePropertyId,
                UIA_IsOffscreenPropertyId,
                UIA_ValueValuePropertyId,
            ] {
                document_request.AddProperty(property)?;
            }
            document_request.AddPattern(UIA_ValuePatternId)?;

            let focus_request = automation.CreateCacheRequest()?;
            focus_request.AddProperty(UIA_IsPasswordPropertyId)?;
            focus_request.SetAutomationElementMode(AutomationElementMode_None)?;

            Ok(Self {
                everything: automation.CreateTrueCondition()?,
                documents,
                subtree_request,
                step_request,
                document_request,
                focus_request,
                automation,
                timeouts,
                _com: com,
            })
        }
    }

    /// Read the window. `started` is when the caller began waiting; the walk stops, keeping
    /// what it has, once most of `options.deadline` has gone or the caller has given up.
    pub fn read_window(
        &self,
        hwnd: isize,
        options: &SnapshotOptions,
        started: Instant,
        cancel: &Cancel,
    ) -> Result<RawTree, PageError> {
        let hwnd = window::to_hwnd(hwnd);
        let mut info = window::describe(hwnd).ok_or(PageError::TargetChanged)?;

        if window::process_id(hwnd).is_some_and(window::is_own_process) {
            // Our own windows answer UI Automation on the application's UI thread, which may
            // well be the thread waiting for this read. They are never read aloud anyway.
            return Ok(RawTree {
                window: info,
                elapsed_ms: started.elapsed().as_millis() as u64,
                ..RawTree::default()
            });
        }

        let window_element = unsafe {
            self.automation
                .ElementFromHandleBuildCache(hwnd, &self.step_request)
        }
        .map_err(|error| read_failure(&error, hwnd))?;

        let lazy = window::builds_content_lazily(&info.class_name);
        let search_started = Instant::now();
        let wait_until = search_started + options.document_wait.min(options.deadline.mul_f32(0.5));
        let mut looks = 0u32;
        let document = loop {
            looks += 1;
            let found = self.find_document(&window_element, lazy);
            let ready = matches!(&found, Some(document) if self.has_children(&document.element));
            if ready || !lazy || Instant::now() >= wait_until || cancel.is_cancelled() {
                break found.map(|document| (document, ready));
            }
            thread::sleep(DOCUMENT_POLL);
        };
        let document_wait_ms = search_started.elapsed().as_millis() as u64;

        let document_found = document.is_some();
        let content = match document {
            Some((document, true)) => {
                info.url = document.url;
                document.element
            }
            // A browser page that stayed empty, or has not appeared yet. Falling back to the
            // window would read the browser's tabs and toolbar aloud, which is worse than
            // reading nothing. A page still being built is usually there on the next try.
            Some((_, false)) if lazy => return Err(PageError::NothingReadable),
            None if lazy => return Err(PageError::Timeout),
            // An editor whose document element exposes its text only through a text
            // pattern, or no document at all: the window itself is what there is.
            _ => window_element,
        };

        let budget = started + options.deadline.mul_f32(WALK_BUDGET);
        let should_stop = || cancel.is_cancelled() || Instant::now() >= budget;
        let limits = Limits {
            max_nodes: options.max_nodes,
            max_depth: options.max_depth,
        };

        // Fetching a long page in one call is one long transaction, so for this call alone
        // the transaction timeout is stretched to most of the time that is left. What it
        // leaves is for copying the tree out of the cache.
        let fetch_started = Instant::now();
        let remaining = budget.saturating_duration_since(fetch_started);
        let timeout_ms =
            (remaining.mul_f32(SUBTREE_FETCH_SHARE).as_millis() as u32).max(PROVIDER_TIMEOUT_MS);
        self.set_transaction_timeout(timeout_ms);
        let fetched = unsafe { content.BuildUpdatedCache(&self.subtree_request) };
        self.set_transaction_timeout(PROVIDER_TIMEOUT_MS);
        let (root, whole_subtree) = match fetched {
            Ok(root) => (root, true),
            Err(error) if error.code().0 as u32 == UIA_E_TIMEOUT => {
                // The application is still busy assembling the answer nobody is waiting for
                // any more, and is deaf to further calls until it is done: walking the page
                // now would only time out again, call after call.
                tracing::debug!(
                    waited_ms = fetch_started.elapsed().as_millis() as u64,
                    "reading the tree in one call timed out"
                );
                return Err(if window::exists(hwnd) {
                    PageError::Timeout
                } else {
                    PageError::TargetChanged
                });
            }
            Err(error) => {
                tracing::debug!(
                    code = error.code().0,
                    "reading the tree in one call failed; walking it instead"
                );
                let root = unsafe { content.BuildUpdatedCache(&self.step_request) }
                    .map_err(|error| read_failure(&error, hwnd))?;
                (root, false)
            }
        };
        let fetch_ms = fetch_started.elapsed().as_millis() as u64;

        let walk_started = Instant::now();
        let mut source = ElementSource {
            session: self,
            whole_subtree,
        };
        let built = tree::build(
            &mut source,
            Element {
                element: root,
                children_cached: true,
            },
            limits,
            &should_stop,
        );
        let walk_ms = walk_started.elapsed().as_millis() as u64;

        // Asked only now, and backed by what the walk saw. A browser that has not built its
        // page tree yet reports the focus as being on the document, whatever field it is
        // really in; by this point the tree exists, so the answer can be trusted.
        let focused_is_password = built.password_has_focus || self.focused_is_password();

        if !window::exists(hwnd) {
            return Err(PageError::TargetChanged);
        }
        if built.root.children.is_empty() {
            // A root on its own is a window title at most. Which error depends on why there
            // is nothing under it; a tree cut down to its root by the caller's own limits is
            // still handed back, marked truncated.
            if built.failed_fetches > 0 {
                return Err(PageError::Timeout);
            }
            if !built.truncated {
                return Err(PageError::NothingReadable);
            }
        }

        let elapsed_ms = started.elapsed().as_millis() as u64;
        tracing::debug!(
            nodes = built.node_count,
            truncated = built.truncated,
            failed_fetches = built.failed_fetches,
            document_found,
            document_looks = looks,
            document_wait_ms,
            whole_subtree,
            fetch_ms,
            walk_ms,
            elapsed_ms,
            "window read"
        );
        Ok(RawTree {
            window: info,
            root: Some(built.root),
            focused_is_password,
            truncated: built.truncated,
            node_count: built.node_count,
            elapsed_ms,
        })
    }

    fn set_transaction_timeout(&self, milliseconds: u32) {
        if let Some(timeouts) = &self.timeouts {
            let _ = unsafe { timeouts.SetTransactionTimeout(milliseconds) };
        }
    }

    /// Whether the keyboard focus, wherever it is, sits in a password field. An unknown
    /// focus (nothing focused, or the focused application is not answering) counts as no.
    fn focused_is_password(&self) -> bool {
        unsafe {
            self.automation
                .GetFocusedElementBuildCache(&self.focus_request)
                .and_then(|focused| focused.CachedIsPassword())
                .map(|is_password| is_password.as_bool())
                .unwrap_or(false)
        }
    }

    /// The document that holds the window's content, if the window has one.
    fn find_document(&self, window_element: &IUIAutomationElement, lazy: bool) -> Option<Document> {
        if is_document(window_element) {
            // The step request did not cache what a candidate needs; fetch it.
            let element =
                unsafe { window_element.BuildUpdatedCache(&self.document_request) }.ok()?;
            return Some(Document::read(element));
        }

        // A search through every descendant is a single call, but it goes through the whole
        // page too: a third of a second on a very long page, again on every look while
        // waiting for the page to appear. A browser's own interface is small, so there the
        // search goes level by level and stops at documents; its cost does not grow with
        // the page. Other applications can have a large interface and get the single call.
        let mut candidates = Vec::new();
        if lazy {
            candidates = self.find_documents_shallow(window_element);
        }
        if candidates.is_empty() {
            candidates = elements(unsafe {
                window_element.FindAllBuildCache(
                    TreeScope_Descendants,
                    &self.documents,
                    &self.document_request,
                )
            })
            .unwrap_or_default();
        }
        let facts: Vec<DocumentFacts> = candidates.iter().map(document_facts).collect();
        let chosen = choose_document(&facts)?;
        candidates.into_iter().nth(chosen).map(Document::read)
    }

    /// Documents reachable from the window without passing through another document.
    fn find_documents_shallow(
        &self,
        window_element: &IUIAutomationElement,
    ) -> Vec<IUIAutomationElement> {
        let mut documents = Vec::new();
        let mut level = vec![window_element.clone()];
        let mut examined = 0usize;
        while !level.is_empty() && examined < SHALLOW_SEARCH_ELEMENTS {
            let mut next = Vec::new();
            for parent in &level {
                let children = elements(unsafe {
                    parent.FindAllBuildCache(
                        TreeScope_Children,
                        &self.everything,
                        &self.document_request,
                    )
                })
                .unwrap_or_default();
                for child in children {
                    examined += 1;
                    if is_document(&child) {
                        documents.push(child);
                    } else {
                        next.push(child);
                    }
                }
                if examined >= SHALLOW_SEARCH_ELEMENTS {
                    break;
                }
            }
            level = next;
        }
        documents
    }

    fn has_children(&self, element: &IUIAutomationElement) -> bool {
        // A null result with a success code means "none found".
        unsafe { element.FindFirst(TreeScope_Children, &self.everything) }.is_ok()
    }
}

/// `element` must have its control type cached.
fn is_document(element: &IUIAutomationElement) -> bool {
    unsafe { element.CachedControlType() }.is_ok_and(|kind| kind.0 == map::CONTROL_TYPE_DOCUMENT)
}

fn node_request(
    automation: &IUIAutomation,
    scope: TreeScope,
    mode: AutomationElementMode,
) -> Com<IUIAutomationCacheRequest> {
    unsafe {
        let request = automation.CreateCacheRequest()?;
        for property in NODE_PROPERTIES {
            request.AddProperty(property)?;
        }
        request.AddProperty(UIA_IsPasswordPropertyId)?;
        for property in NEWER_NODE_PROPERTIES {
            let _ = request.AddProperty(property);
        }
        // The default (control) view leaves out elements a provider marks as not being
        // controls, and Chromium marks paragraphs so: their text then hangs directly under
        // the section and paragraph boundaries are lost. The raw view keeps them.
        request.SetTreeFilter(&automation.RawViewCondition()?)?;
        request.SetTreeScope(scope)?;
        request.SetAutomationElementMode(mode)?;
        Ok(request)
    }
}

/// The elements of an array result. UI Automation returns a null array, which the bindings
/// report as an error carrying a success code, when there are none.
fn elements(array: Com<IUIAutomationElementArray>) -> Com<Vec<IUIAutomationElement>> {
    let array = match array {
        Ok(array) => array,
        Err(error) if error.code().is_ok() => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    unsafe {
        let length = array.Length()?;
        let mut elements = Vec::with_capacity(length.max(0) as usize);
        for index in 0..length {
            elements.push(array.GetElement(index)?);
        }
        Ok(elements)
    }
}

fn text(value: Com<BSTR>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

/// Turn a failed call on the window into the caller's error. Only the HRESULT is kept.
fn read_failure(error: &windows::core::Error, hwnd: HWND) -> PageError {
    let code = error.code();
    if !window::exists(hwnd) || code.0 as u32 == UIA_E_ELEMENTNOTAVAILABLE {
        PageError::TargetChanged
    } else if code.0 as u32 == UIA_E_TIMEOUT {
        PageError::Timeout
    } else if code == E_ACCESSDENIED {
        // A window of a program running with higher privileges refuses a normal reader.
        PageError::NothingReadable
    } else {
        PageError::Unavailable(format!(
            "UI Automation could not read the window (error 0x{:08X}).",
            code.0
        ))
    }
}

struct Document {
    element: IUIAutomationElement,
    url: String,
}

impl Document {
    /// `element` must have been fetched with the document request.
    fn read(element: IUIAutomationElement) -> Self {
        let url = unsafe {
            element
                .GetCachedPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                .and_then(|value| value.CachedValue())
        };
        Self {
            url: text(url),
            element,
        }
    }
}

/// What is known about a document element when choosing between several.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DocumentFacts {
    pub area: i64,
    pub is_offscreen: bool,
}

fn document_facts(element: &IUIAutomationElement) -> DocumentFacts {
    unsafe {
        let area = element
            .CachedBoundingRectangle()
            .ok()
            .and_then(|rect| map::rect(rect.left, rect.top, rect.right, rect.bottom))
            .map(|rect| i64::from(rect.width()) * i64::from(rect.height()))
            .unwrap_or(0);
        DocumentFacts {
            area,
            is_offscreen: element
                .CachedIsOffscreen()
                .is_ok_and(|offscreen| offscreen.as_bool()),
        }
    }
}

/// Which of a window's documents holds its content.
///
/// A window can have several: frames inside a page, a side panel, developer tools. The page
/// is the largest one on screen. Frames come after the page that contains them and are no
/// larger than it, so on a tie the earliest wins.
pub(crate) fn choose_document(candidates: &[DocumentFacts]) -> Option<usize> {
    let mut best: Option<(usize, (bool, i64))> = None;
    for (index, candidate) in candidates.iter().enumerate() {
        let score = (!candidate.is_offscreen, candidate.area);
        if best.is_none_or(|(_, best_score)| score > best_score) {
            best = Some((index, score));
        }
    }
    best.map(|(index, _)| index)
}

struct Element {
    element: IUIAutomationElement,
    /// The element was fetched together with its children, so listing them costs nothing.
    children_cached: bool,
}

struct ElementSource<'a> {
    session: &'a Session,
    /// The whole tree is already in the cache; no element needs a further call.
    whole_subtree: bool,
}

impl TreeSource for ElementSource<'_> {
    type Node = Element;

    fn props(&mut self, node: &Element) -> NodeProps {
        let element = &node.element;
        unsafe {
            let is_password = element
                .CachedIsPassword()
                .is_ok_and(|value| value.as_bool());
            let mut props = NodeProps {
                control_type: element.CachedControlType().map(|kind| kind.0).unwrap_or(0),
                is_offscreen: element
                    .CachedIsOffscreen()
                    .is_ok_and(|value| value.as_bool()),
                is_password,
                has_keyboard_focus: element
                    .CachedHasKeyboardFocus()
                    .is_ok_and(|value| value.as_bool()),
                rect: element
                    .CachedBoundingRectangle()
                    .map(|rect| [rect.left, rect.top, rect.right, rect.bottom])
                    .unwrap_or_default(),
                ..NodeProps::default()
            };
            if is_password {
                // Nothing textual is taken from a password field, not even into memory here.
                return props;
            }
            props.name = text(element.CachedName());
            props.localized_type = text(element.CachedLocalizedControlType());
            props.aria_role = text(element.CachedAriaRole());
            props.aria_properties = text(element.CachedAriaProperties());
            if let Ok(newer) = element.cast::<IUIAutomationElement8>() {
                props.landmark_type = newer.CachedLandmarkType().map(|kind| kind.0).unwrap_or(0);
                props.localized_landmark = text(newer.CachedLocalizedLandmarkType());
                props.heading_level = newer.CachedHeadingLevel().map(|level| level.0).unwrap_or(0);
            } else if let Ok(newer) = element.cast::<IUIAutomationElement5>() {
                props.landmark_type = newer.CachedLandmarkType().map(|kind| kind.0).unwrap_or(0);
                props.localized_landmark = text(newer.CachedLocalizedLandmarkType());
            }
            props
        }
    }

    fn children(&mut self, node: &Element) -> Result<Vec<Element>, ChildrenUnavailable> {
        let (found, children_cached) = if node.children_cached {
            (
                unsafe { node.element.GetCachedChildren() },
                self.whole_subtree,
            )
        } else {
            // One call brings back the children and, cached on each, their own children:
            // a call per parent rather than per element.
            let found = unsafe {
                node.element.FindAllBuildCache(
                    TreeScope_Children,
                    &self.session.everything,
                    &self.session.step_request,
                )
            };
            (found, true)
        };
        let children = elements(found).map_err(|_| ChildrenUnavailable)?;
        Ok(children
            .into_iter()
            .map(|element| Element {
                element,
                children_cached,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(area: i64, is_offscreen: bool) -> DocumentFacts {
        DocumentFacts { area, is_offscreen }
    }

    #[test]
    fn no_documents_means_no_choice() {
        assert_eq!(choose_document(&[]), None);
    }

    #[test]
    fn the_largest_document_on_screen_is_the_content() {
        // A side panel, the page, and a frame inside the page.
        let candidates = [
            facts(300 * 900, false),
            facts(1_200 * 900, false),
            facts(600 * 400, false),
        ];

        assert_eq!(choose_document(&candidates), Some(1));
    }

    #[test]
    fn a_frame_as_large_as_its_page_does_not_displace_the_page() {
        let candidates = [facts(1_000_000, false), facts(1_000_000, false)];

        assert_eq!(choose_document(&candidates), Some(0));
    }

    #[test]
    fn a_document_on_screen_beats_a_larger_one_off_screen() {
        let candidates = [facts(4_000_000, true), facts(500_000, false)];

        assert_eq!(choose_document(&candidates), Some(1));
    }

    #[test]
    fn documents_without_a_known_place_still_yield_the_first() {
        // A minimised window reports no rectangles at all.
        let candidates = [facts(0, true), facts(0, true)];

        assert_eq!(choose_document(&candidates), Some(0));
    }
}
