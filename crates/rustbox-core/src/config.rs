use std::{fmt, path::PathBuf, str::FromStr};

use crate::VmError;

const MAX_VM_NAME_LENGTH: usize = 64;
const MAX_MEMORY_MIB: u64 = 1 << 20;
const MAX_VCPUS: u32 = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Architecture {
    X86_64,
    Aarch64,
}

impl Architecture {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
        }
    }
}

impl fmt::Display for Architecture {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Architecture {
    type Err = VmError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "x86_64" | "x86-64" => Ok(Self::X86_64),
            "aarch64" | "arm64" => Ok(Self::Aarch64),
            other => Err(VmError::InvalidConfiguration(format!(
                "unsupported architecture {other:?}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootConfig {
    /// A built-in guest is useful for integration tests before a kernel loader
    /// exists. Production configurations should set `kernel` instead.
    pub guest: Option<String>,
    pub kernel: Option<PathBuf>,
    pub initrd: Option<PathBuf>,
}

impl Default for BootConfig {
    fn default() -> Self {
        Self {
            guest: Some("hello".to_owned()),
            kernel: None,
            initrd: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskFormat {
    Raw,
}

impl fmt::Display for DiskFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Raw => formatter.write_str("raw"),
        }
    }
}

impl FromStr for DiskFormat {
    type Err = VmError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "raw" => Ok(Self::Raw),
            other => Err(VmError::InvalidConfiguration(format!(
                "unsupported disk format {other:?}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskConfig {
    pub path: PathBuf,
    pub format: DiskFormat,
    pub read_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkMode {
    None,
    Nat,
}

impl fmt::Display for NetworkMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => formatter.write_str("none"),
            Self::Nat => formatter.write_str("nat"),
        }
    }
}

impl FromStr for NetworkMode {
    type Err = VmError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "none" => Ok(Self::None),
            "nat" => Ok(Self::Nat),
            other => Err(VmError::InvalidConfiguration(format!(
                "unsupported network mode {other:?}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkConfig {
    pub mode: NetworkMode,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self { mode: NetworkMode::None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayConfig {
    pub enabled: bool,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self { enabled: false }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmConfig {
    pub name: String,
    /// Guest RAM in mebibytes. The value is deliberately explicit instead of
    /// accepting a host-dependent default at a lower layer.
    pub memory_mib: u64,
    pub cpus: u32,
    pub architecture: Architecture,
    pub boot: BootConfig,
    pub disks: Vec<DiskConfig>,
    pub network: NetworkConfig,
    pub display: DisplayConfig,
}

impl VmConfig {
    pub fn new(name: impl Into<String>) -> Result<Self, VmError> {
        let config = Self {
            name: name.into(),
            memory_mib: 128,
            cpus: 1,
            architecture: Architecture::X86_64,
            boot: BootConfig::default(),
            disks: Vec::new(),
            network: NetworkConfig::default(),
            display: DisplayConfig::default(),
        };
        config.validate()?;
        Ok(config)
    }

    pub fn memory_bytes(&self) -> Result<u64, VmError> {
        self.memory_mib.checked_mul(1024 * 1024).ok_or_else(|| {
            VmError::InvalidConfiguration("memory size overflows a 64-bit address".to_owned())
        })
    }

    pub fn validate(&self) -> Result<(), VmError> {
        if self.name.is_empty() || self.name.len() > MAX_VM_NAME_LENGTH {
            return Err(VmError::InvalidConfiguration(format!(
                "name must contain between 1 and {MAX_VM_NAME_LENGTH} characters"
            )));
        }
        if !self
            .name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        {
            return Err(VmError::InvalidConfiguration(
                "name may contain only ASCII letters, digits, '-' and '_'".to_owned(),
            ));
        }
        if self.memory_mib == 0 || self.memory_mib > MAX_MEMORY_MIB {
            return Err(VmError::InvalidConfiguration(format!(
                "memory_mib must be between 1 and {MAX_MEMORY_MIB}"
            )));
        }
        if self.cpus == 0 || self.cpus > MAX_VCPUS {
            return Err(VmError::InvalidConfiguration(format!(
                "cpus must be between 1 and {MAX_VCPUS}"
            )));
        }
        if self.boot.guest.is_some() && self.boot.kernel.is_some() {
            return Err(VmError::InvalidConfiguration(
                "boot.guest and boot.kernel are mutually exclusive".to_owned(),
            ));
        }
        if self.boot.initrd.is_some() && self.boot.kernel.is_none() {
            return Err(VmError::InvalidConfiguration(
                "boot.initrd requires boot.kernel".to_owned(),
            ));
        }
        for disk in &self.disks {
            if disk.path.as_os_str().is_empty() {
                return Err(VmError::InvalidConfiguration(
                    "disk path cannot be empty".to_owned(),
                ));
            }
        }
        self.memory_bytes()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        let config = VmConfig::new("test-vm").expect("default configuration is valid");
        assert_eq!(config.memory_bytes().expect("memory converts"), 128 * 1024 * 1024);
    }

    #[test]
    fn names_are_safe_for_config_filenames() {
        assert!(VmConfig::new("../escape").is_err());
        assert!(VmConfig::new("safe_name-01").is_ok());
    }

    #[test]
    fn boot_sources_cannot_be_combined() {
        let mut config = VmConfig::new("test").expect("config");
        config.boot.kernel = Some(PathBuf::from("vmlinuz"));
        assert!(config.validate().is_err());
    }
}
