# stormnic-mlx5

A UEFI driver, in Rust (`no_std`), that gives firmware an
`EFI_SIMPLE_NETWORK_PROTOCOL` for **Mellanox mlx5 Ethernet: ConnectX-4,
ConnectX-4 Lx, ConnectX-5, ConnectX-6**. stormbootx loads it from
`\stormboot\drivers` on its rustnic media so it can boot over these NICs
without iPXE or a vendor option ROM. It is a sibling of stormnic-ixgbe,
stormnic-mlx4 and stormnic-virtio, built and pinned the same way (stormbootx
`scripts/build-nic-drivers.sh`).

## Why it exists

stormbootx runs its own TCP/IP stack (smoltcp) on each NIC's SNP. Cheap
ConnectX-4 Lx cards often carry no FlexBoot UEFI driver, so the firmware
offers no SNP for them. The first target is the Dell R230 test machine
(C2NR0Q2), whose PCIe slot holds a ConnectX-4 Lx (MT27710 family,
15b3:1015) (#1): with this driver it boots stormbootx from a USB stick, as
the X9 blades do, with no iDRAC virtual media and no option ROM.

## Hardware

| PCI ID | Family | Run on |
|---|---|---|
| 15b3:1013 | ConnectX-4 (MT27700) | not yet |
| 15b3:1015 | ConnectX-4 Lx (MT27710) | not yet (the Dell is the first target) |
| 15b3:1017 | ConnectX-5 (MT27800) | not yet |
| 15b3:1019 | ConnectX-5 Ex (MT28800) | not yet |
| 15b3:101b | ConnectX-6 (MT28908) | not yet |
| 15b3:101d | ConnectX-6 Dx (MT2892) | not yet |
| 15b3:101f | ConnectX-6 Lx (MT2894) | not yet |

Physical functions only, checked against pci.ids (2026.10.09). Each port of
an mlx5 card is its own PCI function, so each gets its own SNP.

**Written from the public PRM only:** *Mellanox Adapters Programmer's
Reference Manual (PRM) Supporting ConnectX-4 and ConnectX-4 Lx*, Rev 0.40
(MLNX-15-4845), which NVIDIA publishes. `docs/spec/` condenses what the
driver uses, with PRM section and table numbers and the points the PRM leaves
open (HW-CHECK):

- [`docs/spec/cmd.md`](docs/spec/cmd.md): initialization segment, command
  queue and mailboxes, bring-up/teardown, capabilities, port registers.
- [`docs/spec/datapath.md`](docs/spec/datapath.md): UAR doorbells, EQ, CQ,
  SQ, RQ, TIS, TIR, WQE and CQE formats.
- [`docs/spec/steering.md`](docs/spec/steering.md): NIC RX flow tables and
  the NIC vport context.

Nothing is taken from the Linux mlx5 driver or iPXE's golan/flexboot (GPL);
the crate is MIT ([`NOTICE`](NOTICE)).

## How it works

- `src/cmd.rs`: one command-queue entry, polled, with mailbox chains and
  signatures (PRM 7.14). No allocation after setup, so ExitBootServices can
  still send commands.
- `src/hca.rs`: PRM 8.2 bring-up: the queue address, `initializing`,
  ENABLE_HCA, QUERY/SET_ISSI, boot and init pages (QUERY/MANAGE_PAGES),
  QUERY_HCA_CAP (general, Ethernet, NIC flow table), INIT_HCA,
  SET_DRIVER_VERSION, ALLOC_UAR/PD/TRANSPORT_DOMAIN and the reserved LKey
  (QUERY_SPECIAL_CONTEXTS, physical addressing, no MKey). PRM 8.3 teardown:
  TEARDOWN_HCA, every page reclaimed, DISABLE_HCA.
- `src/eth.rs`: the vport (admin up, permanent MAC, own MAC in the unicast
  list, promiscuous), one EQ for page requests, port changes and CQ errors,
  a send CQ/SQ behind a TIS, a receive CQ/RQ (256 × 2 KiB) behind a direct
  TIR with self-loopback off, and a NIC RX flow table whose match-all entry
  forwards to the TIR. Link and speed from PAOS/PTYS. All polled.
- `src/snp.rs`: the SNP on a child handle with a MAC device path.

## Build

`x86_64-unknown-uefi`, built on a build VM with `sc-build` after pushing:

```bash
sc-build scripts/check.sh
```

That runs the locked release build, clippy with warnings as errors, the PE
subsystem check (11, boot-service driver), a `--features verbose` build and
the host tests. `Cargo.lock` is committed; builds use `--locked`. The image
is meant to be byte-reproducible like stormnic-mlx4's (`scripts/rustc-remap.sh`,
`/Brepro`; `sc-build scripts/repro-check.sh` checks it).

## Interfaces and configuration

- **Installs:** `EFI_DRIVER_BINDING_PROTOCOL` on its image handle (version
  0x10). For each started function, a child handle with
  `EFI_DEVICE_PATH_PROTOCOL` (PCI path + MAC node, IfType 1) and
  `EFI_SIMPLE_NETWORK_PROTOCOL` (revision 0x00010000).
- **SNP:** MaxPacketSize 1500, MediaHeaderSize 14, receive filters unicast,
  broadcast and up to 16 multicast addresses, applied in software (the
  hardware delivers every frame the vport takes), MAC not changeable,
  `MediaPresent` from port-change events and PAOS, `WaitForPacket`,
  `MCastIpToMac`. VLAN tags are kept. Own frames are not looped back (TIR
  self-loopback off); one that is, is dropped and logged once.
  `Statistics`, `StationAddress` and `NvData` return `UNSUPPORTED`.
- **Consumes:** `EFI_PCI_IO_PROTOCOL` (`BY_DRIVER`, and `BY_CHILD_CONTROLLER`
  per child): config space, BAR 0 reads/writes (32- and 64-bit), DMA common
  buffers below 4 GiB, attributes. While started it keeps memory space, bus
  master and dual-address-cycle on; `Stop` puts the original attributes back.
- **ExitBootServices:** TEARDOWN_HCA (panic close, which stops bus mastering)
  and DISABLE_HCA, then bus master off. Nothing is freed or printed.
- **A platform driver wins:** a NIC with an SNP already, or whose PCI I/O is
  held `BY_DRIVER`, is left alone.
- **Configuration:** one switch, console output only: the EFI variable
  `StormnicVerbose` under vendor GUID `ce1479a2-eab9-4176-b0ad-c909ea5b8e0b`
  (shared by every stormnic driver), read once at the entry point; a first
  byte other than 0 prints the bring-up trace. `--features verbose` does the
  same at build time.

## Console output

Quiet by default: one line per NIC,

```
stormnic-mlx5 0.1.0: 0000:02:00.0 15b3:1015 ConnectX-4 Lx port 1: MAC xx:xx:xx:xx:xx:xx, link up 10G, SNP installed
```

plus warnings and failures. A failure first replays the last 16 trace lines.
Verbose prints every command (`    INIT_HCA (0x0): ok`), the capabilities, the
pages lent, the vport state and the data-path object numbers.

## Tests

`scripts/test-host.sh` runs the host tests of the UEFI-free code
(`test/layout.rs`: bit fields, command header, entry and mailbox signatures,
send WQE construction, CQE/EQE ownership, PTYS speeds; `test/trace.rs`: the
replay ring).

On metal: only the Dell (C2NR0Q2) has an mlx5 card, and no pve VM can present
one. A run boots the rustnic medium (or a stick) on the Dell with this driver,
verbose; the master does the boot. The HW-CHECK lists in `docs/spec/` say
what that first run must answer.

## Licence

MIT (see LICENSE). NOTICE acknowledges the PRM.
