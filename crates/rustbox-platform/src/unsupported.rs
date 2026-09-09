use crate::{HypervisorBackend, PlatformCapabilities, PlatformError, VirtualMachineBackend};
use rustbox_core::Architecture;

/// Placeholder value that provides a useful diagnostic on unsupported hosts.
/// It keeps the CLI buildable on Windows, macOS, and non-x86 Linux while the
/// corresponding backends are developed.
pub struct KvmHypervisor;

impl KvmHypervisor {
    pub fn new() -> Result<Self, PlatformError> {
        Err(PlatformError::Unavailable(
            "the first RustBox milestone requires a Linux x86-64 host with KVM".to_owned(),
        ))
    }

    pub const fn api_version(&self) -> i32 {
        0
    }
}

impl HypervisorBackend for KvmHypervisor {
    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities {
            host_architecture: if cfg!(target_arch = "aarch64") {
                Architecture::Aarch64
            } else {
                Architecture::X86_64
            },
            x86_64_guest: false,
            aarch64_guest: false,
            vcpu: false,
            guest_memory: false,
        }
    }

    fn create_vm(
        &self,
        architecture: Architecture,
    ) -> Result<Box<dyn VirtualMachineBackend>, PlatformError> {
        Err(PlatformError::Unsupported(format!(
            "KVM backend is not available for {architecture} on this host"
        )))
    }
}
