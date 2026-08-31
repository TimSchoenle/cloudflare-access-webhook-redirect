//! The configuration compiled into what a request is matched against.
//!
//! [`config`](crate::config) is what an operator writes. A [`WebHookData`] is built once per
//! generation of the runtime and shared by every worker, so a configuration reload replaces it
//! whole rather than mutating it under a request.

mod webhook;

pub use webhook::AllowedPath;
pub use webhook::AllowedPaths;
pub use webhook::WebHookData;
