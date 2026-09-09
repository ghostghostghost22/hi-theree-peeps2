# Testing strategy

RustBox treats guest-visible behavior as the important integration contract.
Tests are layered so fast checks remain useful while real KVM coverage is
explicit about host requirements.

## Unit tests

The current crates cover:

- VM ID formatting and parsing.
- Configuration validation and safe VM names.
- Strict lifecycle transitions.
- Page-aligned memory allocation.
- Cross-region memory reads and writes.
- Unmapped guest-address rejection.
- Normalized input exit mutation.
- UART output handling.
- Built-in guest instruction bytes.

Run them with:

```bash
cargo test --workspace
```

## Integration test target

On a Linux x86-64 host with `/dev/kvm`, the first end-to-end test should be
performed through the public CLI:

```bash
RUSTBOX_HOME="$PWD/.rustbox-test" cargo run -p rustbox-cli -- hello
```

The test passes only when the process prints `Hello from guest!` and exits
cleanly. This covers:

- KVM API initialization.
- VM creation.
- KVM memory registration.
- x86 real-mode register setup.
- Guest instruction execution.
- Port-I/O VM exits.
- UART dispatch.
- Guest HLT handling.

The repository does not claim this integration run was performed unless the
host actually provides KVM.

## Failure cases to preserve

Every new device or loader should add tests for:

- Empty and oversized input.
- Guest addresses at the first and last valid byte.
- Ranges crossing a region boundary or unmapped gap.
- Invalid descriptors and unsupported exit reasons.
- Host I/O failures and short operations.
- Repeated lifecycle operations.
- A guest that never halts, using an explicit exit budget or cancellation path.

A malformed guest must produce a VM error, not a RustBox process panic.

## Future guest tests

The next guest fixtures should be small, deterministic Rust or raw machine-code
programs with expected serial markers:

```text
BOOT_OK
MEMORY_OK
IO_OK
SHUTDOWN_OK
```

Later fixtures will cover a Linux kernel boot, persistent VirtIO block I/O,
networking, pause/resume, snapshots, and crash recovery. Long-running and
stress tests belong in a separate host-capable CI job so ordinary unit tests
remain fast and reproducible.
