#[macro_use]
extern crate tracing;

use crate::error::Error;

pub mod config;
pub mod converter;
pub mod data;
pub mod error;
mod routes;
pub mod server;

pub type Result<T> = anyhow::Result<T, Error>;
