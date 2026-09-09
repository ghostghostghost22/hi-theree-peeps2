# RustBox architecture

## Goal

RustBox is a small, Rust-native virtual-machine manager. The first deliverable
is not a visual shell; it is proof that a guest can execute correctly inside a
hardware-assisted VM and communicate with the host through a modeled device.

## Dependency direction

The dependency direction is intentionally one-way:

```text
CLI / future GUI
        |
management API / future daemon
        |
VMM machine model
        |
CPU, memory, device contracts
        |
platform backend
        |
KVM / WHPX / Hypervisor.framework
```

The current workspace represents the lower part of that graph:

- `rustbox-core` contains no host-specific code. It owns `VmId`, `VmConfig`,
  architecture names, errors, and the strict lifecycle state machine.
- `rustbox-memory` owns page-aligned guest allocations and checked guest
  physical accesses. The KVM backend consumes borrowed region descriptors but
  does not own guest RAM.
- `rustbox-cpu` defines normalized exits, registers, boot state, and the vCPU
  callback contract. It does not mention KVM.
- `rustbox-platform` owns the KVM-specific translation from host exits and
  registers to the CPU contract. Unsupported hosts return diagnostics rather
  than exposing fake capabilities.
- `rustbox-vmm` creates the machine, registers memory, creates vCPUs, dispatches
  exits to devices, and enforces the VM lifecycle.
- `rustbox-cli` is a thin configuration and management surface. It never
  imports KVM types.

## First machine

The first machine contains:

1. One page-aligned RAM region beginning at guest physical address `0`.
2. One x86-64 KVM vCPU initialized in real mode.
3. A tiny guest program that writes a known string to COM1 and halts.
4. A UART device model that consumes `OUT DX, AL` exits.
5. A bounded run loop so a broken guest cannot spin forever in the CLI.

The host-visible success condition is the complete serial string:

```text
Hello from guest!
```

This external behavior is more valuable than a test of an individual ioctl
wrapper: it exercises memory registration, CPU setup, instruction execution,
exit normalization, device dispatch, and shutdown together.

## Memory ownership

`GuestMemory` owns its allocations for the entire lifetime of the platform VM.
The KVM `KVM_SET_USER_MEMORY_REGION` registration stores host addresses, so the
VMM must not move, resize, or drop guest memory before dropping the KVM VM. The
current structure makes that ordering explicit: `VirtualMachine::drop`
clears vCPUs and takes the backend before the memory field is automatically
dropped.

All public guest-memory read and write operations validate the complete range.
A range cannot silently cross an unmapped gap. Hypervisor registration also
requires page-aligned bases and sizes.

## Lifecycle

The domain state machine rejects illegal operations:

```text
Created -> Starting -> Running <-> Paused
                              |
                              v
                           Stopping -> Stopped
```

Failures from active states transition to `Crashed`. A future manager may reset
resources from there before allowing a new start. The CLI currently reports
configuration records as `Created`; persistent runtime state will move into the
daemon once multiple long-lived VMs are supported.

## Platform abstraction

`HypervisorBackend` creates an opaque `VirtualMachineBackend`. The VMM can map
memory and create a `VirtualCpu` without knowing whether the implementation is
KVM, WHPX, or HVF. KVM exits are converted to owned normalized payloads before
being handed to a device model. For input exits, the device mutates the payload
and the KVM implementation copies the response back into the host exit buffer.

The first KVM implementation intentionally supports only x86-64. Adding an
architecture requires an explicit `Architecture`, `BootState`, register, and
platform capability path; it must not be inferred from host-specific casts.

## Error boundaries

- Configuration errors are rejected before allocating guest memory.
- Memory errors include the guest address and requested length.
- Platform errors preserve the failing host operation and provide a useful
  `/dev/kvm` diagnostic.
- Unsupported guest exits fail the VM rather than being ignored.
- No production VMM path uses `unwrap()` for guest-controlled values or host
  operations.

## What is intentionally not here yet

The first milestone does not provide a Linux kernel loader, firmware, PCI,
interrupt controllers, timers, storage, networking, snapshots, a daemon, or a
GUI. Each of those will be added behind a testable boundary after the previous
observable milestone is stable.
