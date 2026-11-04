# Changelog

## [Unreleased]

### 2026-11-04
- **docs:** Refreshed from the code since 2026-10-14: README, `docs/spec/datapath.md` and CLAUDE.md already match the
  10 KiB receive buffers, software drop over 1518, PMTU trace and the FlexBoot decline; README line wrap fixed. No
  promise without code found, so no issue filed.
- **fix:** Receive buffers are 10 KiB, and a frame over 1518 bytes is dropped in software and logged once. The Dell's
  network is jumbo and the vport promiscuous: a frame over the 2 KiB buffer was a Local_Length error, which takes the RQ
  to ERR for good. PMTU goes to the trace for the first hardware run (#1).
- **fix:** The image is byte-reproducible: link with `/DEBUG:NONE`, as stormbootx's nic-drivers build does (#6).
- **docs:** The R230's ConnectX-4 Lx carries FlexBoot, so the driver binds there only with stormbootx#131 or
  stormcentral#552's BIOS job (#2).

### 2026-10-10
- **feat:** UEFI SNP driver for mlx5 (#1): driver binding for 15b3:1013/1015/1017/1019/101b/101d/101f (PFs, checked
  against pci.ids 2026.10.09), command queue (PRM 7.14), HCA bring-up and teardown (PRM 8.2/8.3), one SQ/RQ pair
  with a direct TIR and a match-all NIC RX flow entry, EFI_SIMPLE_NETWORK_PROTOCOL on a child handle.
- **docs:** CLAUDE.md work plan; `docs/spec/` notes from the public ConnectX-4 PRM Rev 0.40.

### 2026-10-07
- **chore:** repository created (owner: low priority, paused)
