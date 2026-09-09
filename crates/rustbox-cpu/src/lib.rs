//! Host-hypervisor-independent vCPU contracts.

use std::fmt;

use rustbox_core::Architecture;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CpuError {
    Unsupported(String),
    Hypervisor(String),
    InvalidRegisters(String),
    InvalidExit(String),
}

impl fmt::Display for CpuError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(message) => write!(formatter, "unsupported CPU operation: {message}"),
            Self::Hypervisor(message) => write!(formatter, "hypervisor CPU failure: {message}"),
            Self::InvalidRegisters(message) => write!(formatter, "invalid CPU registers: {message}"),
            Self::InvalidExit(message) => write!(formatter, "invalid vCPU exit: {message}"),
        }
    }
}

impl std::error::Error for CpuError {}

pub type CpuResult<T> = Result<T, CpuError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoDirection {
    In,
    Out,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VcpuExit {
    Io {
        direction: IoDirection,
        port: u16,
        data: Vec<u8>,
    },
    Mmio {
        direction: IoDirection,
        address: u64,
        data: Vec<u8>,
    },
    Halt,
    Shutdown,
    Interrupted,
    Unknown { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunAction {
    Continue,
    Halted,
    Shutdown,
    Paused,
}

/// Handles a normalized host-hypervisor exit.
///
/// Input exits carry a mutable byte vector. A device model fills that vector,
/// and the platform implementation copies it back into the host hypervisor's
/// exit structure before resuming the guest.
pub trait VcpuExitHandler {
    fn handle_exit(&mut self, exit: &mut VcpuExit) -> CpuResult<RunAction>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct X86_64Registers {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rsp: u64,
    pub rbp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
}

impl Default for X86_64Registers {
    fn default() -> Self {
        Self {
            rax: 0,
            rbx: 0,
            rcx: 0,
            rdx: 0,
            rsi: 0,
            rdi: 0,
            rsp: 0,
            rbp: 0,
            r8: 0,
            r9: 0,
            r10: 0,
            r11: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rip: 0,
            rflags: 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aarch64Registers {
    pub general: [u64; 31],
    pub pc: u64,
    pub pstate: u64,
    pub sp: u64,
}

impl Default for Aarch64Registers {
    fn default() -> Self {
        Self {
            general: [0; 31],
            pc: 0,
            pstate: 0,
            sp: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Registers {
    X86_64(X86_64Registers),
    Aarch64(Aarch64Registers),
}

impl Registers {
    pub const fn architecture(&self) -> Architecture {
        match self {
            Self::X86_64(_) => Architecture::X86_64,
            Self::Aarch64(_) => Architecture::Aarch64,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootState {
    X86RealMode { entry: u64, stack: u64 },
    Aarch64 { entry: u64, stack: u64 },
}

impl BootState {
    pub const fn architecture(self) -> Architecture {
        match self {
            Self::X86RealMode { .. } => Architecture::X86_64,
            Self::Aarch64 { .. } => Architecture::Aarch64,
        }
    }
}

/// A vCPU whose exits are normalized across KVM, WHPX, and HVF.
pub trait VirtualCpu {
    fn run(&mut self, handler: &mut dyn VcpuExitHandler) -> CpuResult<RunAction>;
    fn pause(&mut self) -> CpuResult<()>;
    fn resume(&mut self) -> CpuResult<()>;
    fn get_registers(&self) -> CpuResult<Registers>;
    fn set_registers(&mut self, registers: &Registers) -> CpuResult<()>;
    fn initialize(&mut self, boot: BootState) -> CpuResult<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_state_has_an_explicit_architecture() {
        assert_eq!(
            BootState::X86RealMode { entry: 0, stack: 0 }.architecture(),
            Architecture::X86_64
        );
    }

    #[test]
    fn input_exits_are_mutable_device_payloads() {
        let mut exit = VcpuExit::Io {
            direction: IoDirection::In,
            port: 0x3f8,
            data: vec![0],
        };
        if let VcpuExit::Io { data, .. } = &mut exit {
            data[0] = b'R';
        }
        assert!(matches!(exit, VcpuExit::Io { data, .. } if data == vec![b'R']));
    }
}
