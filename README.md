# stormnic-mlx5

A Rust `no_std` UEFI Simple Network Protocol (SNP) driver for **Mellanox mlx5 (ConnectX-4/4 Lx/5/6)** (15b3:1013, 1015, 1017, 1019, 101B, 101D, 101F), loaded by
[stormbootx](https://github.com/glennswest/stormbootx) so it can claim and boot images over these NICs without iPXE.
A sibling of stormnic-ixgbe, stormnic-mlx4 and stormnic-virtio, built and pinned the same way (stormbootx `build-nic-drivers.sh`).

Status: not started (P3, paused; owner 2026-10-07).

## Licence

MIT (see LICENSE). NOTICE acknowledges the sources the hardware facts were learned from; no code was copied.
