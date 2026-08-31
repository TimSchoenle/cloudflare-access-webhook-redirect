//! The two things the listener serves.
//!
//! Private, because a route is reached over HTTP and not by a caller. `health_check` answers
//! `/health` and nothing else; `redirect` claims every remaining path and decides, per request,
//! whether it is forwarded.

pub mod health_check;
pub mod redirect;
