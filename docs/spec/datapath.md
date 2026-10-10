# Data path: EQ, CQ, SQ, RQ, TIS, TIR, WQEs, doorbells

Source: *Mellanox Adapters Programmer's Reference Manual (PRM) Supporting
ConnectX-4 and ConnectX-4 Lx, Rev 0.40* only. "§" = PRM section, "T" = PRM
table number, "p" = PDF page. Nothing here comes from any other driver.
Anything the PRM leaves open is marked **HW-CHECK** (collected in §10).

Conventions (as everywhere in this spec): every structure is a sequence of
big-endian 32-bit dwords (PRM "Byte Endianness", p25: bits 31-24 are at byte
offset 0, bits 7-0 at byte offset 3); "off" is the byte offset of the dword;
bits are numbered 31..0 within that dword. Offsets are relative to the start
of the structure named in the heading unless marked *abs* (absolute offset in
the command input/output data, i.e. from the opcode dword). Fields not listed
are reserved and must be written as 0. Bit positions were taken from the
"Field Descriptions" tables; where only the layout drawing gives a position
(CQE, error CQE, doorbell record) it was measured from the PDF drawing and
says so.

Goal of this driver: one PF, polling only (no interrupts, nothing ever
armed), boot-time only. Objects: 1 UAR page, 1 EQ (page requests + port
change), 1 send CQ, 1 receive CQ, 1 TIS, 1 SQ, 1 cyclic RQ, 1 direct TIR.

---

## 1. UAR — doorbell pages (§7.2, T13, T14)

### 1.1 Where a UAR page is

| Topic | PRM | Ref |
|---|---|---|
| BAR | The UAR is one BAR per function containing multiple pages ("HCA BAR"; the initialization segment is at offset 0 of the same BAR). | §7.2.1, Fig 11, §4.3 |
| Page size | "should match the CPU (system) page size, and should be configured at device initialization": `HCA_CAP.log_uar_page_sz` (off 90h 15:0, RW), log2 of the UAR page **in 4 KB units** → 0 for 4 KB on x86. | §7.2.1, T144/T145 |
| UAR index | `ALLOC_UAR` (opcode 0x802) output off 08h bits 23:0 `uar`. `DEALLOC_UAR` (0x803) input off 08h 23:0. | §12.3.15/16, T201–T208 |
| Address | Fig 11 shows UAR page *i* as the *i*-th page of the BAR, so **UAR page address = BAR base + uar × (4096 << log_uar_page_sz)**. The PRM shows this only as a figure. | Fig 11 |
| Area size | `HCA_CAP.uar_sz` (off 48h 21:16): UAR area = 1 MB × 2^uar_sz. | T144 |
| Overlap | The initialization segment occupies BAR offsets 0000h–~4010h (T7). PRM does not say which UAR indices ALLOC_UAR can return or that they avoid the init segment — **HW-CHECK** (assert `uar × page ≥ 4100h`, or at least ≠ the init segment). | T7 |

### 1.2 UAR page format (T13)

| off (in page) | bits | name | meaning |
|---|---|---|---|
| 000h–01Ch | — | reserved | |
| 020h | 29:28 | `cmdsn` | CQ doorbell: command sequence number (T14: "= (num_of_completion_event_delivered + 1) % 4"; 0 for the first ring) |
| 020h | 24 | `cmd` | 0 = request notification for next solicited **or** unsolicited completion; 1 = next solicited completion only (T14) |
| 020h | 23:0 | `cq_ci` | CQ consumer counter at the time of the ring |
| 024h | 23:0 | `cq_n` (T14 `cqn`) | CQ number |
| 028h–03Ch | — | reserved | |
| 040h | 31:24 | `eqn` | EQ number — **update CI and arm** |
| 040h | 23:0 | Consumer Index | EQ consumer counter |
| 044h | — | reserved | |
| 048h | 31:24 | `eqn` | EQ number — **update CI only (no arm)** |
| 048h | 23:0 | Consumer Index | EQ consumer counter |
| 04Ch–7FCh | — | reserved | |
| 800h–8FCh | — | `DB_BlueFlame_Buffer0_even` | BlueFlame register 0, even buffer (T13 prints "800-0FCh" — typo) |
| 900h–9FCh | — | `DB_BlueFlame_Buffer0_odd` | BlueFlame register 0, odd buffer |
| A00h–AFCh | — | `DB_BlueFlame_Buffer1_even` | BlueFlame register 1, even buffer |
| B00h–BFCh | — | `DB_BlueFlame_Buffer1_odd` | BlueFlame register 1, odd buffer |

Bit positions for 020h/040h/048h are from the T13 drawing; T14 gives the
widths (cmdsn 2, cmd 1, cqn 24, cq_ci 24) and agrees with the CQ doorbell
record T70 (cmd_sn 29:28, cmd 24, arm_ci 23:0).

BlueFlame geometry is reported by `HCA_CAP.bf` (off 4Ch bit 31),
`log_bf_reg_size` (4Ch 20:16, "log2 of BlueFlame max register size in bytes"
— one register = 2 buffers), `log_max_bf_regs_per_page`, `log_max_bf_pages`
(§7.2.2.1). T13 implies 512-byte registers (two 256-byte buffers) —
**HW-CHECK** against `log_bf_reg_size`.

### 1.3 Which doorbell does what

| Doorbell | How it is rung | Ref |
|---|---|---|
| **SQ (send)** | "The DoorBell register in the UAR is the first 2 DWORDS of the blue-flame buffer." Write the **first 8 bytes of the WQE** (ctrl segment dwords 00h and 04h: opcode, WQE index, SQ number, DS) to **offset 0 of a BlueFlame register** (UAR+800h). The WQE index written is the *start* of the WQE to execute — unlike the doorbell record, which points to the next empty WQEBB. A given SQ's doorbells must always use the same BlueFlame register (never interleaved across registers). | §7.2, §7.2.2 p65 |
| RQ | **No UAR doorbell.** Updating the RQ doorbell record (§3.2) hands the WQE to HW. | §7.4.3.1 |
| CQ | Polling only needs the **CQ doorbell record** (`update_ci`, §7.3). The UAR CQ register (020h) is used only to *arm* — never in this driver. "For best performance… a single 64-bit write." | §7.2.2, §7.12.2, §7.12.4 |
| EQ | Write `eqn<<24 | (consumer_counter & 0xFFFFFF)` to **048h** (update CI without arming). 040h arms the EQ (never used: a polled EQ is "created in fired state and never armed", §7.13.4). One dword. | §7.2.2, §7.13.4 |

### 1.4 Write width, order and ordering (§7.2.2, §7.2.2.1, §7.2.2.2, §7.4.3.1, §7.12.3.2)

- "SQ/RQ and CQ DoorBells require 64-bit writes." If the platform cannot do
  a 64-bit write, two 32-bit writes are allowed: **lower address first, then
  upper**, and **no other doorbell may be rung (or start) on the same UAR page
  in between**. (Single-threaded boot driver: satisfied trivially.) §7.2.2.2.
- The 8 bytes land in memory order = the WQE's bytes 0–7 (BE dword at +0,
  BE dword at +4). On little-endian x86 build the u64 from those 8 bytes with
  `from_le_bytes`, or write dword +0 then dword +4 each byte-swapped.
- Regular (non-BlueFlame) doorbells: map **without write-combining**;
  BlueFlame only with WC, in DWORD-aligned chunks, padded to a cache line
  (§7.2.2.1). This driver does not use BlueFlame.
- BlueFlame buffers "must be used alternately" (even, odd, even…) per
  register (§7.2.2.1). The PRM does not say whether that alternation also
  applies to the plain 8-byte send doorbell written at the same address —
  **HW-CHECK**. Recommended: alternate 800h/900h per ring as for BlueFlame
  (this satisfies both readings).
- Posting order (§7.4.3.1): (1) write WQE(s) to the buffer, (2) update the
  doorbell record, (3) ring the UAR doorbell. HW "may execute WQE after the
  second step"; a doorbell for an already-executed WQE is ignored. The PRM
  names the order but no barrier instructions: the driver must make the WQE
  stores visible before the DBR store and the DBR store visible before the
  MMIO write (store fence / `wmb` between each) — derived, not stated.
- CQ/EQ polling: "After seeing that the CQE is in SW ownership, SW should do
  memory barrier and re-read the CQE" (§7.12.3.2 note).

---

## 2. Physical buffers and PAS lists (T138/T139, T54/T55, T75/T76, T93/T94)

| Item | PRM | Ref |
|---|---|---|
| PAS entry | 8 bytes. off 00h 31:0 `pa_h` = PA[63:32]; off 04h 31:12 `pa_l` = PA[31:12]; bits 11:0 reserved (0). | T138, T139 |
| Page size, WQ | WQ ctx `log_wq_pg_sz` (20h 12:8) = log2(page size / 4 KB). | T55 |
| Page size, CQ | CQC `log_page_size` (18h 28:24) = log2(page / 4 KB). | T76 |
| Page size, EQ | EQC `log_page_size` (18h 28:24) = log2(page / 4 KB). | T94 |
| Page offset, CQ/EQ | CQC/EQC `page_offset` (08h 11:6): "Must be 0". | T76, T94 |
| Page offset, WQ | WQ `page_offset` (04h 20:16): "Page offset in quanta of (page_size / 64)". Fig 12 labels it `Page_offset[11:6]`. Use a page-aligned buffer → 0. | T55, Fig 12 |
| Alignment | WQ buffer aligned on WQEBB/stride (the larger), RQ on ≥ 64 B (§7.4.1); CQ buffer on CQE size (cache line recommended, §7.12.1); EQ on EQE size (§7.13.2). | |
| Count | One PAS per page of the buffer, in order (buffers are "virtually contiguous… passed to the HCA as a list of physical pages"). | §7.12.1, §7.13.2 |

A physically contiguous buffer is described as N consecutive 4 KB pages:
`log_*_page_size = 0`, `pas[i] = base + i×4096`. Using one large page
(`log_page_size > 0`) needs the base aligned to that page size; the PRM does
not state that alignment rule explicitly — **HW-CHECK**; use 4 KB pages.
In every create command of this document the PAS array starts at *abs* 110h
(8 bytes each); the input length is `110h + 8×npages` (mailbox chaining per
the command-interface spec).

---

## 3. Work queues common to SQ and RQ (§7.4)

### 3.1 Geometry (§7.4.1, §7.4.1.1, §7.4.1.2, §7.4.4)

| Item | SQ | RQ (cyclic) |
|---|---|---|
| Unit | WQEBB = **64 B**; a WQE occupies 1+ WQEBBs and may wrap past the end of the buffer (§7.4.1.1, Fig 14). | stride = 16 B × 2^k (power of two); one WQE per stride (§7.4.1.2). |
| Depth | power-of-two WQEBBs; max 2^`log_max_qp_sz`; max WQE `max_wqe_sz_sq` (octowords). | power-of-two strides; max WQE `max_wqe_sz_rq` (≤ 512 B). |
| Counter | `sq_wqebb_counter`: starts 0, += WQEBBs per WQE, **wraps at 0xFFFF** (16 bit). | `rq_wqe_counter` / `wqe_counter`: +1 per WQE, wraps at 0xFFFF. |
| Ctx | `log_wq_stride` = 6 (64 B) — PRM does not say explicitly that SQ must use 6 — **HW-CHECK** | `log_wq_stride` = log2(stride) (≥ 4). |

WQ size in bytes = 2^(`log_wq_sz` + `log_wq_stride`) (T55). (§7.4.1's text
about RQ and SQ sharing one buffer refers to the QP layout; SQ and RQ here are
separate objects, each with its own buffer.)

### 3.2 Doorbell record (§7.4.2, T24)

8 bytes in host memory, **4-byte aligned**, address given to HW as
`wq.dbr_addr`. Must be **0 at creation** (§7.4.2; also T57/T63: initialize
the counter to 0 before RST→RDY).

| off | bits | name | meaning |
|---|---|---|---|
| 00h | 15:0 | receive `counter` | RQ: `wqe_counter` = next empty stride (number of WQEs posted) |
| 04h | 15:0 | send `counter` | SQ: `sq_wqebb_counter` = next empty WQEBB |

Bit range 15:0 is from the T24 drawing plus "wraps around 0xFFFF"; 31:16 are
treated as reserved (write 0). **HW-CHECK — which half an SQ reads:** T24
describes one record with receive at +0 and send at +4 ("the receive and send
DoorBell record are consecutive in memory"), but SQ and RQ are separate
objects, each with its own `dbr_addr`; the PRM does not say whether an SQ
reads `dbr_addr+0` or `dbr_addr+4`. Safe choice: give each queue its own
8-byte record; RQ writes +0; **SQ writes the same value to both +0 and +4**.

### 3.3 WQE ownership and posting (§7.4.3, §7.4.3.1)

- Ownership passes to HW by writing the counter to the doorbell record; it
  returns to SW when HW writes the CQE. SW must not touch a WQE or its data
  buffers while HW owns it.
- Send: write WQE(s) contiguously (WQEBB granularity) → write DBR send counter
  (next empty WQEBB) → ring UAR (§1.3) with the ctrl segment of the **last**
  WQE written ("wqe_index in the DoorBell should point to the first wqebb of
  the last WQE posted").
- Receive: write WQE(s) → write DBR receive counter. Nothing else.
- Never post more WQEs (summed over every queue on a CQ) than the CQ has
  entries (§7.12.5).

### 3.4 WQ context (T54 layout, T55 fields) — embedded at SQ/RQ ctx off 30h

T55 columns mark applicability SQ / RQ / RMP.

| off | bits | name | SQ | RQ | meaning |
|---|---|---|---|---|---|
| 00h | 31:28 | `wq_type` | v | v | 0 = WQ_LINKED_LIST, **1 = WQ_CYCLIC**. "Send Queue WQ must be set to 1." |
| 00h | 27 | `wq_signature` | v | v | WQE signature checking. Use 0. |
| 00h | 26:25 | `end_padding_mode` | – | v | 0 = END_PAD_NONE, 1 = END_PAD_ALIGN (pad to cache line, never beyond the scatter entry). Use 0. |
| 04h | 20:16 | `page_offset` | v | v | first-WQE offset in page, in (page_size/64) quanta. 0. |
| 04h | 15:0 | `lwm` | – | – | limit watermark (RMP only). 0. |
| 08h | 23:0 | `pd` | v | v | protection domain (ALLOC_PD 0x800) |
| 0Ch | 23:0 | `uar_page` | see note | see note | UAR used for this WQ's doorbells |
| 10h | 31:0 | `dbr_addr[63:32]` | v | v | doorbell record PA |
| 14h | 31:0 | `dbr_addr[31:0]` | v | v | |
| 18h | 31:0 | `hw_counter` | | | QUERY only (next stride HW consumes; 31:16 reserved) |
| 1Ch | 31:0 | `sw_counter` | | | QUERY only |
| 20h | 19:16 | `log_wq_stride` | v | v | stride = 2^n bytes |
| 20h | 12:8 | `log_wq_pg_sz` | v | v | log2(page / 4 KB) |
| 20h | 4:0 | `log_wq_sz` | v | v | log2(number of strides); must be < `HCA_CAP.log_max_wq_size` |
| 24h–BCh | | reserved | | | |
| C0h… | 64 each | `pas[]` | v | v | T138 entries |

`page_offset`/`lwm` are at **04h** per the T54 drawing; T55 lists them under
the 00h row without an offset of their own. `uar_page`: T55 marks only the RQ
column and says "for RQ… required only if cd_slave is enabled, otherwise
reserved"; T62 lists `uar_page` as **Required** for CREATE_SQ and T56 as N/A
for RQ. So: SQ = our UAR index, RQ = 0.

---

## 4. Send WQE (§7.4.4.1, §7.4.4.2, T25–T30, T38, T39, T41)

Segments are multiples of 16 B (an *octoword*). Order for an Ethernet
**Send** (T41): **Ctrl** (mandatory) → **Eth** (mandatory) → **Data**
(pointer and/or inline, "V"). NOP = Ctrl only.

### 4.1 Ctrl segment (T25 layout, T26 fields) — WQE off 00h, 16 B

| off | bits | name | meaning |
|---|---|---|---|
| 00h | 31:24 | reserved | (no `opmod` is defined in this rev) |
| 00h | 23:8 | `WQE Index` | WQEBB number of this WQE's first block = `sq_wqebb_counter` at post; wraps at 0xFFFF regardless of WQ size |
| 00h | 7:0 | `OPCODE` | **0x00 NOP** (completion only), **0x0A send**, 0x25 UMR |
| 04h | 31:8 | `SQ number` | SQN from CREATE_SQ |
| 04h | 5:0 | `DS` | WQE size in octowords, all segments counted |
| 08h | 31:24 | `signature` | XOR signature (only if `wq_signature`); 0 |
| 08h | 7:5 | `FM` | fence: 000 none, 001 initiator small fence |
| 08h | 3:2 | `CE` | 00 CQE only on error; 01 CQE only on first error; **10 CQE on every completion**; 11 CQE + EQE |
| 08h | 1 | `SE` | solicited event |
| 0Ch | 31:0 | `invalidation_key/umr_mkey` | UMR only. There is **no immediate-data field** in this PRM's ctrl segment. 0. |

The T25 drawing shows only `signature` at 08h; FM/CE/SE positions are from T26.
This driver sets `CE = 10b` (byte 0Bh = 08h) on every WQE so each send is
freed by its own CQE.

### 4.2 Eth segment (T27, T28) — WQE off 10h

| off (in seg) | bits | name | meaning |
|---|---|---|---|
| 00h | 31:0 | reserved | |
| 04h | 31 | `l4cs` | HW computes TCP/UDP checksum (ignored for non-TCP/UDP) |
| 04h | 30 | `l3cs` | HW computes IPv4 header checksum (ignored for non-IPv4) |
| 04h | 13:0 | `mss` | LSO only; **must be 0** on non-LSO WQEs |
| 08h | 31:0 | reserved | |
| 0Ch | 25:16 | `inline_header_size` | number of inlined packet-header bytes (10 bits) |
| 0Ch | 15:0 | `inline_headers` | first 2 inlined bytes (seg bytes 0Eh, 0Fh) |
| 10h… | 31:0 | `inline_headers cont'd` | rest of the inline header, contiguous; "can span multiple octowords up to the maximal WQE size"; last octoword partially filled, rest reserved |

So the inline header is the byte run starting at segment byte 0Eh. For
standard Ethernet the inlined bytes start at the Ethernet header (DA).
"Headers that are inlined should be excluded from the data segments that
follow." Segment length = 16 + roundup16(max(N − 2, 0)) bytes for N inline
bytes. Padding segment (T29/T30): 16 reserved bytes.

### 4.3 Data segments (§7.4.4.1.4, T38, T39)

Memory pointer (bit 31 of dword 0 = 0), 16 B:

| off | bits | name | meaning |
|---|---|---|---|
| 00h | 31 | 0 | pointer type |
| 00h | 30:0 | `byte_count` | bytes to gather; **0 means 2 GB** |
| 04h | 31:0 | `L_key` | memory key (MKey or reserved LKey, see §10) |
| 08h | 31:0 | `local address[63:32]` | |
| 0Ch | 31:0 | `local address[31:0]` | |

"To send a request with zero data, DS should be set such that no data
segments appear in the WQE" — never emit a pointer segment with byte_count 0.

Inline data (bit 31 = 1): 00h bit 31 = 1, bits 9:0 `byte_count`; data from
04h onward, big-endian byte order (first byte in bits 31:24), padded to a
16-byte boundary; `byte_count = 0` adds nothing (usable as filler).

### 4.4 Construction summary for one raw frame (len L, inline N ≤ L)

| WQE off | content |
|---|---|
| 00h | ctrl: dw0 = `(wqebb_counter & 0xFFFF) << 8 | 0x0A`; dw1 = `sqn << 8 | DS`; dw2 = `CE=10b` (0x00000008); dw3 = 0 |
| 10h | eth: 04h = cs flags (0 unless offload wanted), 0Ch = `N << 16 | frame[0] << 8 | frame[1]` |
| 20h… | frame[2..N], padded to 16 |
| next | pointer seg: `byte_count = L − N`, lkey, `addr = buf + N` — omitted when L == N |

DS = 1 (ctrl) + 1 + ceil(max(N−2,0)/16) (eth) + (L > N ? 1 : 0).
Example N = 14 or 18: DS = 4 → exactly one WQEBB.
WQEBBs used = ceil(DS / 4); `sq_wqebb_counter += that`.
The device pads frames shorter than 64 B with zeros before the FCS (§3.3.8;
`HCA_CAP.pad_tx_eth_packet` 4Ch bit 29). DS = 1 WQEs cannot use BlueFlame
(§7.2.2.1) — irrelevant here.

### 4.5 Minimum inline header (§7.4.4.1.2, §7.9.3, §7.9.3.1, T16, T146)

Two different enumerations — do not mix them up:

`HCA_CAP.wqe_inline_mode` (per-protocol networking caps, off 00h bits 13:12):

| value | meaning |
|---|---|
| 0 | L2 — minimum inline is L2 |
| 1 | `nic_vport_context` — the minimum is the vport's `min_wqe_inline_mode` (NIC_Vport ctx off 00h 26:24, read with QUERY_NIC_VPORT_CONTEXT 0x754); SQ.`min_wqe_inline_mode` must be ≥ it or CREATE_SQ fails |
| 2 | not_required |

SQ/vport `min_wqe_inline_mode` (SQ ctx 00h 26:24):

| value | required inline bytes |
|---|---|
| 0 | none |
| 1 | L2: L2 headers up to and including the last EtherType; for LLC/SNAP frames max(17 B, that) |
| 2 | IP: through the end of the IP header (non-IP: L2 rule) |
| 3 | TCP/UDP: through the end of the L4 header (else IP rule) |
| 4 | reserved |
| 5–7 | inner L2 / inner IP / inner TCP-UDP (tunnels) |

A WQE with less inline header than required is **silently dropped**
(T61, §7.9.3). For raw frames on ConnectX-4 Lx: read `wqe_inline_mode`; if 1,
read the vport minimum and create the SQ with exactly that; if 0, treat as
L2. Then per frame inline at least: L2 → 14 B (Ethernet II), 18 B (one
802.1Q tag), 22 B (802.3 + LLC + SNAP), +4 per extra tag; IP/TCP modes →
parse to the end of the IP / L4 header. Inlining more than required is
allowed (the frame is inline bytes ‖ gathered bytes). Simplest correct
policy: inline `min(L, R)` where R is the parsed requirement, never less than
`min(L, 18)` in L2 mode. Which value a CX-4 Lx actually reports — **HW-CHECK**.

---

## 5. Receive WQE (§7.4.1.2, §7.4.4.1.5, §7.4.4.3, T21, T40)

A cyclic-RQ WQE is just a list of 16-byte scatter entries (optionally
preceded by a signature segment if `wq_signature`; not used):

| off | bits | name | meaning |
|---|---|---|---|
| 00h | 31 | 0 | (receive WQEs carry no inline data; bit must be 0) |
| 00h | 30:0 | `byte_count` | buffer length; **0 means 2 GB** |
| 04h | 31:0 | `L_key` | |
| 08h | 31:0 | `local address[63:32]` | |
| 0Ch | 31:0 | `local address[31:0]` | |

- **Terminator ("Null scatter pointer")**: `byte_count = 0` and
  `L_Key = 0x00000100` ends the list early when the stride is larger than the
  WQE (§7.4.1.2). Not needed when the stride holds exactly the entries used.
- Recommended: one entry per WQE, `log_wq_stride = 4` (16 B), buffer ≥ MTU +
  18 (VLAN kept, see `vlan_strip_disable`).
- T40 mentions a `start_padding` field (§3.3.9, only if
  `HCA_CAP.start_pad`) but the drawing does not place it — **HW-CHECK**; leave
  bit 31 = 0.
- Whether the FCS is scattered: PRM does not say — **HW-CHECK** (CQE
  `byte_cnt` gives the real length either way).

---

## 6. Contexts

### 6.1 TIS context (§7.6, T49, T50) — 160 B (A0h)

| off | bits | name | meaning |
|---|---|---|---|
| 00h | 19:16 | `prio` | "Ethernet Priority in prio[3:1]" (sic). 0. |
| 04h–20h | | reserved | |
| 24h | 23:0 | `transport_domain` | from ALLOC_TRANSPORT_DOMAIN (0x816, output off 08h) |
| 28h–9Ch | | reserved | |

TIS is RDY once created (§12.9.1). MODIFY_TIS bitmask (T51): bit 0 = prio.

### 6.2 TIR context (§7.5, T44, T45) — 240 B (F0h)

| off | bits | name | direct-TIR value |
|---|---|---|---|
| 00h | | reserved | |
| 04h | 31:28 | `disp_type` | **0x0 = Direct** (to `inline_rqn`; rx_hash_fn must be HASH_NONE). 0x1 = Indirect (RQT). |
| 08h–0Ch | | reserved | |
| 10h | 27:12 | `lro_timeout_period_usecs` | 0 |
| 10h | 11:8 | `lro_enable_mask` | 0 = LRO off (bit0 IPv4, bit1 IPv6) |
| 10h | 7:0 | `lro_max_msg_sz` | 0 |
| 14h–18h | | reserved | |
| 1Ch | 23:0 | `inline_rqn` | **our RQN** |
| 20h | 31 | `rx_hash_symmetric` | 0 |
| 20h | 23:0 | `indirect_table` | 0 (indirect only) |
| 24h | 31:28 | `rx_hash_fn` | **0 = HASH_NONE** (1 inverted XOR8, 2 Toeplitz) |
| 24h | 25:24 | `self_lb_en` | bitmask of self-loopback to *enable*: bit0 unicast (needs `HCA_CAP.self_lb_uc`), bit1 multicast (`self_lb_mc`). **0** = block our own transmissions (TIS and TIR in the same transport domain, §3.4). |
| 24h | 23:0 | `transport_domain` | same TD as the TIS |
| 28h–4Ch | 320 | `rx_hash_toeplitz_key` | 0 |
| 50h | 31:0 | `rx_hash_field_selector_outer` | 0 (T46/T47) |
| 54h | 31:0 | `rx_hash_field_selector_inner` | 0 |
| 58h–ECh | | reserved | |

CREATE_TIR places the TIR in RDY (§12.8.1). MODIFY_TIR bitmask (T48): bit 0
LRO, 1 indirect_table, 2 hash, 4 self_lb_en (only if `self_lb_en_modifiable`).

### 6.3 RQ context (§7.7, T52, T53) — 248 B incl. pas[0] (1984 bits)

| off | bits | name | value |
|---|---|---|---|
| 00h | 31 | `rlkey` | 1 = reserved LKey usable on this RQ (see §10) |
| 00h | 28 | `vlan_strip_disable` | 0 strip VLAN (reported in CQE cv/vid), **1 keep** — use 1 for SNP |
| 00h | 27:24 | `mem_rq_type` | **0 = MEMORY_RQ_INLINE**, 1 = RMP |
| 00h | 23:20 | `state` | 0 RST, 1 RDY, 3 ERR (create in RST) |
| 00h | 18 | `flush_in_error_en` | 1 = flush posted and future WQEs in error on ERR |
| 04h | 23:0 | `user_index` | reported in CQE if `HCA_CAP.cqe_version == 1` (else reserved) |
| 08h | 23:0 | `cqn` | receive CQ |
| 0Ch | | reserved | |
| 10h | 23:0 | `rmpn` | 0 (RMP only) |
| 14h–2Ch | | reserved | |
| 30h… | | `wq` | §3.4 (pas at ctx off F0h) |

### 6.4 SQ context (§7.9, T60, T61) — 248 B incl. pas[0] (1984 bits)

| off | bits | name | value |
|---|---|---|---|
| 00h | 31 | `rlkey` | reserved LKey enable |
| 00h | 29 | `fre` | fast-register enable. 0 |
| 00h | 28 | `flush_in_error_en` | 1 |
| 00h | 26:24 | `min_wqe_inline_mode` | §4.5 |
| 00h | 23:20 | `state` | 0 RST, 1 RDY, 3 ERR (create in RST) |
| 04h | 23:0 | `user_index` | as RQ |
| 08h | 23:0 | `cqn` | send CQ |
| 0Ch–1Ch | | reserved | |
| 20h | 31:16 | `tis_lst_sz` | **1** ("only a single TIS is supported" in this rev) |
| 24h–28h | | reserved | |
| 2Ch | 23:0 | `tis_num[0]` | our TISN |
| 30h… | | `wq` | §3.4 (pas at ctx off F0h) |

### 6.5 RQ / SQ state machine (Fig 16, Fig 17, T56, T57, T62, T63)

CREATE → **RST**. MODIFY RST2RDY → **RDY**. RDY2RDY (modify fields).
RDY2ERR by command or by HW (error). Any state → RST by MODIFY (2RST).
DESTROY from any state.

| State | Behaviour |
|---|---|
| RST | Doorbells not processed, WQEs not flushed; incoming frames dropped. Doorbell counter must be 0 before RST→RDY. |
| RDY | SQ: WQEs processed. RQ: WQEs can be posted; frames arrive **only if a TIR points at the RQ** (T57). |
| ERR | With `flush_in_error_en`, posted WQEs complete with error (syndrome 0x5 flushed); otherwise not forced to flush and no new WQEs accepted. |

Bitmask tables (columns CREATE / RST2RDY / RDY2RDY / RDY2ERR / 2RST):
- RQ (T56): `state` R in all; bit **1 `vsd`** (vlan_strip_disable) R at
  create, O at RST2RDY and RDY2RDY; every other field ("none") R at create only
  (`uar_page` N/A).
- SQ (T62): `state` R in all; bit **0 `packet_pacing_rate_limit_index`** R at
  create, O otherwise; every other field R at create only (`end_padding_mode`
  and `lwm` N/A).

So RST→RDY needs `modify_bitmask = 0` and only `state = 1` in the context.

---

## 7. Commands (§12.6–§12.11)

Common: input 00h 31:16 `opcode`, 04h 15:0 `op_mod` (0 for all below);
output 00h 31:24 `status`, 04h 31:0 `syndrome`. Lengths in bytes.

| Command | opcode | input (*abs*) | in len | output |
|---|---|---|---|---|
| CREATE_EQ | 0x301 | 08h–0Ch rsvd; **10h–4Ch EQC**; 50h–54h rsvd; **58h–5Ch event bitmask** (64 bit); 60h–10Ch rsvd; **110h pas[]** | 110h+8n | 08h 7:0 `eq_number` |
| DESTROY_EQ | 0x302 | 08h 7:0 `eq_number` | 10h | — |
| CREATE_CQ | 0x400 | 08h–0Ch rsvd; **10h–4Ch CQC**; 50h–10Ch rsvd (T260 prints "58h-10Ch"); **110h pas[]** | 110h+8n | 08h 23:0 `cqn` |
| DESTROY_CQ | 0x401 | 08h 23:0 `cqn` | 10h | — |
| CREATE_TIR | 0x900 | 08h–1Ch rsvd; **20h–10Ch TIR ctx** | 110h | 08h 23:0 `tirn` |
| DESTROY_TIR | 0x902 | 08h 23:0 `tirn` | 10h | — |
| CREATE_SQ | 0x904 | 08h–1Ch rsvd; **20h SQ ctx** (wq at 50h, **pas at 110h**) | 110h+8n | 08h 23:0 `sqn` |
| MODIFY_SQ | 0x905 | 08h 31:28 `sq_state` (current), 08h 23:0 `sqn`; 10h `modify_bitmask[63:32]`, 14h `[31:0]`; 18h–1Ch rsvd; **20h SQ ctx** | 20h+F8h | — |
| DESTROY_SQ | 0x906 | 08h 23:0 `sqn` | 10h | — |
| CREATE_RQ | 0x908 | as CREATE_SQ with RQ ctx at 20h | 110h+8n | 08h 23:0 `rqn` |
| MODIFY_RQ | 0x909 | 08h 31:28 `rq_state` (current), 08h 23:0 `rqn`; 10h/14h bitmask; **20h RQ ctx** | 20h+F8h | — |
| DESTROY_RQ | 0x90A | 08h 23:0 `rqn` | 10h | — |
| CREATE_TIS | 0x912 | 08h–1Ch rsvd; **20h–BFh TIS ctx** | C0h | 08h 23:0 `tisn` |
| DESTROY_TIS | 0x914 | 08h 23:0 `tisn` | 10h | — |

Outputs are 10h bytes. MODIFY with a wrong current state fails with
BAD_RESOURCE_STATE and leaves the object unchanged (§12.10.2, §12.11.2).
QUERY_* (0x303, 0x402, 0x903, 0x907, 0x90B, 0x915) exist for debugging.

### 7.1 Absolute offsets of every field the driver writes

CREATE_SQ / CREATE_RQ (ctx base 20h, wq base 50h):

| *abs* | bits | SQ field | RQ field |
|---|---|---|---|
| 20h | 31 / 29 / 28 / 26:24 / 23:20 | rlkey / fre / flush_in_error_en / min_wqe_inline_mode / state | rlkey 31, vlan_strip_disable 28, mem_rq_type 27:24, state 23:20, flush_in_error_en 18 |
| 24h | 23:0 | user_index | user_index |
| 28h | 23:0 | cqn | cqn |
| 30h | 23:0 | — | rmpn |
| 40h | 31:16 | tis_lst_sz | — |
| 4Ch | 23:0 | tis_num[0] | — |
| 50h | 31:28 / 27 / 26:25 | wq_type / wq_signature / end_padding_mode | same |
| 54h | 20:16 / 15:0 | page_offset / lwm | same |
| 58h | 23:0 | pd | pd |
| 5Ch | 23:0 | uar_page | (0) |
| 60h / 64h | 31:0 | dbr_addr hi / lo | same |
| 70h | 19:16 / 12:8 / 4:0 | log_wq_stride / log_wq_pg_sz / log_wq_sz | same |
| 110h… | | pas[] | pas[] |

CREATE_TIR (ctx base 20h): 24h 31:28 `disp_type`; 30h LRO; **3Ch 23:0
`inline_rqn`**; 40h `indirect_table`; **44h** 31:28 `rx_hash_fn`, 25:24
`self_lb_en`, 23:0 `transport_domain`; 48h–6Ch key; 70h/74h selectors.

CREATE_TIS (ctx base 20h): 20h 19:16 `prio`; **44h 23:0 `transport_domain`**.

CREATE_CQ (CQC base 10h): see §8.6 *abs* column. CREATE_EQ (EQC base 10h):
see §9.6 *abs* column.

### 7.2 Bring-up / teardown order (derived)

Up: ALLOC_UAR, ALLOC_PD, ALLOC_TRANSPORT_DOMAIN → CREATE_EQ (bitmask
page-request + port-change) → CREATE_CQ ×2 (`c_eqn` = our EQ) → CREATE_TIS →
CREATE_SQ (RST) → MODIFY_SQ RST2RDY → CREATE_RQ (RST) → post receive WQEs →
MODIFY_RQ RST2RDY → CREATE_TIR (direct → RQ) → steering (steering.md).
Down: steering → DESTROY_TIR → DESTROY_RQ → DESTROY_SQ → DESTROY_TIS →
DESTROY_CQ ×2 → DESTROY_EQ → dealloc TD/PD/UAR. The PRM gives no mandatory
destroy order beyond "do not destroy an EQ while events are still mapped to
it" (§12.6) — **HW-CHECK** that DESTROY of a RDY SQ/RQ is accepted (no
MODIFY to RST/ERR first is documented as required).

---

## 8. Completion queue (§7.12)

### 8.1 Buffer and counters (§7.12.1, Fig 20)

- 2^`log_cq_size` CQEs (max 2^22; `log_max_cq_sz`), CQE size 64 B (`cqe_sz`
  = 0) or 128 B (1). This driver uses 64 B. (Where the 64-byte CQE sits inside
  a 128-byte CQE is not stated — **HW-CHECK**; avoid.)
- Producer counter (HW) and consumer counter (SW), 24-bit; low
  `log_cq_size` bits are the index. Empty when equal; full when
  producer − consumer = size. Overflow with detection on: CQ and the queue go
  to error and an event is raised; with `oi` set old CQEs may be overwritten.

### 8.2 64-byte CQE (T67 layout, T68 fields)

T68 gives widths only; offsets/bits below are from the T67 drawing (row
boundaries and column positions measured in the PDF).

| off | bits | name | meaning (T68) |
|---|---|---|---|
| 00h | | reserved | (00h–1Ch is also the `inline_32` area when `cqe_format = 1`) |
| 04h | 31 | `lro_timestamp_is_valid` | LRO only |
| 04h | 30 | `lro_tcp_psh` | LRO only |
| 04h | 29:28 | `lro_abrt` | LRO only |
| 04h | 27:24 | `lro_dup_ack_cnt` | LRO only |
| 04h | 23:16 | `lro_min_ttl` | LRO only |
| 04h | 15:0 | `lro_tcp_win` | LRO only |
| 08h | 31:0 | `lro_ack_seq_num` | LRO only |
| 0Ch | 31:0 | `rx_hash_result` | valid if `rx_hash_type ≠ 0` |
| 10h | 31:24 | `rx_hash_type` | 0 = no hash |
| 14h | 31:16 | `checksum` | 1's-complement 16-bit sum (§3.3.1, T4) |
| 18h | | reserved | |
| 1Ch | 26 | `l4_ok` | TCP/UDP parsed and L4 checksum OK |
| 1Ch | 25 | `l3_ok` | IP parsed, checks and IPv4 checksum OK |
| 1Ch | 24 | `l2_ok` | L2 headers passed HW checks |
| 1Ch | 23 | `ip_frag` | fragmented IPv4/IPv6 |
| 1Ch | 22:20 | `l4_hdr_type` | 0 none, 1 TCP, 2 UDP, 3 TCP empty ACK, 4 TCP ACK+data |
| 1Ch | 19:18 | `l3_hdr_type` | 0 none, 1 IPv6, 2 IPv4 |
| 1Ch | 17 | `ip_ext_opts` | IPv4 options / IPv6 extensions present |
| 1Ch | 16 | `cv` | C-VLAN was present (and stripped) |
| 1Ch | 15:13 | `up` | 802.1p priority (if cv) |
| 1Ch | 12 | `cfi` | (if cv) |
| 1Ch | 11:0 | `vid` | VLAN ID (if cv) |
| 20h | 31:24 | `lro_num_seg` | 0 = no LRO |
| 20h | 23:0 | `srqn/user_index` | `user_index` of the RQ/SQ when `cqe_version == 1` |
| 24h–28h | | reserved | |
| 2Ch | 31:0 | `byte_cnt` | receive: bytes scattered (padding excluded) |
| 30h | 31:0 | `timestamp_h` / `lro_timestamp_value` | |
| 34h | 31:0 | `timestamp_l` / `lro_timestamp_echo` | |
| 38h | 31:24 | `s_wqe_opcode` | requester: opcode of the send WQE |
| 38h | 23:0 | `flow_tag` | receive, `cqe_version == 1`: flow entry's tag |
| 3Ch | 31:16 | `wqe_counter` | SQ: `sq_wqe_counter`; RQ: `rq_wqe_counter` of the completed WQE |
| 3Ch | 15:8 | `signature` | byte-XOR of CQE and index = 0xFF |
| 3Ch | 7:4 | `opcode` | see 8.4 |
| 3Ch | 3:2 | `cqe_format` | 0 none, 1 inline in data_32 (00h–1Ch), 2 inline in data_64, 3 compressed |
| 3Ch | 1 | `se` | solicited event |
| 3Ch | 0 | `owner` | ownership bit |

So byte 3Fh = `opcode<<4 | cqe_format<<2 | se<<1 | owner`. There is **no
QPN/SQN/RQN field** in this CQE: identify the queue by the CQ (one queue per
CQ here) or by `user_index`. T68 also lists `rx_drop_counter` (8 bits, "dropped
for lack of RCV WQE") with no position in T67 — **HW-CHECK**.

### 8.3 Completion with error (§7.12.7, T73, T74) — opcode 13 or 14

| off | bits | name | meaning |
|---|---|---|---|
| 00h | 15:0 | `wqe_id` | |
| 04h–1Ch | | reserved | |
| 20h | 23:0 | `usr_index` | |
| 24h–28h | | reserved | |
| 2Ch | 31:0 | `byte_cnt` | |
| 30h | | reserved | |
| 34h | 15:8 | `vendor_error_syndrome` | undocumented |
| 34h | 7:0 | `syndrome` | 0x1 Local_Length, 0x4 Local_Protection, **0x5 Work_Request_Flushed**, 0x6 Memory_Window_Bind, 0x10 Bad_Response, 0x11 Local_Access, 0x12 Remote_Invalid_Request, 0x13 Remote_Access, 0x14 Remote_Operation |
| 38h | 31:24 / 23:0 | `s_wqe_opcode` / `flow_tag` | |
| 3Ch | 31:16 / 15:8 / 7:4 / 0 | `wqe_counter` / `signature` / `opcode` / `owner` | |

(Positions from the T73 drawing; T74 gives only the syndrome bits.) After an
SQ/RQ enters ERR, later WQEs complete as flushed errors.

### 8.4 CQE opcodes (T68) and the resize CQE (§7.12.6)

| opcode | meaning |
|---|---|
| 0x0 | requester (send completed) |
| 0x2 | responder – send (frame received) |
| 0x5 | Resize_CQ special CQE (only after MODIFY_CQ resize; unused) |
| 0xD (13) | requester error |
| 0xE (14) | responder error |
| 0xF (15) | invalid CQE — "ownership bit is not valid and the CQE is in HW ownership" |

### 8.5 Ownership, initial state, polling (§7.12.3.1, §7.12.3.2, §12.7.1)

```
sw_owned(cqe) = cqe.owner == ((consumer_counter >> log_cq_size) & 1)
```

- Initial state (both rules apply): every CQE **opcode = 0xF** (§12.7.1) and
  **owner = HW** (§7.12.3.1). With consumer = 0, HW = 1. So byte 3Fh = **0xF1**
  in every CQE before CREATE_CQ.
- Poll: entry = `CQE[consumer_counter & (size−1)]`; valid iff `sw_owned` **and**
  `opcode ≠ 0xF`; then memory barrier and re-read the CQE; consume;
  `consumer_counter++`; write `update_ci`. Repeat while valid. The doorbell
  record may be written once after a batch.
- Counters are 24-bit; mask when writing.

### 8.6 CQ doorbell record (§7.12.2, T69, T70) — 8 B, **8-byte aligned**

| off | bits | name | meaning |
|---|---|---|---|
| 00h | 23:0 | `update_ci` | consumer counter of the last polled CQE (written after every poll) |
| 04h | 29:28 | `cmd_sn` | arm sequence number (arming only) |
| 04h | 24 | `cmd` | 1 = solicited only; 0 = solicited or unsolicited |
| 04h | 23:0 | `arm_ci` | consumer counter for arming |

Polling driver: write only `update_ci`; leave 04h = 0; never arm.

### 8.7 CQ context (§7.12.8, T75, T76) — 64 B; *abs* = offset in CREATE_CQ

| off | *abs* | bits | name | value |
|---|---|---|---|---|
| 00h | 10h | 31:28 | `status` | QUERY only (0 OK, 9 overflow, A write fail) |
| 00h | 10h | 23:21 | `cqe_sz` | **0 = 64 B**, 1 = 128 B |
| 00h | 10h | 20 | `cc` | collapse all CQEs to entry 0. 0 |
| 00h | 10h | 18 | `scqe_break_moderation_en` | 0 |
| 00h | 10h | 17 | `oi` | overrun ignore (then CI/arm doorbells must not be rung). **0** |
| 00h | 10h | 16:15 | `cq_period_mode` | 0 |
| 00h | 10h | 14 | `cqe_compression_en` | 0 |
| 00h | 10h | 13:12 | `mini_cqe_res_format` | 0 |
| 00h | 10h | 11:8 | `st` | QUERY only; reserved for CREATE |
| 08h | 18h | 11:6 | `page_offset` | must be 0 |
| 0Ch | 1Ch | 28:24 | `log_cq_size` | log2(entries), ≤ 22 |
| 0Ch | 1Ch | 23:0 | `uar_page` | our UAR index |
| 10h | 20h | 27:16 | `cq_period` | 0 (moderation off) |
| 10h | 20h | 15:0 | `cq_max_count` | 0 |
| 14h | 24h | 7:0 | `c_eqn` | our EQ (completion events unused but name a real EQ) |
| 18h | 28h | 28:24 | `log_page_size` | log2(page / 4 KB) |
| 20h | 30h | 23:0 | `last_notified_index` | QUERY only |
| 24h | 34h | 23:0 | `last_solicit_index` | QUERY only |
| 28h | 38h | 23:0 | `consumer_counter` | **must be 0** at create |
| 2Ch | 3Ch | 23:0 | `producer_counter` | HW (0) |
| 38h–3Ch | 48h–4Ch | 64 | `dbr_addr` | PA of the CQ doorbell record |

The T75 drawing does not show `cq_period_mode`, `cqe_compression_en`,
`mini_cqe_res_format` (T76 gives their bits).

---

## 9. Event queue (§7.13)

### 9.1 Buffer and EQE (§7.13.1, §7.13.2, T77, T78)

2^`log_eq_size` EQEs of **64 B** (max `log_max_eq_sz`). Producer/consumer
counters as for the CQ.

| off | bits | name | meaning |
|---|---|---|---|
| 00h | 23:16 | `event_type` | T79 |
| 00h | 7:0 | `event_sub_type` | per type, else 0 |
| 04h–1Ch | | reserved | |
| 20h–38h | 224 | `event_data` | type-specific (below) |
| 3Ch | 15:8 | `signature` | byte-XOR of EQE and index = 0xFF |
| 3Ch | 0 | `owner` | ownership |

Event-data tables (T80–T92) number their offsets from the start of
`event_data` (00h–18h = 7 dwords = the 224-bit field); the *EQE* column adds 20h.

### 9.2 Event types (§7.13.6, T79)

| type | event | data |
|---|---|---|
| 0x00 | Completion_Events | T80: ed 18h (EQE **38h**) 23:0 CQ number |
| 0x04 | CQ_Error | T84/T85: ed 00h (EQE 20h) 23:0 `cqn`; ed 08h (EQE 28h) 7:0 `syndrome` 1 CQ_overrun, 2 CQ_access_violation |
| 0x08 | Internal_Error | no EQE — reported in the init-segment health buffer (§7.13.9.1); poll it |
| 0x09 | Port_State_Change | T86/T87: ed 08h (EQE **28h**) 31:28 `port_num`; subtype T88 |
| 0x0A | Command_Interface_Completion | T89/T90: ed 00h (EQE 20h) 31:0 `command_completion_vector` |
| 0x0B | Page_Request | T91/T92: ed 04h (EQE **24h**) 31:0 `num_pages`, signed (msb = sign): >0 the device wants pages, <0 it returns pages; a recommendation — answer with one MANAGE_PAGES. ed 00h is reserved (no function id in this rev). |
| 0x13 | Last_WQE_Reached | T82/T83: ed 14h (EQE 34h) 31:24 `type` 1 RQ / 2 SQ; ed 18h (EQE 38h) 23:0 `rqn/sqn` |
| 0x17 | Temp_Warning_Event | layout not given |

Page request: "once Page Request Event is generated, it will not be generated
again before the driver executes the MANAGE_PAGES command" — MANAGE_PAGES
re-arms it (§7.13.10.2).

Port change subtypes (T88): 0x1, 0x4, 0x5 are all listed as "port changed its
state" — the PRM text says the subtype denotes up/down but the table does not
say which is which (**HW-CHECK**); on any of them, re-read the port state
(PAOS / QUERY_VPORT_STATE).

### 9.3 Ownership, initial state, polling (§7.13.2.1, §7.13.4, §12.6.1)

- Same rule as the CQ: SW owns the EQE iff `owner == ((consumer_counter >>
  log_eq_size) & 1)`.
- Initial: every EQE **owner = 1** ("initial HW ownership", §12.6.1).
- A polled EQ is "created in fired state and never be armed"; it is managed
  "by polling the EQ and updating the EQ consumer index using DoorBells".
  Drain: while `sw_owned(EQ[cc & mask])` { barrier, read, handle, cc++ };
  then write `eqn<<24 | (cc & 0xFFFFFF)` to UAR+**048h**. Without `oi`, HW
  uses this counter for overflow detection, so update it after every drain.

### 9.4 EQ doorbell register

See §1.2: UAR+040h (update + arm), UAR+048h (update only); dword: eqn 31:24,
consumer counter 23:0. "When arming the EQ at the first time, Consumer-index
should be 0" (§7.2.2) — not relevant (never armed).

### 9.5 EQ state machine (§7.13.12, Fig 24)

CREATE_EQ → **Fired**. ARM doorbell → Armed. Event while Armed → interrupt,
back to Fired. A polled EQ stays Fired.

### 9.6 EQ context (§7.13.11, T93, T94) — 64 B; *abs* = offset in CREATE_EQ

| off | *abs* | bits | name | value |
|---|---|---|---|---|
| 00h | 10h | 31:28 | `status` | QUERY only (0 OK, A write failure) |
| 00h | 10h | 18 | `ec` | collapse to entry 0. 0 |
| 00h | 10h | 17 | `oi` | overrun ignore. 0 |
| 00h | 10h | 11:8 | `st` | 9 ARMED, A FIRED; reserved for CREATE |
| 08h | 18h | 11:6 | `page_offset` | must be 0 |
| 0Ch | 1Ch | 28:24 | `log_eq_size` | log2(entries) |
| 0Ch | 1Ch | 23:0 | `uar_page` | our UAR index |
| 14h | 24h | 7:0 | `intr` | MSI-X table index; reserved if MSI-X disabled (UEFI: 0) |
| 18h | 28h | 28:24 | `log_page_size` | log2(page / 4 KB) |
| 28h | 38h | 23:0 | `consumer_counter` | must be 0 |
| 2Ch | 3Ch | 23:0 | `producer_counter` | should be 0 |
| 30h–3Ch | 40h–4Ch | | reserved | |

### 9.7 CREATE_EQ event bitmask (*abs* 58h–5Ch)

64 bits: 58h = bits 63:32, 5Ch = bits 31:0; "Bitmask[i] is related to the
event with event-type = i"; bit 0 reserved (completion events are routed by
`CQC.c_eqn`, not by the bitmask). For this driver: bit 9 (port change) and
bit 11 (page request) → dword 5Ch = **0x00000A00**, 58h = 0. (Optionally bit 4
CQ_Error.) The "bit i of the 64-bit BE quantity = bit (i mod 32) of the dword
at 5Ch/58h" reading is the PRM's usual numbering, not stated for this field —
**HW-CHECK** (verify a page-request EQE arrives).

Events not mapped to an EQ are not reported. The EQ number is per function and
starts at 0 (§7.13.11). CREATE_EQ output: 08h 7:0 `eq_number`.

---

## 10. HW-CHECK list (what the PRM leaves open)

1. **UAR index vs init segment**: whether ALLOC_UAR can return an index whose
   page overlaps the init segment (BAR 0000h–~4010h); BAR offset formula is
   only shown as Fig 11.
2. **BlueFlame register size** vs `log_bf_reg_size`; T13 shows 2 registers of
   2×256 B, while §7.2.2.1 says "Blue flame registers 2 and 3 are used for
   fast path posting".
3. **Plain send doorbell and even/odd alternation**: whether the 8-byte
   doorbell at 800h must alternate with 900h like BlueFlame posting.
4. **SQ doorbell record half**: whether an SQ reads its counter at
   `dbr_addr+0` or `+4` (write both).
5. **SQ `log_wq_stride`**: expected to be 6 (64-byte WQEBB); not stated.
6. **Minimum inline mode actually reported** by a CX-4 Lx
   (`wqe_inline_mode`, vport `min_wqe_inline_mode`).
7. **Lkey**: reserved LKey (§6.3) is "programmed by the Level 0 privilege
   agent" and read by QUERY_SPECIAL_CONTEXTS (0x203) with `rlkey` set in the
   SQ/RQ ctx; whether a bare-metal PF gets a usable value, or must
   CREATE_MKEY a PA-mode key instead.
8. **Barriers**: the PRM orders WQE → DBR → doorbell but specifies no fence
   instructions; only the CQE re-read barrier is explicit.
9. **WQ `page_offset` units**: "(page_size/64) quanta" in 5 bits vs Fig 12
   `Page_offset[11:6]` (use 0).
10. **PAS alignment** for `log_page_size > 0` (use 4 KB pages).
11. **128-byte CQE** layout (where the 64-byte part sits).
12. **CQE `rx_drop_counter`** has no position in T67; no QPN in the CQE.
13. **CQE `wqe_counter` for multi-WQEBB sends**: assumed = WQE Index of the
    completed WQE's first WQEBB.
14. **Receive `start_padding`** bit position (not drawn in T40).
15. **FCS** scattered to the receive buffer or not.
16. **Port-change subtypes** 0x1/0x4/0x5: which means up/down.
17. **Event bitmask bit numbering** across the two dwords at 58h/5Ch.
18. **Destroy order / states**: whether DESTROY_SQ/RQ from RDY is fine and
    whether DESTROY_EQ needs events unmapped first (§12.6 says both "SW must
    unmap" and "destroying the EQ unmaps").
19. **TIS `prio`** "prio[3:1]" in a 4-bit field (use 0).
20. **`c_eqn`** must name a valid EQ even when the CQ is never armed
    (assumed yes).

### PRM inconsistencies found

- T13: BlueFlame Buffer0_even printed "800-0FCh" (= 800h–8FCh); §7.2.2.1
  mentions BF registers 2 and 3 that T13 does not show.
- T54 places `page_offset`/`lwm` at 04h; T55 lists them under 00h.
- `uar_page`: T55 marks RQ only; T62 requires it for SQ; T56 says N/A for RQ.
- T56/T62 name fields absent from the context tables (`cs`, `rq_index`,
  `vsd`, `sq_index`, `cd_slave`, `packet_pacing_rate_limit_index`).
- T68 lists `rx_drop_counter`, T67 has no slot for it.
- EQE `event_data` sub-tables use offsets relative to event_data without
  saying so.
- T88 lists three port-change subtypes with one description.
- T260 (CREATE_CQ) skips 50h–54h ("58h-10Ch").
- §7.4.4.1 cross-references "Section 9.3.4.1.5, Eth Segment… page 140"
  (stale; it is §7.4.4.1.2).
- §7.2.2.2 says RQ doorbells need 64-bit writes, but an RQ has no UAR
  doorbell (§7.4.3.1).
- QUERY_TIS output places the TIS ctx at 10h; CREATE/MODIFY_TIS at 20h.
- §7.12.1 says the CQ buffer contains the CQ doorbell record; CQC has a
  separate `dbr_addr`.
- §12.6 says SW must unmap events before DESTROY_EQ and also that
  DESTROY_EQ unmaps them.
- `wqe_inline_mode` (HCA_CAP: 0 L2, 1 vport, 2 none) and
  `min_wqe_inline_mode` (0 none, 1 L2, 2 IP, …) are different enums with
  similar names.
