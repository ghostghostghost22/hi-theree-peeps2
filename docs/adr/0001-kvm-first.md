# ADR 0001: Start with Linux KVM

- Status: accepted
- Date: 2026-09-08

## Context

RustBox is intended to become a cross-platform VM manager, but implementing
Linux/KVM, Windows/WHPX, and macOS/Hypervisor.framework at the same time would
make the first execution milestone difficult to diagnose. A GUI-first approach
would also risk producing a convincing shell without a working VM engine.

## Decision

The first backend targets Linux x86-64 KVM. The application remains Rust-only,
while using the host's documented virtualization interface through maintained
Rust bindings. Platform-independent traits live below the VMM so later WHPX and
HVF implementations do not change the GUI or management contracts.

## Consequences

- The first integration test requires `/dev/kvm`.
- The initial guest architecture is x86-64.
- Unsupported hosts report a clear diagnostic rather than pretending to run.
- Linux-specific work can be validated before portability work begins.
- The VMM must avoid importing KVM types outside `rustbox-platform`.
