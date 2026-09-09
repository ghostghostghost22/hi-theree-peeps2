//! Platform-independent RustBox domain types.
//!
//! This crate intentionally contains no hypervisor, storage, GUI, or operating
//! system code. Keeping the VM contract here makes it possible for the CLI,
//! daemon, and future GUI to share the same lifecycle and configuration rules.

mod config;
mod error;
mod id;
mod lifecycle;

pub use config::{
    Architecture, BootConfig, DiskConfig, DiskFormat, DisplayConfig, NetworkConfig, NetworkMode,
    VmConfig,
};
pub use error::VmError;
pub use id::{VmId, VmIdParseError};
pub use lifecycle::{VmEvent, VmState};
