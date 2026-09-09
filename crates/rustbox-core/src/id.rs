use std::{fmt, str::FromStr, sync::atomic::{AtomicU64, Ordering}, time::{SystemTime, UNIX_EPOCH}};

use crate::VmError;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Stable identifier for a VM.
///
/// IDs are formatted as 32 hexadecimal characters so they remain safe to use
/// in filenames and IPC payloads. They are identifiers, not security tokens;
/// callers must not use them as authentication credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VmId(u128);

impl VmId {
    pub fn new() -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let counter = u128::from(NEXT_ID.fetch_add(1, Ordering::Relaxed));
        let process = u128::from(std::process::id());
        Self((timestamp << 64) ^ (process << 32) ^ counter)
    }

    pub const fn from_u128(value: u128) -> Self {
        Self(value)
    }

    pub const fn as_u128(self) -> u128 {
        self.0
    }
}

impl Default for VmId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for VmId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:032x}", self.0)
    }
}

/// Error returned when parsing a VM id from a user-facing identifier.
pub type VmIdParseError = VmError;

impl FromStr for VmId {
    type Err = VmIdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty() || value.len() > 32 {
            return Err(VmError::InvalidId(
                "expected one to thirty-two hexadecimal characters".to_owned(),
            ));
        }
        u128::from_str_radix(value, 16)
            .map(Self::from_u128)
            .map_err(|_| VmError::InvalidId(format!("{value:?} is not hexadecimal")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_through_display() {
        let id = VmId::new();
        let parsed: VmId = id.to_string().parse().expect("generated id parses");
        assert_eq!(id, parsed);
    }

    #[test]
    fn short_hex_ids_are_accepted() {
        let id: VmId = "2a".parse().expect("hex id parses");
        assert_eq!(id.as_u128(), 42);
        assert_eq!(id.to_string(), "0000000000000000000000000000002a");
    }
}
