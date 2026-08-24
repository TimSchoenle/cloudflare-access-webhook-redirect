//! A reverse proxy that fronts a Cloudflare Access protected service for senders that cannot
//! authenticate to it themselves.
//!
//! A webhook sender is pointed here instead of at the protected service. A request whose path and
//! method are both on the configured allow list is forwarded to `webhook.target_base` carrying the
//! Cloudflare Access service token as `CF-Access-Client-Id` and `CF-Access-Client-Secret`. A path
//! or method the allow list refuses is answered `404` rather than `403`, so a caller learns
//! nothing about which paths the protected service has. Only `GET`, `POST`, `PUT`, `PATCH` and
//! `DELETE` are routed at all; any other method gets actix's `405` without reaching the handler.
//!
//! # What each module owns
//!
//! [`config`] is the surface an operator writes and the dialect of [`terrace_config`] it is read
//! through. [`data`] is that same configuration compiled for the request path: one
//! [`RegexSet`](regex::RegexSet) over the anchored patterns, both credentials already parsed as
//! header values, and the shared [`reqwest::Client`]. [`converter`] moves a body, a header map, a
//! status code and a response between actix's types and reqwest's. [`server`] binds the listener,
//! and [`shutdown`] owns the token every part of it stops on.
//!
//! # Failure posture
//!
//! A configuration that cannot be assembled fails the boot, and the binary prints which file or
//! variable each key came from before it exits. A request the allow list refuses is a `404`. A
//! body that cannot be read, an upstream that refuses and a response that cannot be relayed are
//! all a `400`. A reload whose new configuration does not load leaves the previous generation
//! serving; one that loads but whose patterns will not compile, whose rotated credential is not a
//! valid header value, or whose address cannot be bound, ends the process.
//!
//! The credentials stay in [`SecretString`](secrecy::SecretString) from the configuration layer
//! until they become header values in [`WebHookData`](data::WebHookData), so neither a log line
//! nor a `Debug` rendering of [`Config`](config::Config) can carry them.

#[macro_use]
extern crate getset;
#[macro_use]
extern crate tracing;

use crate::error::Error;

pub mod config;
pub mod converter;
pub mod data;
pub mod error;
mod routes;
pub mod server;
pub mod shutdown;

/// A `Result` carrying this crate's [`Error`].
///
/// The request path does not use it. A handler returns [`actix_web::Error`], which is the type
/// actix renders into a response.
pub type Result<T> = anyhow::Result<T, Error>;
