# RustBox

RustBox is a Rust-native virtual-machine project. It is being developed in the
order required by the virtualization engine rather than starting with a GUI:

```text
CLI / future GUI
      |
VM management API
      |
Machine / VMM
      |
CPU + guest memory + devices
      |
Linux KVM (first backend)
```

The repository currently contains the first end-to-end milestone: a tiny
x86-64 guest is loaded into page-aligned guest RAM, executed by a real Linux
KVM vCPU, and observed through a UART I/O exit. The expected output is:

```text
Hello from guest!
```

This milestone is intentionally small. It proves that the project can create a
VM, register memory, initialize a vCPU, execute guest instructions, handle
port-I/O exits, and stop on a guest `HLT`. It does not claim to boot Linux or
provide a general-purpose VM product yet.

## Current scope

- 100% application source in Rust.
- Linux x86-64 host only.
- KVM through the maintained Rust `kvm-ioctls` and `kvm-bindings` crates.
- Checked, page-aligned guest memory with multiple-region support.
- Platform-independent VM lifecycle, configuration, vCPU, and exit contracts.
- A minimal COM1 UART device model.
- A dependency-light CLI for VM configuration and the first integration guest.

The Windows/WHPX and macOS/Hypervisor.framework backends are intentionally not
pretended to work yet. The public platform traits keep those additions from
leaking into the VMM or CLI.

## Requirements

- Rust 1.75 or newer.
- Linux x86-64.
- CPU virtualization enabled in firmware.
- A usable `/dev/kvm` device. The current user generally needs to be in the
  `kvm` group.

On Debian/Ubuntu-like systems, the host setup is commonly equivalent to:

```bash
sudo apt install build-essential pkg-config
sudo usermod -aG kvm "$USER"
# Log out and back in after changing group membership.
```

The exact package and permission setup is distribution-specific. RustBox reports
an actionable error instead of panicking when `/dev/kvm` cannot be opened.

## Build and test

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo build --release
```

The automated unit tests do not require `/dev/kvm`. The end-to-end commands do.
They should be run on a Linux x86-64 host with KVM enabled.

## CLI quick start

Use `RUSTBOX_HOME` to keep development state outside your home directory when
needed:

```bash
export RUSTBOX_HOME="$PWD/.rustbox"

cargo run -p rustbox-cli -- help
cargo run -p rustbox-cli -- capabilities
cargo run -p rustbox-cli -- create hello-vm --memory 128 --cpus 1
cargo run -p rustbox-cli -- list
cargo run -p rustbox-cli -- inspect hello-vm
cargo run -p rustbox-cli -- start hello-vm
cargo run -p rustbox-cli -- destroy hello-vm
```

For an ephemeral integration run without creating a configuration:

```bash
cargo run -p rustbox-cli -- hello
```

Configurations are stored as a deliberately small, human-readable subset of
TOML under `$RUSTBOX_HOME/config/` (or `~/.rustbox/config/`). The format is
versioned by the application contract and will gain kernel, disk, and network
sections as those features become executable.

## Workspace layout

```text
crates/
├── rustbox-core       VM IDs, configuration, architecture, lifecycle
├── rustbox-memory     checked guest physical memory and host mappings
├── rustbox-cpu        normalized vCPU and exit interfaces
├── rustbox-platform   KVM backend behind platform-neutral traits
├── rustbox-vmm        machine orchestration and UART device model
└── rustbox-cli        initial CLI and configuration store

docs/
├── architecture.md    dependency direction and milestone boundaries
├── testing.md         validation strategy
└── adr/               architectural decisions
```

The intended dependency direction is:

```text
rustbox-cli -> rustbox-vmm -> rustbox-platform -> KVM
                          \-> rustbox-cpu / rustbox-memory -> rustbox-core
```

The GUI and future daemon should consume the management API rather than
accessing KVM directly.

## Roadmap

The next milestones are deliberately ordered around observable behavior:

1. Add a real Linux kernel loader, boot parameters, and serial console.
2. Add interrupt/timer foundations and a raw block device.
3. Add VirtIO block and persistent disk integration tests.
4. Add NAT networking and VirtIO-net.
5. Add a daemon/API, multiple VM ownership, snapshots, and crash recovery.
6. Add the GUI only after the engine and CLI are demonstrably useful.
7. Add WHPX and HVF backends behind the existing platform contracts.

Features such as GPU passthrough, USB passthrough, live migration, and a custom
UEFI implementation are outside the first release target.
