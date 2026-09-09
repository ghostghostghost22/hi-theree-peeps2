//! The first RustBox machine model.
//!
//! This crate owns the platform-independent machine orchestration: guest RAM,
//! vCPU creation, the normalized exit loop, and the initial UART device. The
//! current guest loader is deliberately tiny and deterministic so the first
//! integration milestone proves that real guest instructions execute before a
//! Linux kernel loader or GUI is introduced.

use std::fmt;

use rustbox_core::{Architecture, VmConfig, VmError, VmEvent, VmId, VmState};
use rustbox_cpu::{BootState, CpuError, CpuResult, IoDirection, RunAction, VcpuExit, VcpuExitHandler, VirtualCpu};
use rustbox_memory::{GuestAddress, GuestMemory, MemoryError};
use rustbox_platform::{HypervisorBackend, PlatformError, VirtualMachineBackend};

const UART_COM1: u16 = 0x3f8;
const GUEST_ENTRY: GuestAddress = GuestAddress(0);
const GUEST_MESSAGE: GuestAddress = GuestAddress(0x100);
const MAX_GUEST_EXITS: usize = 1_000_000;

#[derive(Debug)]
pub enum VmmError {
    Configuration(VmError),
    Memory(MemoryError),
    Platform(PlatformError),
    Cpu(CpuError),
    InvalidState(String),
    Unsupported(String),
    Guest(String),
}

impl fmt::Display for VmmError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration(error) => write!(formatter, "{error}"),
            Self::Memory(error) => write!(formatter, "guest memory error: {error}"),
            Self::Platform(error) => write!(formatter, "{error}"),
            Self::Cpu(error) => write!(formatter, "{error}"),
            Self::InvalidState(message) => write!(formatter, "invalid VM state: {message}"),
            Self::Unsupported(message) => write!(formatter, "unsupported VMM feature: {message}"),
            Self::Guest(message) => write!(formatter, "guest execution failed: {message}"),
        }
    }
}

impl std::error::Error for VmmError {}

impl From<VmError> for VmmError {
    fn from(error: VmError) -> Self {
        Self::Configuration(error)
    }
}

impl From<MemoryError> for VmmError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

impl From<PlatformError> for VmmError {
    fn from(error: PlatformError) -> Self {
        Self::Platform(error)
    }
}

impl From<CpuError> for VmmError {
    fn from(error: CpuError) -> Self {
        Self::Cpu(error)
    }
}

/// Minimal serial device used by the first guest and by future console tests.
#[derive(Debug, Default)]
pub struct DeviceBus {
    uart_output: Vec<u8>,
}

impl DeviceBus {
    pub fn serial_output(&self) -> &[u8] {
        &self.uart_output
    }

    pub fn take_serial_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.uart_output)
    }

    fn handle_io(
        &mut self,
        direction: IoDirection,
        port: u16,
        data: &mut [u8],
    ) -> CpuResult<RunAction> {
        if port != UART_COM1 {
            return Err(CpuError::InvalidExit(format!(
                "guest accessed unsupported I/O port 0x{port:x}"
            )));
        }
        match direction {
            IoDirection::Out => {
                self.uart_output.extend_from_slice(data);
                Ok(RunAction::Continue)
            }
            IoDirection::In => {
                data.fill(0);
                Ok(RunAction::Continue)
            }
        }
    }
}

impl VcpuExitHandler for DeviceBus {
    fn handle_exit(&mut self, exit: &mut VcpuExit) -> CpuResult<RunAction> {
        match exit {
            VcpuExit::Io {
                direction,
                port,
                data,
            } => self.handle_io(*direction, *port, data),
            VcpuExit::Mmio { address, .. } => Err(CpuError::InvalidExit(format!(
                "guest accessed unsupported MMIO address 0x{:x}",
                *address
            ))),
            VcpuExit::Halt => Ok(RunAction::Halted),
            VcpuExit::Shutdown => Ok(RunAction::Shutdown),
            VcpuExit::Interrupted => Ok(RunAction::Paused),
            VcpuExit::Unknown { reason } => Err(CpuError::InvalidExit(reason.clone())),
        }
    }
}

pub struct VirtualMachine {
    id: VmId,
    config: VmConfig,
    state: VmState,
    // The backend is optional only so Drop can explicitly close it after all
    // vCPU file descriptors and before the registered host allocation.
    backend: Option<Box<dyn VirtualMachineBackend>>,
    memory: GuestMemory,
    vcpus: Vec<Box<dyn VirtualCpu>>,
    devices: DeviceBus,
}

impl Drop for VirtualMachine {
    fn drop(&mut self) {
        // A vCPU owns a KVM_RUN mapping associated with the VM fd. Close all
        // vCPUs before closing the VM fd and releasing the registered
        // guest-memory allocation. Taking the backend makes this ordering
        // explicit instead of relying on struct field drop order.
        self.vcpus.clear();
        let _ = self.backend.take();
    }
}

impl VirtualMachine {
    pub fn new(
        id: VmId,
        config: VmConfig,
        hypervisor: &dyn HypervisorBackend,
    ) -> Result<Self, VmmError> {
        config.validate()?;
        let memory = GuestMemory::new(config.memory_bytes()?)?;
        let mut backend = hypervisor.create_vm(config.architecture)?;
        backend.map_memory(&memory)?;
        Ok(Self {
            id,
            config,
            state: VmState::Created,
            memory,
            backend: Some(backend),
            vcpus: Vec::new(),
            devices: DeviceBus::default(),
        })
    }

    pub fn id(&self) -> VmId {
        self.id
    }

    pub fn config(&self) -> &VmConfig {
        &self.config
    }

    pub fn state(&self) -> VmState {
        self.state
    }

    pub fn memory(&self) -> &GuestMemory {
        &self.memory
    }

    pub fn devices(&self) -> &DeviceBus {
        &self.devices
    }

    /// Execute the deterministic RustBox guest through a real vCPU.
    ///
    /// This is intentionally the first end-to-end milestone. It validates
    /// guest RAM registration, x86 real-mode setup, instruction execution,
    /// port-I/O exits, UART handling, and a clean HLT exit. A kernel loader is
    /// a separate feature and is rejected rather than silently ignored.
    pub fn run_builtin_guest(&mut self) -> Result<String, VmmError> {
        if self.state != VmState::Created && self.state != VmState::Stopped {
            return Err(VmmError::InvalidState(format!(
                "cannot start a VM while it is {}",
                self.state
            )));
        }
        if self.config.architecture != Architecture::X86_64 {
            return Err(VmmError::Unsupported(
                "the built-in guest currently targets x86_64".to_owned(),
            ));
        }
        if self.config.cpus != 1 {
            return Err(VmmError::Unsupported(
                "the built-in guest currently requires exactly one vCPU".to_owned(),
            ));
        }
        if self.config.boot.kernel.is_some() {
            return Err(VmmError::Unsupported(
                "Linux kernel loading has not been enabled in the first milestone".to_owned(),
            ));
        }
        if self.config.boot.guest.as_deref() != Some("hello") {
            return Err(VmmError::Unsupported(
                "only the built-in guest named 'hello' is available".to_owned(),
            ));
        }

        self.state = self.state.transition(VmEvent::Start)?;
        let result = self.run_builtin_guest_inner();
        if result.is_err() {
            self.state = match self.state.transition(VmEvent::Crash) {
                Ok(state) => state,
                Err(_) => VmState::Crashed,
            };
        }
        result
    }

    fn run_builtin_guest_inner(&mut self) -> Result<String, VmmError> {
        let message = b"Hello from guest!\n";
        let program = build_hello_program(message.len())?;
        self.memory.write(GUEST_ENTRY, &program)?;
        self.memory.write(GUEST_MESSAGE, message)?;

        let stack = self
            .memory
            .max_address()
            .0
            .checked_sub(16)
            .ok_or_else(|| VmmError::Guest("guest RAM is too small for a stack".to_owned()))?;
        // A stopped run retains its vCPU for register inspection. Close it
        // before creating the next vCPU with the same KVM id.
        self.vcpus.clear();
        let mut vcpu = self
            .backend
            .as_ref()
            .ok_or_else(|| VmmError::InvalidState("VM backend is no longer available".to_owned()))?
            .create_vcpu(0)?;
        vcpu.initialize(BootState::X86RealMode {
            entry: GUEST_ENTRY.0,
            stack,
        })?;

        self.state = self.state.transition(VmEvent::Started)?;
        let mut devices = DeviceBus::default();
        let mut exits = 0usize;
        let output = loop {
            exits += 1;
            if exits > MAX_GUEST_EXITS {
                return Err(VmmError::Guest(
                    "built-in guest exceeded the exit budget".to_owned(),
                ));
            }
            match vcpu.run(&mut devices)? {
                RunAction::Continue => continue,
                RunAction::Halted | RunAction::Shutdown => {
                    break String::from_utf8_lossy(devices.serial_output()).into_owned();
                }
                RunAction::Paused => {
                    return Err(VmmError::Guest(
                        "built-in guest was interrupted before it halted".to_owned(),
                    ));
                }
            }
        };

        self.state = self.state.transition(VmEvent::Stop)?;
        self.state = self.state.transition(VmEvent::Stopped)?;
        self.devices = devices;
        self.vcpus.clear();
        self.vcpus.push(vcpu);
        Ok(output)
    }
}

fn build_hello_program(message_length: usize) -> Result<Vec<u8>, VmmError> {
    let message_length = u8::try_from(message_length)
        .map_err(|_| VmmError::Guest("built-in guest message is too long".to_owned()))?;
    // x86 real-mode instructions:
    //   mov dx, 0x3f8
    //   mov si, 0x0100
    //   mov cx, message_length
    // loop: lodsb; out dx, al; loop loop; hlt
    Ok(vec![
        0xba,
        0xf8,
        0x03,
        0xbe,
        0x00,
        0x01,
        0xb9,
        message_length,
        0x00,
        0xac,
        0xee,
        0xe2,
        0xfc,
        0xf4,
    ])
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;
    use rustbox_cpu::{Registers, X86_64Registers};
    use rustbox_platform::PlatformCapabilities;

    #[test]
    fn hello_program_has_a_halt_instruction() {
        let program = build_hello_program(4).expect("program builds");
        assert_eq!(program.last(), Some(&0xf4));
    }

    #[test]
    fn uart_collects_serial_output() {
        let mut bus = DeviceBus::default();
        let mut exit = VcpuExit::Io {
            direction: IoDirection::Out,
            port: UART_COM1,
            data: b"hello".to_vec(),
        };
        let action = bus.handle_exit(&mut exit).expect("UART accepts output");
        assert_eq!(action, RunAction::Continue);
        assert_eq!(bus.serial_output(), b"hello");
    }

    #[test]
    fn machine_run_loop_is_testable_without_host_kvm() {
        let mut config = VmConfig::new("mock").expect("configuration is valid");
        config.memory_mib = 1;
        let hypervisor = MockHypervisor;
        let mut machine = VirtualMachine::new(VmId::new(), config, &hypervisor)
            .expect("mock machine can be created");
        let output = machine
            .run_builtin_guest()
            .expect("mock exits complete the guest");
        assert_eq!(output, "mock output");
        assert_eq!(machine.state(), VmState::Stopped);
    }

    struct MockHypervisor;

    impl HypervisorBackend for MockHypervisor {
        fn capabilities(&self) -> PlatformCapabilities {
            PlatformCapabilities {
                host_architecture: Architecture::X86_64,
                x86_64_guest: true,
                aarch64_guest: false,
                vcpu: true,
                guest_memory: true,
            }
        }

        fn create_vm(
            &self,
            architecture: Architecture,
        ) -> Result<Box<dyn VirtualMachineBackend>, PlatformError> {
            if architecture != Architecture::X86_64 {
                return Err(PlatformError::Unsupported(
                    "mock backend only supports x86_64".to_owned(),
                ));
            }
            Ok(Box::new(MockMachine))
        }
    }

    struct MockMachine;

    impl VirtualMachineBackend for MockMachine {
        fn map_memory(&mut self, _memory: &GuestMemory) -> Result<(), PlatformError> {
            Ok(())
        }

        fn create_vcpu(&self, id: u32) -> Result<Box<dyn VirtualCpu>, PlatformError> {
            if id != 0 {
                return Err(PlatformError::Unsupported(
                    "mock backend has one vCPU".to_owned(),
                ));
            }
            Ok(Box::new(MockVcpu {
                exits: VecDeque::from(vec![
                    VcpuExit::Io {
                        direction: IoDirection::Out,
                        port: UART_COM1,
                        data: b"mock output".to_vec(),
                    },
                    VcpuExit::Halt,
                ]),
                paused: false,
            }))
        }
    }

    struct MockVcpu {
        exits: VecDeque<VcpuExit>,
        paused: bool,
    }

    impl VirtualCpu for MockVcpu {
        fn run(&mut self, handler: &mut dyn VcpuExitHandler) -> CpuResult<RunAction> {
            if self.paused {
                let mut exit = VcpuExit::Interrupted;
                return handler.handle_exit(&mut exit);
            }
            let mut exit = self.exits.pop_front().ok_or_else(|| {
                CpuError::InvalidExit("mock guest ran out of exits".to_owned())
            })?;
            handler.handle_exit(&mut exit)
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
            Ok(Registers::X86_64(X86_64Registers::default()))
        }

        fn set_registers(&mut self, registers: &Registers) -> CpuResult<()> {
            if registers.architecture() != Architecture::X86_64 {
                return Err(CpuError::InvalidRegisters(
                    "mock backend received non-x86 registers".to_owned(),
                ));
            }
            Ok(())
        }

        fn initialize(&mut self, boot: BootState) -> CpuResult<()> {
            if boot.architecture() != Architecture::X86_64 {
                return Err(CpuError::Unsupported(
                    "mock backend only supports x86_64".to_owned(),
                ));
            }
            Ok(())
        }
    }
}
