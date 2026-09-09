# Platform support matrix

The matrix distinguishes implemented behavior from planned architecture. A
platform is not marked supported until it can create, execute, and stop a real
vCPU guest through the public machine path.

| Feature | Linux x86-64 / KVM | Windows / WHPX | macOS / HVF |
| --- | --- | --- | --- |
| CLI configuration | Implemented | Builds with diagnostic | Builds with diagnostic |
| Platform capability query | Implemented | Planned | Planned |
| x86-64 guest | First milestone | Planned | Planned |
| Guest RAM | Implemented | Planned | Planned |
| vCPU abstraction | Implemented | Contract only | Contract only |
| Built-in UART guest | Implemented | Planned | Planned |
| Linux kernel loader | Planned | Planned | Planned |
| Raw block device | Planned | Planned | Planned |
| VirtIO block/network | Planned | Planned | Planned |
| Snapshots | Planned | Planned | Planned |
| GUI | Planned | Planned | Planned |

The platform crate owns each host API. The VMM and management surfaces should
only depend on the platform-neutral traits, so a missing row is an explicit
capability gap rather than an accidental host assumption.
