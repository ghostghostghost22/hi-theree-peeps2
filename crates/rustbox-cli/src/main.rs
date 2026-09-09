use std::{
    env,
    ffi::OsStr,
    fmt,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::PathBuf,
    process,
    str::FromStr,
};

use rustbox_core::{Architecture, NetworkMode, VmConfig, VmError, VmId};
use rustbox_platform::{HypervisorBackend, KvmHypervisor};
use rustbox_vmm::{VirtualMachine, VmmError};

const VERSION: &str = "0.1.0";

fn main() {
    if let Err(error) = run() {
        eprintln!("rustbox: {error}");
        process::exit(1);
    }
}

#[derive(Debug)]
enum CliError {
    Usage(String),
    Io(io::Error),
    Configuration(VmError),
    Vmm(VmmError),
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) => write!(formatter, "{message}\n\nRun `rustbox help` for usage."),
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Configuration(error) => write!(formatter, "{error}"),
            Self::Vmm(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for CliError {}

impl From<io::Error> for CliError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<VmError> for CliError {
    fn from(error: VmError) -> Self {
        Self::Configuration(error)
    }
}

impl From<VmmError> for CliError {
    fn from(error: VmmError) -> Self {
        Self::Vmm(error)
    }
}

fn run() -> Result<(), CliError> {
    let mut arguments = env::args().skip(1);
    let command = arguments.next().unwrap_or_else(|| "help".to_owned());
    let rest: Vec<String> = arguments.collect();

    match command.as_str() {
        "help" | "--help" | "-h" => {
            print_help();
            Ok(())
        }
        "--version" | "-V" | "version" => {
            println!("rustbox {VERSION}");
            Ok(())
        }
        "create" => create_command(&rest),
        "list" => list_command(&rest),
        "inspect" => inspect_command(&rest),
        "destroy" | "delete" => destroy_command(&rest),
        "start" => start_command(&rest),
        "hello" => hello_command(&rest),
        "capabilities" => capabilities_command(&rest),
        other => Err(CliError::Usage(format!("unknown command {other:?}"))),
    }
}

fn print_help() {
    println!(
        "RustBox {VERSION} — a Rust-native virtual machine foundation\n\n\
         USAGE:\n    rustbox <COMMAND> [OPTIONS]\n\n\
         COMMANDS:\n    create <name>       Create a persistent VM configuration\n    list                List configured VMs\n    inspect <name>     Show a VM configuration\n    start <name>       Run the built-in guest for a VM\n    hello               Run the first real KVM guest without saving a VM\n    capabilities        Check Linux KVM availability\n    destroy <name>     Delete a VM configuration\n    help                Show this help\n\n\
         CREATE OPTIONS:\n    --memory <MiB>      Guest RAM (default: 128)\n    --cpus <count>      vCPUs (default: 1)\n    --architecture <a>  x86_64 (the first milestone)\n\n\
         ENVIRONMENT:\n    RUSTBOX_HOME        Override the state directory (default: ~/.rustbox)\n\n\
         The first milestone runs a tiny x86-64 guest through Linux KVM.\n         It requires a Linux x86-64 host with /dev/kvm available."
    );
}

struct ConfigStore {
    root: PathBuf,
}

impl ConfigStore {
    fn discover() -> Result<Self, CliError> {
        let root = if let Some(path) = env::var_os("RUSTBOX_HOME") {
            PathBuf::from(path)
        } else if let Some(path) = env::var_os("HOME") {
            PathBuf::from(path).join(".rustbox")
        } else {
            return Err(CliError::Usage(
                "HOME is not set; use RUSTBOX_HOME to choose RustBox's state directory".to_owned(),
            ));
        };
        Ok(Self { root })
    }

    fn configs_dir(&self) -> PathBuf {
        self.root.join("config")
    }

    fn config_path(&self, name: &str) -> Result<PathBuf, CliError> {
        let config = VmConfig::new(name.to_owned())?;
        Ok(self.configs_dir().join(format!("{}.toml", config.name)))
    }

    fn save_new(&self, config: &VmConfig) -> Result<(), CliError> {
        config.validate()?;
        fs::create_dir_all(self.configs_dir())?;
        let path = self.config_path(&config.name)?;
        if path.exists() {
            return Err(CliError::Usage(format!(
                "VM {:?} already exists",
                config.name
            )));
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                if error.kind() == io::ErrorKind::AlreadyExists {
                    CliError::Usage(format!("VM {:?} already exists", config.name))
                } else {
                    CliError::Io(error)
                }
            })?;
        if let Err(error) = file
            .write_all(render_config(config).as_bytes())
            .and_then(|_| file.sync_all())
        {
            let _ = fs::remove_file(&path);
            return Err(CliError::Io(error));
        }
        Ok(())
    }

    fn load(&self, name: &str) -> Result<VmConfig, CliError> {
        let path = self.config_path(name)?;
        let contents = fs::read_to_string(&path).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                CliError::Usage(format!("VM {:?} does not exist", name))
            } else {
                CliError::Io(error)
            }
        })?;
        parse_config(&contents, name)
    }

    fn list(&self) -> Result<Vec<VmConfig>, CliError> {
        let directory = self.configs_dir();
        if !directory.exists() {
            return Ok(Vec::new());
        }
        let mut configs = Vec::new();
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension() != Some(OsStr::new("toml")) {
                continue;
            }
            let name = path
                .file_stem()
                .and_then(OsStr::to_str)
                .ok_or_else(|| CliError::Usage("configuration filename is not valid UTF-8".to_owned()))?;
            let contents = fs::read_to_string(&path)?;
            configs.push(parse_config(&contents, name)?);
        }
        configs.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(configs)
    }

    fn remove(&self, name: &str) -> Result<(), CliError> {
        let path = self.config_path(name)?;
        fs::remove_file(&path).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                CliError::Usage(format!("VM {:?} does not exist", name))
            } else {
                CliError::Io(error)
            }
        })?;
        Ok(())
    }
}

fn create_command(arguments: &[String]) -> Result<(), CliError> {
    let (name, memory_mib, cpus, architecture) = parse_create_arguments(arguments)?;
    let mut config = VmConfig::new(name)?;
    config.memory_mib = memory_mib;
    config.cpus = cpus;
    config.architecture = architecture;
    config.validate()?;

    let store = ConfigStore::discover()?;
    store.save_new(&config)?;
    println!("VM created: {}", config.name);
    println!("  memory: {} MiB", config.memory_mib);
    println!("  vCPUs:  {}", config.cpus);
    println!("  boot:   built-in hello guest");
    Ok(())
}

fn list_command(arguments: &[String]) -> Result<(), CliError> {
    if !arguments.is_empty() {
        return Err(CliError::Usage("list does not accept options".to_owned()));
    }
    let configs = ConfigStore::discover()?.list()?;
    if configs.is_empty() {
        println!("No VMs configured.");
        return Ok(());
    }
    println!("NAME\tSTATE\tMEMORY\tVCPUS");
    for config in configs {
        println!(
            "{}\tCreated\t{} MiB\t{}",
            config.name, config.memory_mib, config.cpus
        );
    }
    Ok(())
}

fn inspect_command(arguments: &[String]) -> Result<(), CliError> {
    let name = one_name(arguments, "inspect")?;
    let config = ConfigStore::discover()?.load(name)?;
    print_config(&config);
    Ok(())
}

fn destroy_command(arguments: &[String]) -> Result<(), CliError> {
    let name = one_name(arguments, "destroy")?;
    ConfigStore::discover()?.remove(name)?;
    println!("VM destroyed: {name}");
    Ok(())
}

fn start_command(arguments: &[String]) -> Result<(), CliError> {
    let name = one_name(arguments, "start")?;
    let config = ConfigStore::discover()?.load(name)?;
    run_configured_guest(config)
}

fn hello_command(arguments: &[String]) -> Result<(), CliError> {
    let (memory_mib, cpus) = parse_hello_arguments(arguments)?;
    let mut config = VmConfig::new("hello")?;
    config.memory_mib = memory_mib;
    config.cpus = cpus;
    run_configured_guest(config)
}

fn run_configured_guest(config: VmConfig) -> Result<(), CliError> {
    let hypervisor = KvmHypervisor::new().map_err(|error| CliError::Vmm(VmmError::Platform(error)))?;
    let capabilities = hypervisor.capabilities();
    if !capabilities.x86_64_guest {
        return Err(CliError::Usage(
            "the selected host cannot run an x86_64 guest".to_owned(),
        ));
    }
    println!("Starting {}...", config.name);
    println!("{} vCPU{}", config.cpus, if config.cpus == 1 { "" } else { "s" });
    println!("{} MiB RAM", config.memory_mib);
    let mut vm = VirtualMachine::new(VmId::new(), config, &hypervisor)?;
    let output = vm.run_builtin_guest()?;
    print!("{output}");
    io::stdout().flush()?;
    println!("VM stopped cleanly.");
    Ok(())
}

fn capabilities_command(arguments: &[String]) -> Result<(), CliError> {
    if !arguments.is_empty() {
        return Err(CliError::Usage("capabilities does not accept options".to_owned()));
    }
    match KvmHypervisor::new() {
        Ok(hypervisor) => {
            let capabilities = hypervisor.capabilities();
            println!("KVM API version: {}", hypervisor.api_version());
            println!("host architecture: {}", capabilities.host_architecture);
            println!("x86_64 guest: {}", capabilities.x86_64_guest);
            println!("guest memory: {}", capabilities.guest_memory);
            println!("vCPU: {}", capabilities.vcpu);
        }
        Err(error) => {
            println!("KVM unavailable: {error}");
            println!("RustBox's first milestone requires Linux x86-64 and /dev/kvm.");
        }
    }
    Ok(())
}

fn parse_create_arguments(
    arguments: &[String],
) -> Result<(String, u64, u32, Architecture), CliError> {
    let name = arguments
        .first()
        .ok_or_else(|| CliError::Usage("create requires a VM name".to_owned()))?
        .clone();
    let mut memory_mib = 128u64;
    let mut cpus = 1u32;
    let mut architecture = Architecture::X86_64;
    let mut index = 1usize;
    while index < arguments.len() {
        let option = &arguments[index];
        let value = arguments.get(index + 1).ok_or_else(|| {
            CliError::Usage(format!("option {option:?} requires a value"))
        })?;
        match option.as_str() {
            "--memory" => memory_mib = parse_u64(value, "memory")?,
            "--cpus" => cpus = parse_u32(value, "cpus")?,
            "--architecture" => {
                architecture = Architecture::from_str(value).map_err(CliError::Configuration)?
            }
            _ => return Err(CliError::Usage(format!("unknown create option {option:?}"))),
        }
        index += 2;
    }
    Ok((name, memory_mib, cpus, architecture))
}

fn parse_hello_arguments(arguments: &[String]) -> Result<(u64, u32), CliError> {
    let mut memory_mib = 128u64;
    let mut cpus = 1u32;
    let mut index = 0usize;
    while index < arguments.len() {
        let option = &arguments[index];
        let value = arguments.get(index + 1).ok_or_else(|| {
            CliError::Usage(format!("option {option:?} requires a value"))
        })?;
        match option.as_str() {
            "--memory" => memory_mib = parse_u64(value, "memory")?,
            "--cpus" => cpus = parse_u32(value, "cpus")?,
            _ => return Err(CliError::Usage(format!("unknown hello option {option:?}"))),
        }
        index += 2;
    }
    Ok((memory_mib, cpus))
}

fn parse_u64(value: &str, name: &str) -> Result<u64, CliError> {
    value
        .parse()
        .map_err(|_| CliError::Usage(format!("{name} must be a positive integer")))
}

fn parse_u32(value: &str, name: &str) -> Result<u32, CliError> {
    value
        .parse()
        .map_err(|_| CliError::Usage(format!("{name} must be a positive integer")))
}

fn one_name<'a>(arguments: &'a [String], command: &str) -> Result<&'a str, CliError> {
    match arguments {
        [name] => Ok(name),
        [] => Err(CliError::Usage(format!("{command} requires a VM name"))),
        _ => Err(CliError::Usage(format!("{command} accepts exactly one VM name"))),
    }
}

fn render_config(config: &VmConfig) -> String {
    format!(
        "# RustBox VM configuration\nformat_version = 1\nname = \"{}\"\nmemory_mib = {}\ncpus = {}\narchitecture = \"{}\"\nboot_guest = \"{}\"\nnetwork_mode = \"{}\"\n",
        config.name,
        config.memory_mib,
        config.cpus,
        config.architecture,
        config.boot.guest.as_deref().unwrap_or(""),
        config.network.mode,
    )
}

fn parse_config(contents: &str, expected_name: &str) -> Result<VmConfig, CliError> {
    let mut config = VmConfig::new(expected_name.to_owned())?;
    for (line_number, raw_line) in contents.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, raw_value) = line.split_once('=').ok_or_else(|| {
            CliError::Usage(format!("invalid configuration line {}", line_number + 1))
        })?;
        let key = key.trim();
        let value = parse_config_value(raw_value.trim()).map_err(|message| {
            CliError::Usage(format!("configuration line {}: {message}", line_number + 1))
        })?;
        match key {
            "format_version" => {
                if value != "1" {
                    return Err(CliError::Usage(format!(
                        "unsupported configuration format version {value:?}"
                    )));
                }
            }
            "name" => {
                if value != expected_name {
                    return Err(CliError::Usage(format!(
                        "configuration filename {expected_name:?} does not match name {value:?}"
                    )));
                }
            }
            "memory_mib" => config.memory_mib = value.parse().map_err(|_| {
                CliError::Usage(format!("configuration line {} has invalid memory_mib", line_number + 1))
            })?,
            "cpus" => config.cpus = value.parse().map_err(|_| {
                CliError::Usage(format!("configuration line {} has invalid cpus", line_number + 1))
            })?,
            "architecture" => config.architecture = Architecture::from_str(&value)?,
            "boot_guest" => {
                config.boot.guest = if value.is_empty() { None } else { Some(value) }
            }
            "network_mode" => config.network.mode = NetworkMode::from_str(&value)?,
            unknown => {
                return Err(CliError::Usage(format!(
                    "unknown configuration key {unknown:?} on line {}",
                    line_number + 1
                )))
            }
        }
    }
    config.validate()?;
    Ok(config)
}

fn parse_config_value(value: &str) -> Result<String, String> {
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        let inner = &value[1..value.len() - 1];
        if inner.contains('"') || inner.contains('\\') {
            return Err("quoted values may not contain quotes or escapes in the first format".to_owned());
        }
        return Ok(inner.to_owned());
    }
    if value.is_empty() {
        return Err("value cannot be empty".to_owned());
    }
    Ok(value.to_owned())
}

fn print_config(config: &VmConfig) {
    println!("Name:         {}", config.name);
    println!("State:        Created");
    println!("Architecture: {}", config.architecture);
    println!("Memory:       {} MiB", config.memory_mib);
    println!("vCPUs:        {}", config.cpus);
    println!(
        "Boot:         {}",
        config.boot.guest.as_deref().unwrap_or("none")
    );
    println!("Network:      {}", config.network.mode);
    println!("Disks:        {}", config.disks.len());
}
