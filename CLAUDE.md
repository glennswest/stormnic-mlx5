# CLAUDE.md — stormnic-mlx5

A `no_std` UEFI driver giving firmware an `EFI_SIMPLE_NETWORK_PROTOCOL` for
Mellanox ConnectX-4 / 4 Lx / 5 / 6 Ethernet (the Linux `mlx5` family). It is
loaded by stormbootx from `\stormboot\drivers` on its rustnic media, on
machines whose firmware has no UEFI driver for the NIC. The first target is
the Dell R230 test machine (C2NR0Q2), which carries a ConnectX-4 Lx
(MT27710 family, 15b3:1015) and no FlexBoot UEFI (#1). Read README.md first.

Read the cross-project rules in `../CLAUDE.md` first. In particular, **build
with `sc-build` after pushing, never on this VM and never as root**, and
scratch files go in `tmp/`.

## Rules for this crate

- **Written from the public PRM only:** *Mellanox Adapters Programmer's
  Reference Manual (PRM) Supporting ConnectX-4 and ConnectX-4 Lx*, Rev 0.40,
  MLNX-15-4845 (2016), published by NVIDIA at
  `https://network.nvidia.com/files/doc-2020/ethernet-adapters-programming-manual.pdf`.
  Cite PRM sections/tables in code (`PRM 7.14.1`, `PRM Table 95`). Never
  translated from Linux mlx5 or iPXE golan/flexboot (GPL); the crate is MIT.
  `NOTICE` acknowledges the PRM. Download it into `tmp/prm/` to read it (it
  is not committed). Rev 0.40 has no opcode for anything after chapter 12;
  anything it does not settle is a hardware check (docs/spec/connectx4.md §HW).
- It is a *driver*: an EFI boot-service driver (subsystem 11) that installs
  `EFI_DRIVER_BINDING_PROTOCOL`. stormbootx loads it, then connects controllers.
- A platform's own driver must win: Supported tries a `BY_DRIVER` open of the
  NIC's PciIo and declines if anyone holds it.
- Console (`src/console.rs`, as stormnic-mlx4#16 / stormnic-ixgbe#22):
  `trace!` for the bring-up trace (printed only when verbose: EFI variable
  `StormnicVerbose` or `--features verbose`; else kept in a 16-line ring that
  a failure replays), `say!` for the one line per NIC and every warning,
  `alarm!` for failures. Never `uefi::println!` directly.

## Build

```bash
sc-build 'cargo build --locked --release --target x86_64-unknown-uefi && scripts/pe-subsystem.sh "${CARGO_TARGET_DIR:-target}"/x86_64-unknown-uefi/release/stormnic-mlx5.efi && scripts/test-host.sh'
```

`Cargo.lock` is committed; keep `--locked`. Zero warnings: `[lints.rust]
warnings = "deny"` and clippy `-D warnings` (`scripts/check.sh`).

## How it ships

No golden of its own. stormbootx's `scripts/build-nic-drivers.sh` builds it
from a pinned `STORMNIC_MLX5_REF` (stormbootx#73), and the
`stormbootx-rustnic` media carries `stormnic-mlx5.efi`.

## Test

Host tests: `scripts/test-host.sh` (UEFI-independent decoding: mailbox
signatures, PAS layout, CQE/opcode decoding, PTYS speeds).

Hardware: only the Dell (C2NR0Q2) has the card; no pve VM can present it.
A run is booting the rustnic medium (or a stick) on the Dell — the master's
job, coordinated on the issue (the Dell is held for shelf work, but a
stormbootx boot test does not touch the shelf).

## Version

`Cargo.toml` → `package.version`. Current: `v0.1.0` (unreleased).

## Work plan

#1 — bind, link, transmit/receive; pinned in stormbootx's nic-drivers (P1, owner 2026-10-10).

- [ ] Crate scaffold from stormnic-mlx4's shape (binding, PciIo, DMA, console/trace, verbose switch,
      reproducible build), PCI IDs checked against pci.ids.
- [ ] Command interface (PRM 7.14, 8.2): init segment, cmdq, mailboxes + signatures, polling.
- [ ] Bring-up: ENABLE_HCA, ISSI, pages (QUERY/MANAGE_PAGES), QUERY/SET_HCA_CAP, INIT_HCA, teardown.
- [ ] Objects: UAR, PD, transport domain, MKey (PA, reserved lkey), EQ (pages/port events), CQ×2, TIS, SQ, RQ, TIR,
      NIC RX flow table + group + match-all entry → TIR, vport context (MAC, promisc), PAOS/PTYS link.
- [ ] SNP on a child handle: Transmit/Receive/GetStatus, ExitBootServices teardown.
- [ ] sc-build passes (subsystem 11, no warnings, host tests).
- [ ] Spec notes `docs/spec/connectx4.md` (which PRM tables each step uses, HW-checks).
- [ ] Pin request in stormbootx (comment on stormbootx#73); hardware boot on the Dell via the master.
