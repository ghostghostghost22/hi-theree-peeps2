# ADR 0002: Rust-only application source

- Status: accepted
- Date: 2026-09-08

## Context

The project goal is a Rust-native VM manager. Host hypervisor APIs and guest
artifacts necessarily originate outside the application, so “only Rust” must
refer to application source and its Rust interfaces to the host.

## Decision

All RustBox application code is Rust. Host virtualization APIs may be accessed
through Rust FFI bindings or Rust crates that wrap the documented operating
system interface. Firmware images, kernels, disk images, and the host kernel
are external data or platform resources, not application-language exceptions.

## Consequences

- No custom C/C++ helper library or scripting runtime is part of the product.
- Platform bindings remain isolated in `rustbox-platform`.
- Dependency additions must be justified and kept out of the VMM critical path
  where the standard library or an existing crate is sufficient.
