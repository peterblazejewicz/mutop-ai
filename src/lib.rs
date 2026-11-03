// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Piotr Błażejewicz and contributors

pub mod hardware;
pub mod snapshot;

pub use hardware::{CollectorMode, HardwareCollector};
pub use snapshot::{get_schema_json, print_schema, MutopSnapshot};

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
