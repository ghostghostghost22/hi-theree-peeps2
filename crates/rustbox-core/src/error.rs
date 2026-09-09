use std::fmt;

use crate::{VmEvent, VmState};

/// Errors that can be detected without talking to a host hypervisor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VmError {
    InvalidConfiguration(String),
    InvalidTransition { from: VmState, event: VmEvent },
    InvalidId(String),
}

impl fmt::Display for VmError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration(message) => {
                write!(formatter, "invalid VM configuration: {message}")
            }
            Self::InvalidTransition { from, event } => {
                write!(formatter, "cannot apply {event} while VM is {from}")
            }
            Self::InvalidId(message) => write!(formatter, "invalid VM id: {message}"),
        }
    }
}

impl std::error::Error for VmError {}
