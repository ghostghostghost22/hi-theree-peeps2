use std::fmt;

use crate::VmError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VmState {
    Created,
    Starting,
    Running,
    Paused,
    Stopping,
    Stopped,
    Crashed,
}

impl fmt::Display for VmState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Created => "Created",
            Self::Starting => "Starting",
            Self::Running => "Running",
            Self::Paused => "Paused",
            Self::Stopping => "Stopping",
            Self::Stopped => "Stopped",
            Self::Crashed => "Crashed",
        };
        formatter.write_str(name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VmEvent {
    Start,
    Started,
    Pause,
    Resume,
    Stop,
    Stopped,
    Crash,
    Reset,
}

impl fmt::Display for VmEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Start => "start",
            Self::Started => "started",
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Stop => "stop",
            Self::Stopped => "stopped",
            Self::Crash => "crash",
            Self::Reset => "reset",
        };
        formatter.write_str(name)
    }
}

impl VmState {
    /// Apply one lifecycle event, rejecting illegal transitions.
    pub fn transition(self, event: VmEvent) -> Result<Self, VmError> {
        let next = match (self, event) {
            (Self::Created, VmEvent::Start) | (Self::Stopped, VmEvent::Start) => Self::Starting,
            (Self::Starting, VmEvent::Started) => Self::Running,
            (Self::Running, VmEvent::Pause) => Self::Paused,
            (Self::Paused, VmEvent::Resume) => Self::Running,
            (Self::Running, VmEvent::Stop) | (Self::Paused, VmEvent::Stop) => Self::Stopping,
            (Self::Stopping, VmEvent::Stopped) => Self::Stopped,
            (Self::Starting, VmEvent::Crash)
            | (Self::Running, VmEvent::Crash)
            | (Self::Paused, VmEvent::Crash)
            | (Self::Stopping, VmEvent::Crash) => Self::Crashed,
            (Self::Crashed, VmEvent::Reset) => Self::Stopped,
            _ => {
                return Err(VmError::InvalidTransition { from: self, event });
            }
        };
        Ok(next)
    }

    pub const fn is_active(self) -> bool {
        matches!(self, Self::Starting | Self::Running | Self::Paused | Self::Stopping)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_lifecycle_is_strict() {
        let state = VmState::Created
            .transition(VmEvent::Start)
            .expect("created can start")
            .transition(VmEvent::Started)
            .expect("starting can become running")
            .transition(VmEvent::Pause)
            .expect("running can pause")
            .transition(VmEvent::Resume)
            .expect("paused can resume")
            .transition(VmEvent::Stop)
            .expect("running can stop")
            .transition(VmEvent::Stopped)
            .expect("stopping can finish");
        assert_eq!(state, VmState::Stopped);
    }

    #[test]
    fn illegal_transition_is_rejected() {
        let error = VmState::Created
            .transition(VmEvent::Pause)
            .expect_err("created cannot pause");
        assert!(matches!(error, VmError::InvalidTransition { .. }));
    }
}
