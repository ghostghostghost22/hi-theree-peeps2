//! Host virtualization backends.
//!
//! Only the Linux/x86-64 KVM backend is enabled in the first milestone. The
//! public traits deliberately do not expose KVM types so WHPX and HVF can be
//! added without leaking platform details into the VMM or GUI.

use std::fmt;

use rustbox_core::Architecture;
use rustbox_cpu::VirtualCpu;
use rustbox_memory::GuestMemory;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod kvm;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use kvm::KvmHypervisor;

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
mod unsupported;

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
pub use unsupported::KvmHypervisor;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformError {
    Unavailable(String),
    Unsupported(String),
    Hypervisor(String),
    Memory(String),
}

impl fmt::Display for PlatformError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(message) => write!(formatter, "virtualization unavailable: {message}"),
            Self::Unsupported(message) => write!(formatter, "unsupported platform feature: {message}"),
            Self::Hypervisor(message) => write!(formatter, "host hypervisor failure: {message}"),
            Self::Memory(message) => write!(formatter, "host memory registration failure: {message}"),
        }
    }
}

impl std::error::Error for PlatformError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformCapabilities {
    pub host_architecture: Architecture,
    pub x86_64_guest: bool,
    pub aarch64_guest: bool,
    pub vcpu: bool,
    pub guest_memory: bool,
}

pub trait HypervisorBackend {
    fn capabilities(&self) -> PlatformCapabilities;
    fn create_vm(
        &self,
        architecture: Architecture,
    ) -> Result<Box<dyn VirtualMachineBackend>, PlatformError>;
}

pub trait VirtualMachineBackend {
    fn map_memory(&mut self, memory: &GuestMemory) -> Result<(), PlatformError>;
    fn create_vcpu(&self, id: u32) -> Result<Box<dyn VirtualCpu>, PlatformError>;
}
