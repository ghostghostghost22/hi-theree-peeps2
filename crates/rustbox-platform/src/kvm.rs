use kvm_bindings::{kvm_regs, kvm_userspace_memory_region};
use kvm_ioctls::{Cap, Kvm, VcpuExit as KvmVcpuExit, VcpuFd, VmFd};
use rustbox_core::Architecture;
use rustbox_cpu::{
    BootState, CpuError, CpuResult, IoDirection, Registers, RunAction,
    VcpuExit, VcpuExitHandler, VirtualCpu, X86_64Registers,
};
use rustbox_memory::GuestMemory;

use crate::{HypervisorBackend, PlatformCapabilities, PlatformError, VirtualMachineBackend};

pub struct KvmHypervisor {
    kvm: Kvm,
}

impl KvmHypervisor {
    pub fn new() -> Result<Self, PlatformError> {
        let kvm = Kvm::new().map_err(|error| {
            PlatformError::Unavailable(format!(
                "cannot open /dev/kvm ({error}); enable hardware virtualization and grant the current user access to /dev/kvm"
            ))
        })?;
        let api_version = kvm.get_api_version();
        if api_version < 0 {
            return Err(PlatformError::Unavailable(format!(
                "KVM reported an invalid API version: {api_version}"
            )));
        }
        Ok(Self { kvm })
    }

    pub fn api_version(&self) -> i32 {
        self.kvm.get_api_version()
    }
}

impl HypervisorBackend for KvmHypervisor {
    fn capabilities(&self) -> PlatformCapabilities {
        let guest_memory = self.kvm.check_extension(Cap::UserMemory);
        let vcpu = self.kvm.get_nr_vcpus() > 0;
        PlatformCapabilities {
            host_architecture: Architecture::X86_64,
            x86_64_guest: guest_memory && vcpu,
            aarch64_guest: false,
            vcpu,
            guest_memory,
        }
    }

    fn create_vm(
        &self,
        architecture: Architecture,
    ) -> Result<Box<dyn VirtualMachineBackend>, PlatformError> {
        if architecture != Architecture::X86_64 {
            return Err(PlatformError::Unsupported(format!(
                "Linux KVM milestone supports x86_64 guests, not {architecture}"
            )));
        }
        let vm = self
            .kvm
            .create_vm()
            .map_err(|error| PlatformError::Hypervisor(format!("KVM_CREATE_VM failed: {error}")))?;
        Ok(Box::new(KvmVirtualMachine { vm, mapped: false }))
    }
}

struct KvmVirtualMachine {
    vm: VmFd,
    mapped: bool,
}

impl VirtualMachineBackend for KvmVirtualMachine {
    fn map_memory(&mut self, memory: &GuestMemory) -> Result<(), PlatformError> {
        if self.mapped {
            return Err(PlatformError::Memory(
                "guest memory was registered more than once".to_owned(),
            ));
        }
        for (slot, region) in memory.regions().enumerate() {
            let slot = u32::try_from(slot).map_err(|_| {
                PlatformError::Memory("guest memory has more regions than KVM supports".to_owned())
            })?;
            let descriptor = kvm_userspace_memory_region {
                slot,
                flags: 0,
                guest_phys_addr: region.guest_base().0,
                memory_size: region.len(),
                userspace_addr: region.host_address() as u64,
            };
            // KVM requires the userspace mapping to remain alive for the life
            // of the VM. The VMM owns `GuestMemory` for at least that long.
            unsafe {
                self.vm.set_user_memory_region(descriptor).map_err(|error| {
                    PlatformError::Memory(format!("KVM_SET_USER_MEMORY_REGION failed: {error}"))
                })?;
            }
        }
        self.mapped = true;
        Ok(())
    }

    fn create_vcpu(&self, id: u32) -> Result<Box<dyn VirtualCpu>, PlatformError> {
        if !self.mapped {
            return Err(PlatformError::Memory(
                "guest memory must be registered before creating a vCPU".to_owned(),
            ));
        }
        let vcpu = self
            .vm
            .create_vcpu(u64::from(id))
            .map_err(|error| PlatformError::Hypervisor(format!("KVM_CREATE_VCPU failed: {error}")))?;
        Ok(Box::new(KvmVcpu {
            fd: vcpu,
            paused: false,
        }))
    }
}

struct KvmVcpu {
    fd: VcpuFd,
    paused: bool,
}

impl KvmVcpu {
    fn hypervisor_error(error: impl std::fmt::Display) -> CpuError {
        CpuError::Hypervisor(error.to_string())
    }

    fn configure_x86_real_mode(&mut self, entry: u64, stack: u64) -> CpuResult<()> {
        let mut sregs = self.fd.get_sregs().map_err(Self::hypervisor_error)?;
        sregs.cs.base = 0;
        sregs.cs.selector = 0;
        self.fd.set_sregs(&sregs).map_err(Self::hypervisor_error)?;

        let registers = kvm_regs {
            rax: 0,
            rbx: 0,
            rcx: 0,
            rdx: 0,
            rsi: 0,
            rdi: 0,
            rsp: stack,
            rbp: 0,
            r8: 0,
            r9: 0,
            r10: 0,
            r11: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rip: entry,
            rflags: 2,
        };
        self.fd.set_regs(&registers).map_err(Self::hypervisor_error)
    }

    fn dispatch(
        handler: &mut dyn VcpuExitHandler,
        exit: &mut VcpuExit,
    ) -> CpuResult<RunAction> {
        handler.handle_exit(exit)
    }
}

impl VirtualCpu for KvmVcpu {
    fn run(&mut self, handler: &mut dyn VcpuExitHandler) -> CpuResult<RunAction> {
        if self.paused {
            let mut exit = VcpuExit::Interrupted;
            return Self::dispatch(handler, &mut exit);
        }

        match self.fd.run().map_err(Self::hypervisor_error)? {
            KvmVcpuExit::IoIn(port, data) => {
                let mut exit = VcpuExit::Io {
                    direction: IoDirection::In,
                    port,
                    data: vec![0; data.len()],
                };
                let action = Self::dispatch(handler, &mut exit)?;
                if let VcpuExit::Io {
                    direction: IoDirection::In,
                    data: response,
                    ..
                } = &exit
                {
                    if response.len() != data.len() {
                        return Err(CpuError::InvalidExit(
                            "device changed the length of a KVM input payload".to_owned(),
                        ));
                    }
                    data.copy_from_slice(response);
                }
                Ok(action)
            }
            KvmVcpuExit::IoOut(port, data) => {
                let mut exit = VcpuExit::Io {
                    direction: IoDirection::Out,
                    port,
                    data: data.to_vec(),
                };
                Self::dispatch(handler, &mut exit)
            }
            KvmVcpuExit::MmioRead(address, data) => {
                let mut exit = VcpuExit::Mmio {
                    direction: IoDirection::In,
                    address,
                    data: vec![0; data.len()],
                };
                let action = Self::dispatch(handler, &mut exit)?;
                if let VcpuExit::Mmio {
                    direction: IoDirection::In,
                    data: response,
                    ..
                } = &exit
                {
                    if response.len() != data.len() {
                        return Err(CpuError::InvalidExit(
                            "device changed the length of a KVM MMIO input payload".to_owned(),
                        ));
                    }
                    data.copy_from_slice(response);
                }
                Ok(action)
            }
            KvmVcpuExit::MmioWrite(address, data) => {
                let mut exit = VcpuExit::Mmio {
                    direction: IoDirection::Out,
                    address,
                    data: data.to_vec(),
                };
                Self::dispatch(handler, &mut exit)
            }
            KvmVcpuExit::Hlt => {
                let mut exit = VcpuExit::Halt;
                Self::dispatch(handler, &mut exit)
            }
            KvmVcpuExit::Shutdown => {
                let mut exit = VcpuExit::Shutdown;
                Self::dispatch(handler, &mut exit)
            }
            KvmVcpuExit::Intr => {
                let mut exit = VcpuExit::Interrupted;
                Self::dispatch(handler, &mut exit)
            }
            _ => {
                let mut exit = VcpuExit::Unknown {
                    reason: "unhandled KVM exit".to_owned(),
                };
                Self::dispatch(handler, &mut exit)
            }
        }
    }

    fn pause(&mut self) -> CpuResult<()> {
        self.paused = true;
        Ok(())
    }

    fn resume(&mut self) -> CpuResult<()> {
        self.paused = false;
        Ok(())
    }

    fn get_registers(&self) -> CpuResult<Registers> {
        let registers = self.fd.get_regs().map_err(Self::hypervisor_error)?;
        Ok(Registers::X86_64(X86_64Registers {
            rax: registers.rax,
            rbx: registers.rbx,
            rcx: registers.rcx,
            rdx: registers.rdx,
            rsi: registers.rsi,
            rdi: registers.rdi,
            rsp: registers.rsp,
            rbp: registers.rbp,
            r8: registers.r8,
            r9: registers.r9,
            r10: registers.r10,
            r11: registers.r11,
            r12: registers.r12,
            r13: registers.r13,
            r14: registers.r14,
            r15: registers.r15,
            rip: registers.rip,
            rflags: registers.rflags,
        }))
    }

    fn set_registers(&mut self, registers: &Registers) -> CpuResult<()> {
        let Registers::X86_64(registers) = registers else {
            return Err(CpuError::InvalidRegisters(
                "x86 KVM vCPU received non-x86 registers".to_owned(),
            ));
        };
        let registers = kvm_regs {
            rax: registers.rax,
            rbx: registers.rbx,
            rcx: registers.rcx,
            rdx: registers.rdx,
            rsi: registers.rsi,
            rdi: registers.rdi,
            rsp: registers.rsp,
            rbp: registers.rbp,
            r8: registers.r8,
            r9: registers.r9,
            r10: registers.r10,
            r11: registers.r11,
            r12: registers.r12,
            r13: registers.r13,
            r14: registers.r14,
            r15: registers.r15,
            rip: registers.rip,
            rflags: registers.rflags,
        };
        self.fd.set_regs(&registers).map_err(Self::hypervisor_error)
    }

    fn initialize(&mut self, boot: BootState) -> CpuResult<()> {
        match boot {
            BootState::X86RealMode { entry, stack } => self.configure_x86_real_mode(entry, stack),
            BootState::Aarch64 { .. } => Err(CpuError::Unsupported(
                "AArch64 boot state is not implemented by the KVM milestone".to_owned(),
            )),
        }
    }
}
