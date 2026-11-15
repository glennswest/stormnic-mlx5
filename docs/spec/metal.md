# What the first metal runs settle (#1)

The spec notes (`cmd.md` §9, `datapath.md` §10, `steering.md` §9, `fec.md`,
`linkdiag.md`) list what the PRM leaves open as **HW-CHECK**. This page says
which of them the first boots on real cards answer, and how.

## The runs

| Run | Machine | Card | Driver | Result |
|---|---|---|---|---|
| serial 12 (2026-11-14) | C2NR0Q2 (R230), forge1 (R6415) | ConnectX-4 Lx 15b3:1015, both functions | 32ddb3f | binds after the take-over from FlexBoot, PPLM/PDDR read and written, links up running RS; no lease (#12) |
| serial 14 (2026-11-14 20:58Z, 21:02Z) | C2NR0Q2, forge1 | same | 9d86045 | lease, claim, NVMe/TCP attach, bootloader started, on both |
| forge1 02:24Z 2026-11-15 | forge1 | same, :80 and :81 both linked | 9d86045 | both ports `25GBASE-SR 25g … running rs, link up`; leased on :80; the kernel ran (stormcos formatting its disk) |

Consoles: stormnic-mlx5#1 and #8 (C2NR0Q2's syslog, forge1's serial log on
stormcentral). Those boots are not verbose, so they show the one line per
port and the result, not the trace's values.

## Settled by a working bring-up

Each answer below is "the driver's reading works on a ConnectX-4 Lx": the
command was accepted and the next step, which depends on it, worked. A
misreading would have failed bind (every command checks its status) or
the lease (every frame in and out goes through the rings).

### cmd.md §9

| # | Answer |
|---|---|
| 3 | Init-segment MMIO is big-endian, byte-swapped on x86, as assumed: the queue address, `initializing` and every doorbell work. |
| 6 | A function FlexBoot left running comes back with the driver's recovery: the take-over (`taking it from Mellanox ConnectX Driver`) binds on all four functions. Whether ENABLE_HCA refused first is in the trace only. |
| 8 | The entry and mailbox signatures as computed are accepted (or not checked): every mailbox command (pages, caps, contexts, ACCESS_REG) completes. |
| 9 | 1 KiB-aligned mailbox blocks and a 4 KiB queue page are accepted. |
| 10 | The ISSI choice from `supported_issi` is accepted by SET_ISSI. |
| 11 | QUERY_PAGES' sign and MANAGE_PAGES' input as read (Table 133) are right: boot and init pages are given and INIT_HCA succeeds. |
| 12 | Page requests are met: bring-up and the datapath run with the one EQ. |
| 13 | No SET_HCA_CAP is needed: the defaults as queried work. |
| 14 | The reserved LKey (QUERY_SPECIAL_CONTEXTS, `rlkey`) is usable on a bare-metal PF: no MKey is made and DMA in both directions works. |
| 15 | ACCESS_REG READ with `local_port` 1 in the input reads this function's port: PTYS, PAOS, PPLM and PDDR give each function's own media, module and FEC. |
| 16 | The command lengths the driver sends (ENABLE/DISABLE_HCA 16 bytes) are accepted. |

Open (values in the trace only): 1, 2, 4, 5, 7 (how long `initializing`
stayed set is traced), 17. A command's real completion time under
ExitBootServices is #15.

### datapath.md §10

| # | Answer |
|---|---|
| 1 | The UAR index ALLOC_UAR returns does not collide with the init segment: send doorbells reach the SQ. |
| 3 | The 8-byte send doorbell alternating 800h/900h works. |
| 4 | Writing the SQ counter in both halves of the doorbell record works (which half is read stays open). |
| 5 | SQ `log_wq_stride` 6 (64-byte WQEBB) is accepted and sends complete. |
| 7 | As cmd.md 14: the reserved LKey works for the SQ and RQ. |
| 8 | The order WQE → doorbell record → doorbell with the driver's fences carries every frame of a DHCP exchange and an NVMe/TCP attach. |
| 9, 10 | `page_offset` 0 and 4 KiB PAS pages are accepted. |
| 13 | Send completions are matched to WQEs as assumed (sends are reclaimed; the SQ never fills). |
| 18 | ExitBootServices' quiesce returns on forge1 (the kernel ran after the 02:24Z boot); its bounds are #15. |
| 20 | `c_eqn` naming the one EQ is accepted for both CQs. |

Open: 2, 6, 11, 12, 14, 15, 16, 17, 19 (values or cases these boots do
not show).

### steering.md §9

| # | Answer |
|---|---|
| 1 | The match-all entry's `match_criteria_enable` as the driver sends it is accepted, and frames reach the TIR. |
| 4 | With the vport promiscuous, a PF with no e-switch gets the broadcast and unicast DHCP needs and the TCP stream NVMe/TCP needs. |
| 6 | The vport state as queried and set lets traffic through with the port up. |
| 7 | The NIC RX table carries traffic (root by default or by SET_FLOW_TABLE_ROOT when `modify_root`). |

Open: 2, 3, 5, 8, 9, 10, 11 (promiscuous mode hides 2, 3 and 5).

### fec.md, linkdiag.md

`fec.md` HW-CHECK 5 is answered there (#8). `linkdiag.md`'s lines are traced
only on a linked port, so they wait on the verbose forge1 boot of #13.
