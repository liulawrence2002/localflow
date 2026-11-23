//! Turning what is on screen into something that can be read aloud.
//!
//! The platform layer reads a window's accessibility tree into plain data ([`raw`]). The
//! code here is pure: it decides whether the window may be read at all ([`policy`]), finds
//! the main content and cuts it into readable blocks ([`extract`]), keeps track of where the
//! reader is ([`cursor`]), and prepares block text for a voice without changing what it says
//! ([`speech`]).
//!
//! Page text is as private as dictated text. It is held in memory for one request, never
//! logged, never stored, and never given to a language model by this code.

pub mod cursor;
pub mod extract;
pub mod policy;
pub mod raw;
pub mod speech;
pub mod types;
