# Command interface and bring-up

Source: *Mellanox Adapters Programmer's Reference Manual (PRM) Supporting
ConnectX-4 and ConnectX-4 Lx*, Rev 0.40 (MLNX-15-4845). Nothing else. Where the
PRM is silent the text says **HW-CHECK**; the full list is in §9.

Conventions used below:

- **Byte order** (PRM "Byte Endianness", p.25): every structure is big-endian.
  In a dword at offset *o*, bits 31:24 are byte *o*+0, 23:16 byte *o*+1, 15:8
  byte *o*+2, 7:0 byte *o*+3. Bit numbers below are the PRM's (31..0 within the
  big-endian dword). A 64-bit field spanning *o*..*o*+7 is big-endian too
  (high dword first). PCI config space is little-endian (same page).
- **Reserved** fields: written as 0, ignored on read (p.25; PRM 12.1).
- Offsets of capability fields (§6.8) are relative to the start of the
  capability structure, i.e. command-output offset = 0x10 + cap offset.
- "Len" for a command = byte length of the input / output as given by the last
  row of the PRM layout table (PRM gives no explicit lengths).
- PCI IDs (PRM Table 1): ConnectX-4 PF 4115 (0x1013), VF 4116, recovery 521;
  ConnectX-4 Lx PF 4117 (0x1015), VF 4118 (0x1016), recovery 523 (0x020B).

---

## 1. HCA BAR, initialization segment, UAR

### 1.1 Which BAR (PRM 4, p.47; 8.1.2; 8.2)

- Each function exposes a single **HCA BAR**, prefetchable; "This is Bar0 in
  the PCI header." Both the initialization segment and the UAR pages are in it.
- Initialization segment is at **offset 0 of the HCA BAR** (PRM 4.3).
- Before using the command interface the driver must set **Bus Master** in the
  PCI Command register (PRM 8.1.2, note on p.151).

### 1.2 Initialization segment (PRM 4.3, Table 7 layout, Table 8 descriptions)

| Offset | Bits | Name | Acc | Meaning |
|---|---|---|---|---|
| 0x0000 | 31:16 | fw_rev_minor | RO | Firmware revision, minor |
| 0x0000 | 15:0 | fw_rev_major | RO | Firmware revision, major |
| 0x0004 | 31:16 | cmd_interface_rev | RO | Command-interface revision; bumped on every non-backward-compatible change. Driver "must match" it (8.2). Expected value: **PRM does not say — HW-CHECK** |
| 0x0004 | 15:0 | fw_rev_subminor | RO | Firmware sub-minor (patch) |
| 0x0008–0x000C | — | reserved | | |
| 0x0010 | 31:0 | cmdq_phy_addr[63:32] | RW | Physical address of the command queue, high. Not to be modified between INIT_HCA and TEARDOWN_HCA |
| 0x0014 | 31:12 | cmdq_phy_addr[31:12] | RW | Physical address, low (so 4 KB aligned) |
| 0x0014 | 11:10 | — | | reserved (blank in layout) |
| 0x0014 | 9:8 | nic_interface | WO | 0x0 full_driver, 0x1 disabled. 8.2: must be written 0 |
| 0x0014 | 7:4 | log_cmdq_size | RO | log2 number of command-queue entries |
| 0x0014 | 3:0 | log_cmdq_stride | RO | "Stride between start of each cmdq" (log2; unit **not stated — HW-CHECK**, presumably bytes) |
| 0x0018 | 31:0 | command DoorBell vector | WO | Bit *n* = entry *n*. Writing a 1 moves that entry to HW ownership. Valid bits [num_entries-1..0]. Several bits may be set in one write (7.14.1). Must not be written in "No DRAM NIC mode" |
| 0x001C–0x01F8 | — | reserved | | |
| 0x01FC | 31 | initializing | RO | 1 = device still initializing; 0 = ready for commands |
| 0x01FC | 26:24 | nic_interface_supported | RO | Bitmask of supported nic_interface modes: bit 0 full_driver, bit 1 disabled |
| 0x0200–0x0FFC | — | (no fields described) | | |
| 0x1000 | 31:0 | internal_timer_h | RO | Internal timer MSBs (7.12.10) |
| 0x1004 | 31:0 | internal_timer_l | RO | Internal timer LSBs |
| 0x1008 | — | reserved | | |
| 0x100C | 0 | clear_int | WO | Write 1 to clear the interrupt (always INTA) |
| 0x1010 | 31:24 | health_syndrome | RO | 0x1 FW_INTERNAL_ERR (FW assert), 0x7 DEAD_IRISC, 0x8 HW_FATAL_ERR, 0x9 FW_CRC_ERR, 0xA ICM_FETCH_PCI_ERR, 0xB ICM_PAGE_ERR, 0xC ASYNCHRONOUS_EQ_BUF_OVERRUN, 0xD EQ_IN_ERR, 0xE EQ_INV, 0xF FFSER_ERR, 0x10 HIGH_TEMP_ERR |
| 0x1010 | ~23:0 | health counter | | Shown in Table 7 layout only, no description; bit range read off the layout drawing (≈23:0) — **HW-CHECK** |
| 0x1014–0x4008 | — | (rows drawn, no fields described) | | |

Notes:

- 8.2: "If using 32-bit writes, write the most significant word first" — write
  0x0010 (addr[63:32]) then 0x0014 (addr[31:12] | nic_interface=0; the RO
  log_cmdq_size/stride bits written as 0).
- Internal-error buffer (PRM 7.13.9.1): "located on the initialization segment";
  an error shows as a non-zero first DWORD; SW should poll it periodically
  (interrupt not guaranteed). Its exact offset/size is not given beyond
  health_syndrome at 0x1010 — **HW-CHECK**.
- Reset: the PRM gives **no** reset register in the initialization segment and
  no software-reset procedure. On internal error "hardware reset should be
  performed" (7.13.9.1); FFSER "Reset is required" (Table 103). How a driver
  recovers a function left initialized by an earlier owner (e.g. firmware's
  own driver) — **PRM does not say — HW-CHECK** (PCIe FLR is outside the PRM).

### 1.3 UAR location (PRM 7.2, Figure 11, Table 13; HCA cap fields)

- 7.2.1: the UAR is "a single BAR ... per each function that contains multiple
  pages; page size should match the CPU (system) page size, and should be
  configured at device initialization". Figure 11 ("HCA BAR Structure") shows
  "UAR DB page i … UAR DB page N-1" indexed by "UAR page number".
- Page size knob: HCA cap `log_uar_page_sz` (0x90 15:0, RW, "Log (base 2) of
  UAR page in 4Kbyte chunks"). Area size: `uar_sz` (0x48 21:16, "UAR Area Size
  = 1MB * 2^uar_sz").
- ALLOC_UAR returns a UAR number (§6.13). The formula `BAR0 + uar *
  uar_page_size` and whether UAR page 0 overlaps the initialization segment at
  BAR offset 0 are **not stated (Figure 11 is not legible in text) — HW-CHECK**.
- UAR page format (Table 13), offsets within one UAR page (field bits are a
  layout reading, for the datapath spec to confirm):

| Offset | Content |
|---|---|
| 0x000–0x01C | reserved |
| 0x020 | CQ doorbell: cmdsn, cmd, cq_ci (cq_ci in low 24 bits) |
| 0x024 | CQ doorbell: cqn (low 24 bits) |
| 0x028–0x03C | reserved |
| 0x040 | EQ doorbell "update CI and Arm": eqn (high byte), consumer index (low 24 bits) |
| 0x044 | reserved |
| 0x048 | EQ doorbell "update CI" (no arm): eqn, consumer index |
| 0x04C–0x7FC | reserved |
| 0x800–0x8FC | BlueFlame register 0, buffer even |
| 0x900–0x9FC | BlueFlame register 0, buffer odd |
| 0xA00–0xAFC | BlueFlame register 1, buffer even |
| 0xB00–0xBFC | BlueFlame register 1, buffer odd |

  "The DoorBell register in the UAR is the first 2 DWORDS of the blue-flame
  buffer" (7.2); send doorbells are the first 8 bytes of the WQE written to
  offset 0 of a BlueFlame register (7.2.2).

---

## 2. Command queue

### 2.1 Queue (PRM 7.14.1)

- One command queue per function; entries are fixed size. Number of entries =
  2^log_cmdq_size, stride from log_cmdq_stride (init segment 0x14).
- Resides in a **physically contiguous 4 KB chunk aligned on its size**; its
  address is written to init segment 0x10/0x14.
- SW posts an entry, then writes the DoorBell vector with that entry's bit set.
  HW may complete out of order. On completion HW clears the entry's ownership
  bit (and raises an EQE if an EQ is mapped for command completions).
- **Polling**: "SW can poll the ownership bit in the command to understand when
  the command is completed." Before an EQ exists polling is the only way. A
  CREATE_EQ that maps command-completion events must be posted while the
  queue is empty with no other command outstanding.
- SW must zero `status` and all reserved fields when posting.
- First 16 bytes of input are inline in the entry (0x10–0x1F); the rest goes to
  the input mailbox chain. First 16 bytes of output are inline (0x20–0x2F); the
  rest comes back in the output mailbox chain. (Implication, not stated as a
  formula: input byte 16+k → input mailbox chain data byte k; block *b* holds
  data bytes 512·b .. 512·b+511.)

### 2.2 Command Queue Entry (PRM Table 95 layout, Table 96 descriptions) — 64 bytes

| Offset | Bits | Name | Meaning |
|---|---|---|---|
| 0x00 | 31:24 | type | Transport type; **0x7 = PCIe_cmd_if_transport** |
| 0x00 | 23:0 | — | reserved |
| 0x04 | 31:0 | input_length | Input command length in bytes (whole command incl. 16-byte header) |
| 0x08 | 31:0 | input mailbox pointer[63:32] | |
| 0x0C | 31:9 | input mailbox pointer[31:9] | First input mailbox block (512-byte aligned per field width) |
| 0x0C | 8:0 | — | reserved |
| 0x10–0x1C | 128 bits | command_input_inline_data | First 16 bytes of the command input |
| 0x20–0x2C | 128 bits | command_output_inline_data | First 16 bytes of the command output |
| 0x30 | 31:0 | output mailbox pointer[63:32] | |
| 0x34 | 31:9 | output mailbox pointer[31:9] | |
| 0x34 | 8:0 | — | reserved |
| 0x38 | 31:0 | output_length | Output command length in bytes |
| 0x3C | 31:24 | token | Same value in the entry and in every mailbox block |
| 0x3C | 23:16 | signature | "8 bit signature of the command queue entry, does not include the mailbox" (see 2.4) |
| 0x3C | 15:8 | — | reserved |
| 0x3C | 7:1 | status | **Delivery** status, SW writes 0 (table 2.5) |
| 0x3C | 0 | ownership | SW sets **1** when posting; HW sets **0** to return the entry to SW |

### 2.3 Mailbox block (PRM 7.14.1.1, Table 97 layout, Table 98 descriptions) — 0x240 bytes

| Offset | Bits | Name | Meaning |
|---|---|---|---|
| 0x000–0x1FC | 4096 bits | mailbox data | 512 bytes of command data |
| 0x200–0x22C | — | reserved | |
| 0x230 | 31:0 | next_pointer[63:32] | Next block; 0 if none |
| 0x234 | 31:10 | next_pointer[31:10] | Next block (1 KB aligned per field width); 0 if none |
| 0x234 | 9:0 | — | reserved |
| 0x238 | 31:0 | block number | 0 for the first block, +1 per block in the chain |
| 0x23C | 31:24 | — | reserved |
| 0x23C | 23:16 | token | Same as the entry's token |
| 0x23C | 15:8 | ctrl_signature | see 2.4 |
| 0x23C | 7:0 | signature | see 2.4 |

Alignment: the entry's mailbox pointers drop bits 8:0 (512 B), next_pointer
drops bits 9:0 (1 KB); delivery status 0x4/0x5 say "pointer not aligned to
mailbox size". Allocating every block 4 KB-aligned satisfies all of them
(our choice; exact rule **HW-CHECK**).

### 2.4 Signatures (PRM 7.14.1.1.1, Table 96, Table 98, 9.2.1)

Mailbox block, PRM 7.14.1.1.1 verbatim steps:

1. Fill all fields of the block.
2. Write 0 to `ctrl_signature` and `signature`.
3. XOR all bytes of "the ctrl part (bytes 1C0h – 1FFh)"; invert; write to
   `ctrl_signature`.
4. XOR all bytes of the whole block; invert; write to `signature`.
5. Before posting: compute the entry's signature and the input mailboxes'
   signatures, and the **ctrl_signature of the output mailbox blocks** ("to
   protect the output mailbox pointers and token").

Table 98 restates them as checks: ctrl_signature — "dwords 1D0h–1FCh ... XOR of
all those DWORDs including the ctrl_signature should result in 0xFF (while the
signature byte is considered as 0)"; signature — "XOR of all bytes of the page,
including the reserved fields and the signature fields should result in 0xFF".

**Conflict:** 1C0h–1FFh (7.14.1.1.1) and 1D0h–1FCh (Table 98) both lie inside the
data area, while the ctrl fields (next_pointer, block number, token) are at
0x230–0x23F. Each range is exactly 0x40 too low compared to 0x200–0x23F /
0x210–0x23C. Since 0x200–0x22F is reserved (zero), the reading "ctrl part =
block trailer 0x200–0x23F" gives

```
ctrl_signature = ~XOR(bytes 0x230..0x23F, with byte 0x23E = 0 and 0x23F = 0)
signature      = ~XOR(bytes 0x000..0x23F, with 0x23F = 0, 0x23E already set)
```

The literal reading (data bytes 0x1C0–0x1FF) gives a different value — **HW-CHECK**.

Entry signature: the PRM gives no algorithm for the 64-byte entry beyond
"8 bit signature ... does not include the mailbox" (Table 96). 9.2.1 says the
command is sealed "with an 8-bit XOR signature" and, contradicting Table 96,
that the input mailbox "is also protected by the command signature". By analogy
with the mailbox: `signature = ~XOR(bytes 0x00..0x3F with signature = 0)` —
**HW-CHECK** (and whether status/ownership are included).

Whether signatures are checked at all is controlled by HCA cap
`cmdif_checksum` (0x40 15:14, RW): 0 disabled (neither checked nor generated);
1 initial_state (input not checked, output generated); 3 enabled (both). Its
power-on value is **not stated — HW-CHECK** (the name suggests 1). Computing
signatures always is harmless.

### 2.5 Delivery status (entry 0x3C bits 7:1, PRM Table 96)

| Value | Meaning |
|---|---|
| 0x0 | no errors |
| 0x1 | signature error |
| 0x2 | token error |
| 0x3 | bad block number |
| 0x4 | bad output pointer (not aligned to mailbox size) |
| 0x5 | bad input pointer (not aligned to mailbox size) |
| 0x6 | internal error |
| 0x7 | input len error (input length < 0x8) |
| 0x8 | output len error (output length < 0x8) |
| 0x9 | reserved not zero |
| 0x10 | bad command type |

Value is `(byte 0x3F >> 1) & 0x7F`.

### 2.6 Command input / output header (PRM 7.14.1.2, Tables 99–102)

Input (Table 99/100):

| Offset | Bits | Name |
|---|---|---|
| 0x00 | 31:16 | opcode |
| 0x00 | 15:0 | reserved |
| 0x04 | 31:16 | reserved |
| 0x04 | 15:0 | op_mod — "if there is no description, then op_mod must be 0" |
| 0x08… | 31:0 | command[...] |

Output (Table 101/102):

| Offset | Bits | Name |
|---|---|---|
| 0x00 | 31:24 | status (command return status, 2.7) |
| 0x00 | 23:0 | reserved |
| 0x04 | 31:0 | syndrome — Table 102 says "Valid only if status = 0x0" (almost certainly inverted; treat as diagnostic when status ≠ 0) |
| 0x08… | 31:0 | command output — valid only if status = 0 (also 12.1: on failure output data is undefined) |

### 2.7 Command return status (PRM 12.2, Table 128)

| Value | Name | Meaning |
|---|---|---|
| 0x00 | OK | succeeded |
| 0x01 | INTERNAL_ERR | internal error (e.g. bus error) |
| 0x02 | BAD_OP | opcode or op_mod not supported |
| 0x03 | BAD_PARAM | parameter unsupported/out of range, reserved ≠ 0 |
| 0x04 | BAD_SYS_STATE | system not enabled / bad system state |
| 0x05 | BAD_RESOURCE | reserved/unallocated resource or resource in wrong state |
| 0x06 | RESOURCE_BUSY | resource is executing a command; nothing changed |
| 0x08 | EXCEED_LIM | requested capability exceeds device limits |
| 0x09 | BAD_RES_STATE | resource not in appropriate state/ownership |
| 0x0A | BAD_INDEX | index out of range |
| 0x0F | NO_RESOURCES | lack of resources (e.g. ICM pages); unrecoverable for the driver |
| 0x10 | BAD_RESOURCE_STATE | (RQ/SQ/TIR/TIS) resource not in presumed state |
| 0x40 | BAD_SIZE | (CQ) more outstanding CQEs than new CQ size |
| 0x50 | BAD_INPUT_LEN | bad command input length |
| 0x51 | BAD_OUTPUT_LEN | bad command output length |

HW need not check everything (BAD_SYS_STATE, BAD_RES_STATE, BAD_INDEX,
INTERNAL_ERR may not be returned); SW is expected to validate.

### 2.8 Polling procedure (assembled from 7.14.1, Table 96, Table 98)

1. Pick a free entry *n* (ownership = 0 / never used). Zero all 64 bytes.
2. Build the input: 16 bytes inline at 0x10; remainder into mailbox blocks
   (block_number 0,1,…; next_pointer chained, 0 on the last; token in each).
3. Build output blocks if output_length > 16: zero them, chain, set token and
   block number, compute their ctrl_signature.
4. Set type=0x7, input_length, output_length, mailbox pointers, token,
   status=0, ownership=1; compute signatures (input blocks, then entry).
5. Memory barrier, then write `1 << n` to init segment 0x0018.
6. Poll entry byte 0x3F bit 0 until 0 (timeout: **PRM gives none — HW-CHECK**).
7. Check delivery status (2.5) ≠ 0 → transport failure. Else read output
   `status` (entry byte 0x20) and `syndrome` (0x24..0x27); output bytes 16+ are
   in the output mailbox chain.

---

## 3. Opcodes (PRM 12.1, Table 127 — complete list)

| Opcode | Command | PRM § |
|---|---|---|
| 0x100 | QUERY_HCA_CAP | 12.3.3 |
| 0x101 | QUERY_ADAPTER | 12.3.5 |
| 0x102 | INIT_HCA | 12.3.6 |
| 0x103 | TEARDOWN_HCA | 12.3.7 |
| 0x104 | ENABLE_HCA | 12.3.8 |
| 0x105 | DISABLE_HCA | 12.3.9 |
| 0x107 | QUERY_PAGES | 12.3.1 |
| 0x108 | MANAGE_PAGES | 12.3.2 |
| 0x109 | SET_HCA_CAP | 12.3.4 |
| 0x10A | QUERY_ISSI | 12.3.10 |
| 0x10B | SET_ISSI | 12.3.11 |
| 0x10D | SET_DRIVER_VERSION | 12.3.12 |
| 0x200 | CREATE_MKEY | 12.5.1 |
| 0x201 | QUERY_MKEY | 12.5.2 |
| 0x202 | DESTROY_MKEY | 12.5.3 |
| 0x203 | QUERY_SPECIAL_CONTEXTS | 12.5.4 |
| 0x301 | CREATE_EQ | 12.6.1 |
| 0x302 | DESTROY_EQ | 12.6.2 |
| 0x303 | QUERY_EQ | 12.6.3 |
| 0x304 | GEN_EQE | 12.6.4 |
| 0x400 | CREATE_CQ | 12.7.1 |
| 0x401 | DESTROY_CQ | 12.7.2 |
| 0x402 | QUERY_CQ | 12.7.3 |
| 0x403 | MODIFY_CQ | 12.7.4 |
| 0x750 | QUERY_VPORT_STATE | 12.16.1 |
| 0x751 | MODIFY_VPORT_STATE | 12.16.2 |
| 0x754 | QUERY_NIC_VPORT_CONTEXT | 12.16.3 |
| 0x755 | MODIFY_NIC_VPORT_CONTEXT | 12.16.4 |
| 0x770 | QUERY_VPORT_COUNTER | 12.17.1 |
| 0x800 | ALLOC_PD | 12.3.13 |
| 0x801 | DEALLOC_PD | 12.3.14 |
| 0x802 | ALLOC_UAR | 12.3.15 |
| 0x803 | DEALLOC_UAR | 12.3.16 |
| 0x804 | CONFIG_INT_MODERATION | 12.3.17 |
| 0x805 | ACCESS_REG | 12.4.1 |
| 0x80D | NOP | 12.18.1 |
| 0x816 | ALLOC_TRANSPORT_DOMAIN | 12.3.18 |
| 0x817 | DEALLOC_TRANSPORT_DOMAIN | 12.3.19 |
| 0x829 | SET_L2_TABLE_ENTRY | 12.15.1 |
| 0x82A | QUERY_L2_TABLE_ENTRY | 12.15.2 |
| 0x82B | DELETE_L2_TABLE_ENTRY | 12.15.3 |
| 0x900 | CREATE_TIR | 12.8.1 |
| 0x901 | MODIFY_TIR | 12.8.2 |
| 0x902 | DESTROY_TIR | 12.8.3 |
| 0x903 | QUERY_TIR | 12.8.4 |
| 0x904 | CREATE_SQ | 12.10.1 |
| 0x905 | MODIFY_SQ | 12.10.2 |
| 0x906 | DESTROY_SQ | 12.10.3 |
| 0x907 | QUERY_SQ | 12.10.4 |
| 0x908 | CREATE_RQ | 12.11.1 |
| 0x909 | MODIFY_RQ | 12.11.2 |
| 0x90A | DESTROY_RQ | 12.11.3 |
| 0x90B | QUERY_RQ | 12.11.3 (sic) |
| 0x90C | CREATE_RMP | 12.13.1 |
| 0x90D | MODIFY_RMP | 12.13.2 |
| 0x90E | DESTROY_RMP | 12.13.3 |
| 0x90F | QUERY_RMP | 12.13.4 |
| 0x912 | CREATE_TIS | 12.9.1 |
| 0x913 | MODIFY_TIS | 12.9.2 |
| 0x914 | DESTROY_TIS | 12.9.3 |
| 0x915 | QUERY_TIS | 12.9.4 |
| 0x916 | CREATE_RQT | 12.12.1 |
| 0x917 | MODIFY_RQT | 12.12.2 |
| 0x918 | DESTROY_RQT | 12.12.3 |
| 0x919 | QUERY_RQT | 12.12.4 |
| 0x92F | SET_FLOW_TABLE_ROOT | 12.14.4 |
| 0x930 | CREATE_FLOW_TABLE | 12.14.1 |
| 0x931 | DESTROY_FLOW_TABLE | 12.14.3 |
| 0x932 | QUERY_FLOW_TABLE | 12.14.5 |
| 0x933 | CREATE_FLOW_GROUP | 12.14.6 |
| 0x934 | DESTROY_FLOW_GROUP | 12.14.7 |
| 0x935 | QUERY_FLOW_GROUP | 12.14.8 |
| 0x936 | SET_FLOW_TABLE_ENTRY | 12.14.9 |
| 0x937 | QUERY_FLOW_TABLE_ENTRY | 12.14.10 |
| 0x938 | DELETE_FLOW_TABLE_ENTRY | 12.14.11 |
| 0x939 | ALLOC_FLOW_COUNTER | 12.14.12 |
| 0x93A | DEALLOC_FLOW_COUNTER | 12.14.13 |
| 0x93B | QUERY_FLOW_COUNTER | 12.14.14 |
| 0x93C | MODIFY_FLOW_TABLE | 12.14.2 |

---

## 4. Bring-up and teardown order

### 4.1 Start-up (PRM 8.2, summarised step by step)

0. (8.1.2) PCI enumeration done, BAR assigned; set Bus Master.
1. Read init segment (offset 0 of HCA BAR): firmware and command-interface
   revisions; driver must match `cmd_interface_rev`.
2. Write the command-queue physical address (0x10 then 0x14 if 32-bit writes);
   `nic_interface`, `log_cmdq_size`, `log_cmdq_stride` written as 0.
3. Poll `initializing` (0x1FC bit 31) until 0.
4. ENABLE_HCA.
5. QUERY_ISSI (7.1). 6. SET_ISSI.
7. QUERY_PAGES (boot) — HCA's need for boot pages.
8. MANAGE_PAGES — give all boot pages; "allowed to give the sum of boot pages
   and num_init_pages".
9. QUERY_HCA_CAP — device limits.
10. SET_HCA_CAP — modify capabilities.
11. QUERY_PAGES (init) — if 0, skip the next step.
12. MANAGE_PAGES — all init pages (may take several commands).
13. INIT_HCA.
14. SET_DRIVER_VERSION — only if HCA_CAP.driver_version == 1.
15. CREATE_EQ, mapping the PAGE_REQUEST event to it.
16. QUERY_VPORT_STATE.
17. Ethernet: QUERY_NIC_VPORT_CONTEXT for the permanent MAC (14.1.7).
18. MODIFY_NIC_VPORT_CONTEXT to set the current MAC.

### 4.2 Teardown (PRM 8.3)

1. Stop all operations of the function; destroy all resources.
2. Unmap the EQ that PAGE_REQUEST events are mapped to; destroy it.
3. TEARDOWN_HCA.
4. MANAGE_PAGES (op_mod HCA_RETURN_PAGES) in a loop until all pages are reclaimed.
5. DISABLE_HCA.

"After TEARDOWN_HCA is acknowledged by the hardware, HCA will not attempt to
access PCI on behalf of that ICM." When a PF is shut down, its VFs are too.

---

## 5. ISSI (PRM 7.1, 12.3.10, 12.3.11)

- ISSI of this PRM revision is **1**.
- SW must QUERY_ISSI then SET_ISSI with `actual_issi = min(sw_issi,
  max supported_issi)`.
- If QUERY_ISSI fails with "BAD_OPCODE" (i.e. BAD_OP 0x02, the name in Table
  128), only ISSI 0 is supported and SET_ISSI is not needed.
- ISSI history (7.1.1.1), the only change listed for ISSI = 1: **`cqe_version`
  added to HCA_CAP** (0x3C 3:0); SW must read it to know the CQE format. The
  PRM lists no other difference between ISSI 0 and 1.

---

## 6. Command layouts

Header rows (0x00 opcode 31:16; 0x04 op_mod 15:0; output 0x00 status 31:24,
0x04 syndrome) are the same in every command and are omitted unless they carry
something. Every row not listed is reserved.

### 6.1 ENABLE_HCA 0x104 (PRM 12.3.8, Tables 173–176)

In: 0x00–0x0C, only opcode/op_mod (op_mod 0). Len in 16.
Out: 0x08 reserved. Len out 12 (layout ends at 0x08).

### 6.2 DISABLE_HCA 0x105 (PRM 12.3.9, Tables 177–180)

In: only opcode/op_mod; layout 0x00–0x0C. Len in 16. Out: 0x08 reserved; len 12.

### 6.3 QUERY_ISSI 0x10A (PRM 12.3.10, Tables 181–184)

In: opcode/op_mod, 0x08–0x0C reserved. Len in 16.

| Out offset | Bits | Name | Meaning |
|---|---|---|---|
| 0x08 | 15:0 | current_issi | ISSI currently in use |
| 0x0C–0x1C | — | reserved | |
| 0x20–0x6C | 640 bits | supported_issi | Bitmask; bit N set ⇒ ISSI N supported |

Len out 0x70. Bit position: by the PRM's bit-numbering convention bit 0 of a
640-bit field is bit 0 of its last dword, i.e. **byte 0x6F bit 0 = ISSI 0,
byte 0x6F bit 1 = ISSI 1**. The PRM does not say this explicitly — **HW-CHECK**.

### 6.4 SET_ISSI 0x10B (PRM 12.3.11, Tables 185–188)

| In offset | Bits | Name | Meaning |
|---|---|---|---|
| 0x08 | 15:0 | current_issi | ISSI to work with (recommended min(mine, max supported)) |
| 0x0C | — | reserved | |

Len in 16, out 16.

### 6.5 QUERY_PAGES 0x107 (PRM 12.3.1, Tables 130–133)

| In offset | Bits | Name | Meaning |
|---|---|---|---|
| 0x04 | 15:0 | op_mod | **0x1 boot_pages, 0x2 init_pages, 0x3 regular_pages** |

| Out offset | Bits | Name | Meaning |
|---|---|---|---|
| 0x08 | — | reserved | |
| 0x0C | 31:0 | num_pages | Signed 32-bit, 4 KB pages. Table 133: **positive ⇒ driver should provide pages; negative ⇒ driver should reclaim** |

Len in 16, out 16. Note: the 12.3.1 prose says the opposite ("A positive number
indicates ... spare pages that can be reclaimed"); Table 133 and the Page
Request event (Table 92: "missing / unneeded pages (signed number, msb indicate
sign)") agree with the table reading — use Table 133; **HW-CHECK** on first run.

### 6.6 MANAGE_PAGES 0x108 (PRM 12.3.2, Tables 134–139)

Passes 4 KB pinned physical pages to the HCA or returns them. Used before
INIT_HCA for boot/init pages; afterwards once per Page Request event (it also
re-arms that event).

| In offset | Bits | Name | Meaning |
|---|---|---|---|
| 0x04 | 15:0 | op_mod | **0x0 ALLOCATION_FAIL** (SW cannot give pages; no mailbox valid) · **0x1 ALLOCATION_SUCCESS** (SW gives pages; input valid) · **0x2 HCA_RETURN_PAGES** (SW asks for pages back; input param, output param and output mailbox valid) |
| 0x08 | — | reserved | |
| 0x0C | 31:0 | input_num_entries | Number of valid PAS entries (for op_mod 2: presumably the number of pages requested back — **HW-CHECK**) |
| 0x10 + 8·i | 64 | pas[i] | PAS (below) |

| Out offset | Bits | Name | Meaning |
|---|---|---|---|
| 0x08 | 31:0 | output_num_entries | Number of returned PAS entries (op_mod 2) |
| 0x0C | — | reserved | |
| 0x10 + 8·i | 64 | pas[i] | Returned page addresses |

PAS (Table 138/139), 8 bytes:

| Offset | Bits | Name |
|---|---|---|
| 0x00 | 31:0 | pa_h = PA[63:32] |
| 0x04 | 31:12 | pa_l = PA[31:12] (bits 11:0 reserved ⇒ 4 KB aligned) |

Len in = 0x10 + 8·n; len out = 0x10 (give/fail) or 0x10 + 8·n (return).
With 512-byte mailbox data, one block holds 64 PAS entries; the first PAS is
at input byte 0x10, i.e. mailbox block 0 byte 0. Max entries per command:
**PRM does not say — HW-CHECK**.

### 6.7 QUERY_HCA_CAP 0x100 (PRM 12.3.3, Tables 140–143)

| In offset | Bits | Name | Meaning |
|---|---|---|---|
| 0x04 | 0 | op_mod[0] | 0 = Maximum, 1 = Current |
| 0x04 | 15:1 | op_mod[15:1] | Capability type: **0x0 General Device**, **0x1 Ethernet Offload**, **0x7 NIC Flow Table** |

So op_mod = (type << 1) | cur: general max 0x0000, general cur 0x0001,
ethernet cur 0x0003, flow-table cur 0x000F.

| Out offset | Bits | Name |
|---|---|---|
| 0x08–0x0C | — | reserved |
| 0x10–0x100C | 32768 bits | capability (4096 bytes, layout per type) |

Len in 16, out 0x1010 (eight mailbox blocks after the 16 inline bytes).

### 6.8 HCA Device Capabilities (PRM 12.3.3.1, Table 144 layout, Table 145 descriptions)

Offsets relative to the capability structure. Access: blank = RO, RW as marked.

| Offset | Bits | Name | Acc | Meaning |
|---|---|---|---|---|
| 0x00–0x14 | — | reserved | | |
| 0x18 | 23:16 | log_max_cq_sz | | log2 max CQEs per CQ |
| 0x18 | 4:0 | log_max_cq | | log2 max number of CQs |
| 0x1C | 31:24 | log_max_eq_sz | | log2 max EQEs per EQ |
| 0x1C | 21:16 | log_max_mkey | | log2 max data MKeys |
| 0x1C | 3:0 | log_max_eq | | log2 max number of EQs |
| 0x20 | 31:24 | max_indirection | | Max MKey indirection level |
| 0x20 | 22:16 | log_max_mrw_sz | | log2 max Memory Region/Window size |
| 0x20 | 5:0 | log_max_klm_list_size | | log2 max indirect KLM list |
| 0x24–0x28 | — | reserved | | |
| 0x2C | 31 | end_pad | | RX end padding supported (3.3.10) |
| 0x2C | 28 | start_pad | | RX start padding supported (3.3.9) |
| 0x2C | 27 | cache_line_128byte | RW | 1 = 128 B cache line supported; 0 = only 64 B |
| 0x30 | 31 | reserved | | |
| 0x30 | 30 | vport_counters | | Vport counters supported (12.17) |
| 0x34 | 31 | vport_group_manager | | Vport group manager (enables other vports) |
| 0x34 | 25 | nic_flow_table | | NIC flow tables supported; query type 0x7 |
| 0x34 | 9:8 | port_type | | 0x1 = Ethernet |
| 0x34 | 7:0 | num_ports | | Number of network ports |
| 0x38 | 31 | reserved | | |
| 0x38 | 28:24 | log_max_msg | | log2 max message size (bytes) |
| 0x38 | 19:16 | max_tc | | Number of TCs (0 = 8) |
| 0x38 | 15 | temp_warn_event | | Temperature warning event supported |
| 0x3C | 31:16 | stat_rate_support | | In Table 144 layout; Table 145 calls 31:16 "reserved" and gives no description — **HW-CHECK** |
| 0x3C | 3:0 | cqe_version | | CQE version (Table 67); added with ISSI 1 |
| 0x40 | 31 | reserved | | |
| 0x40 | 15:14 | cmdif_checksum | RW | 0 disabled, 1 initial_state (out generated, in not checked), 3 enabled |
| 0x40 | 11 | wq_signature | | WQE signature check on SQ |
| 0x40 | 10 | sctr_data_cqe | | Scatter data to CQE supported |
| 0x40 | 3 | eth_net_offloads | | Ethernet offloads supported; query type 0x1 |
| 0x44 | 31 | cq_oi | | cq.oi modifiable by MODIFY_CQ |
| 0x44 | 30 | cq_resize | | CQ resize supported |
| 0x44 | 29 | cq_moderation | | CQ moderation via MODIFY_CQ |
| 0x44 | 25 | cq_eq_remap | | cq.eqn modifiable by MODIFY_CQ |
| 0x44 | 21 | scqe_break_moderation | | Moderation broken by solicited CQE |
| 0x44 | 20 | cq_period_start_from_cqe | | cq_period_mode = 1 supported |
| 0x48 | 21:16 | uar_sz | | UAR area size = 1 MB · 2^uar_sz |
| 0x48 | 7:0 | log_pg_sz | | log2 **minimum** system page size supported; must be ≤ host page size |
| 0x4C | 31 | bf | | BlueFlame may be used |
| 0x4C | 30 | driver_version | | SET_DRIVER_VERSION supported and should be used by the start-up driver |
| 0x4C | 29 | pad_tx_eth_packet | | Device pads TX Ethernet frames < 64 B to 64 B |
| 0x4C | 20:16 | log_bf_reg_size | | log2 BlueFlame register size (bytes) |
| 0x50–0x5C | — | reserved | | |
| 0x60 | 31:16 | reserved | | |
| 0x64 | 28:24 | log_max_transport_domain | | log2 max transport domains |
| 0x64 | 20:16 | log_max_pd | | log2 max PDs |
| 0x68 | 23:16 | log_max_flow_counter_bulk | | log2 max counters per QUERY_FLOW_COUNTER |
| 0x68 | 15:0 | max_flow_counter | | Max flow counters |
| 0x6C | 31 | modify_tis | | MODIFY_TIS supported |
| 0x6C | 28:24 | log_max_rq | | log2 number of RQs (0 = unsupported) |
| 0x6C | 20:16 | log_max_sq | RW (sic) | log2 number of SQs (0 = unsupported). The RW mark on this row is probably misplaced |
| 0x6C | 12:8 | log_max_tir | | log2 number of TIRs (0 = unsupported) |
| 0x6C | 4:0 | log_max_tis | | log2 number of TISs (0 = unsupported) |
| 0x70 | 31 | basic_cyclic_rcv_wqe | | 0: cyclic RX WQE always has the 16-byte ctrl; 1: only when RMP.basic_cyclic_rcv_wqe |
| 0x70 | 28:24 | log_max_rmp | | log2 number of RMPs |
| 0x70 | 20:16 | log_max_rqt | | log2 number of RQTs |
| 0x70 | 12:8 | log_max_rqt_size | | log2 max RQT size |
| 0x70 | 4:0 | log_max_tis_per_sq | | log2 TISs per SQ |
| 0x74 | 28:24 | log_max_stride_sz_rq | | log2 max RQ stride (bytes) |
| 0x74 | 20:16 | log_min_stride_sz_rq | | log2 min RQ stride |
| 0x74 | 12:8 | log_max_stride_sz_sq | | log2 max SQ stride |
| 0x74 | 4:0 | log_min_stride_sz_sq | | log2 min SQ stride |
| 0x78 | 4:0 | log_max_wq_sz | | log2 max WQEs per WQ |
| 0x7C | 31 | reserved | | |
| 0x7C | 20:16 | log_max_vlan_list | | log2 max VLAN list in nic_vport_context |
| 0x7C | 12:8 | log_max_current_mc_list | | log2 max current MC MAC list |
| 0x7C | 4:0 | log_max_current_uc_list | | log2 max current UC MAC list |
| 0x80–0x8C | — | reserved | | |
| 0x90 | 28:24 | log_max_l2_table | | log2 max L2 table size (12.15) |
| 0x90 | 15:0 | log_uar_page_sz | RW | log2 UAR page size in 4 KB chunks |
| 0x94 | — | reserved | | |
| 0x98 | 31:0 | device_frequency_mhz | | Internal clock, MHz; valid if ≠ 0 |
| 0x9C–0xFC | — | reserved / not described | | |

Not present in this revision (do not look for them): `log_max_bf_regs_per_page`,
`log_max_bf_pages` (named in 7.2.2.1 but absent from Table 144), any pkey field,
`imaicl` (named in 10.1, absent from Table 144). No `log_max_cq`-style field
for UARs other than `uar_sz`.

### 6.9 Networking Offload capabilities, type 0x1 (PRM 12.3.3.2, Tables 146/147)

"Before querying ... software must make sure that the Ethernet field in
HCA_CAP is equal to 1" (i.e. eth_net_offloads).

| Offset | Bits | Name | Meaning |
|---|---|---|---|
| 0x00 | 31 | csum_cap | Checksum offload |
| 0x00 | 30 | vlan_cap | VLAN insert/strip |
| 0x00 | 29 | lro_cap | LRO |
| 0x00 | 28 | lro_psh_flag | LRO with TCP PSH |
| 0x00 | 27 | lro_time_stamp | LRO with TCP timestamp option |
| 0x00 | 26:25 | lro_max_msg_sz_mode | 0 from TCP header, 1 from L2 header |
| 0x00 | 23 | self_lb_en_modifiable | TIR.self_lb_en modifiable |
| 0x00 | 22 | self_lb_mc | Multicast self-loopback supported |
| 0x00 | 21 | self_lb_uc | Unicast self-loopback supported |
| 0x00 | 20:16 | max_lso_cap | log2 max LSO payload (0 = no LSO) |
| 0x00 | 13:12 | wqe_inline_mode | **0 L2 (min inline is L2 headers), 1 per nic_vport_context, 2 not_required** |
| 0x00 | 11:8 | rss_ind_tbl_cap | log2 max RSS indirection table (0 = no RSS) |
| 0x04 | — | reserved | |
| 0x08 | 15:0 | lro_min_mss_size | Min TCP payload for LRO |
| 0x0C–0x2C | — | reserved | |
| 0x30–0x3C | 4×32 | lro_timer_supported_periods[0..3] | µs, ascending |
| 0x40–0xFC | — | reserved | |

### 6.10 NIC Flow Table capabilities, type 0x7 (PRM 12.3.3.3, Tables 148–153)

Valid when HCA_CAP.nic_flow_table == 1.

| Offset | Bits | Name |
|---|---|---|
| 0x00 | 31 | nic_rx_multi_path_tirs |
| 0x00 | 30 | nic_rx_multi_path_tirs_fts |
| 0x04–0x3C | — | reserved |
| 0x40–0x7C | 512 | flow_table_properties_nic_receive (Table 150) |
| 0x80–0xFC | — | reserved |
| 0x100–0x13C | 512 | flow_table_properties_nic_transmit (Table 150) |
| 0x140–0xFFC | — | reserved |

Flow Table Properties (Table 150/151), relative to the properties block:

| Offset | Bits | Name | Meaning |
|---|---|---|---|
| 0x00 | 31 | ft_support | This flow-table type supported |
| 0x00 | 30 | flow_tag | flow_tag reported in CQE |
| 0x00 | 29 | flow_counter | Flow counters supported |
| 0x00 | 28 | flow_modify_en | Flow entries modifiable |
| 0x00 | 27 | modify_root | Root table dynamically modifiable |
| 0x00 | 26 | identified_miss_table | Forward-to-miss-table on create |
| 0x00 | 25 | flow_table_modify | MODIFY_FLOW_TABLE supported |
| 0x00 | 22 | reset_root_to_default | Reset root to default supported |
| 0x04 | 29:24 | log_max_ft_size | log2 flow-table size |
| 0x04 | 7:0 | max_ft_level | Max flow-table level |
| 0x08 | — | reserved | |
| 0x0C | 7:0 | log_max_ft_num | log2 number of flow tables |
| 0x10 | 15:8 | log_max_flow_counter | log2 counters per flow |
| 0x10 | 7:0 | log_max_destination | log2 destinations per flow |
| 0x14 | 7:0 | log_max_flow | log2 total flows of this type |
| 0x18–0x1C | — | reserved | |
| 0x20–0x2C | 128 | ft_field_support | Table 152/153 bitmap |
| 0x30–0x3C | 128 | ft_field_bitmask_support | Table 152/153 bitmap |

Table 153 (dword 0x00 of each bitmap): 31 outer_dmac, 30 outer_smac,
29 outer_ether_type, 27 outer_first_prio, 26 outer_first_cfi, 25 outer_first_vid,
23 outer_second_prio, 22 outer_second_cfi, 21 outer_second_vid,
20 outer_ipv6_flow_label, 19 outer_sip, 18 outer_dip, 17 outer_frag,
16 outer_ip_protocol, 15 outer_ip_ecn, 14 outer_ip_dscp, 13 outer_udp_sport,
12 outer_udp_dport, 11 outer_tcp_sport, 10 outer_tcp_dport, 9 outer_tcp_flags.

### 6.11 SET_HCA_CAP 0x109 (PRM 12.3.4, Tables 154–157)

| In offset | Bits | Name | Meaning |
|---|---|---|---|
| 0x04 | 0 | op_mod[0] | reserved (Table 155; the capability row of the same table still says "op_mod[0] max/current" — copy-paste) |
| 0x04 | 15:1 | op_mod[15:1] | Capability type 0x0 general, 0x1 Ethernet offload, 0x7 NIC flow table |
| 0x08–0x0C | — | reserved | |
| 0x10–0x100C | 32768 | capability | Structure of the chosen type |

Len in 0x1010, out 16. "Must be run prior to initial QUERY_PAGES" (12.3.4) —
but 8.2 orders boot QUERY_PAGES/MANAGE_PAGES *before* QUERY/SET_HCA_CAP;
following 8.2, "initial" is read as the init-pages query. **HW-CHECK.**

Writable fields in this revision (general caps, RW in Table 145):
`cache_line_128byte` (0x2C bit 27), `cmdif_checksum` (0x40 15:14),
`log_max_sq` (0x6C 20:16, RW mark probably misplaced), `log_uar_page_sz`
(0x90 15:0). No pkey fields exist. The PRM states nothing that **must** be set;
which value of log_uar_page_sz (0 = 4 KB, matching x86 UEFI) and whether
unmodified fields may be written back as queried — **HW-CHECK** (natural approach:
QUERY_HCA_CAP current, edit, SET).

### 6.12 INIT_HCA 0x102 / TEARDOWN_HCA 0x103 (PRM 12.3.6–12.3.7, Tables 165–172)

INIT_HCA: in 0x08–0x0C reserved; out 0x08–0x0C reserved. Len 16/16.

TEARDOWN_HCA:

| In offset | Bits | Name | Meaning |
|---|---|---|---|
| 0x04 | 15:0 | op_mod | no values given (0) |
| 0x08 | 15:0 | profile | **0 Graceful_close, 1 Panic_close** |
| 0x0C | — | reserved | |

Out 0x08–0x0C reserved. Len 16/16. In this revision graceful/panic is the
`profile` field at 0x08, **not** op_mod.
Graceful: releases all HCA resources, stops descriptor execution; SW must first
close all SQs/RQs, CQs, EQs and unmap all EQs. Panic: stops bus mastering,
moves ports to link down; doorbells afterwards are silently dropped.

### 6.13 SET_DRIVER_VERSION 0x10D (PRM 12.3.12, Tables 189–192)

| In offset | Bits | Name | Meaning |
|---|---|---|---|
| 0x08–0x0C | — | reserved | |
| 0x10–0x4C | 512 | driver_version | ASCII, ≤ 64 bytes incl. NUL: `OS_name,Driver_name,Main.Minor.Subminor` — Main up to 3 digits (no leading zeros), Minor 3 digits (leading zeros), Subminor 6 digits (leading zeros) |

Len in 0x50, out 16. Only if HCA_CAP.driver_version (0x4C bit 30) == 1.

### 6.14 ALLOC_PD 0x800 / DEALLOC_PD 0x801 (PRM 12.3.13–14, Tables 193–200)

ALLOC_PD: in 0x08–0x0C reserved; out 0x08 **23:0 pd**, 0x0C reserved. Len 16/16.
DEALLOC_PD: in 0x08 **23:0 pd**, 0x0C reserved; out 0x08–0x0C reserved. Len 16/16.

### 6.15 ALLOC_UAR 0x802 / DEALLOC_UAR 0x803 (PRM 12.3.15–16, Tables 201–208)

ALLOC_UAR: in 0x08–0x0C reserved; out 0x08 **23:0 uar**, 0x0C reserved. Len 16/16.
DEALLOC_UAR: in 0x08 **23:0 uar**; out reserved. Len 16/16.

### 6.16 ALLOC_TRANSPORT_DOMAIN 0x816 / DEALLOC 0x817 (PRM 12.3.18–19, Tables 213–220)

ALLOC: in 0x08–0x0C reserved; out 0x08 **23:0 transport_domain**, 0x0C reserved. Len 16/16.
DEALLOC: in 0x08 **23:0 transport_domain**, 0x0C reserved; out reserved. Len 16/16.

### 6.17 QUERY_SPECIAL_CONTEXTS 0x203 (PRM 12.5.4, Tables 238–241)

In 0x08–0x0C reserved. Out: 0x08 reserved, **0x0C 31:0 resd_lkey** ("reserved
Lkey for Base Memory Management Extension"). Len 16/16.
PRM 6.3: the reserved LKey maps VA = PA one-to-one; its use is "qualified by
the rlky bit in the RQ/SQ contexts".

### 6.18 MKey context (PRM 6.4.1, Table 9 layout, Table 10 fields) — 64 bytes

| Offset | Bits | Name | Meaning |
|---|---|---|---|
| 0x00 | 30 | free | 0 in use; 1 free (not usable) |
| 0x00 | 15 | umr_en | UMR allowed on this MKey |
| 0x00 | 13 | rw | Remote write |
| 0x00 | 12 | rr | Remote read |
| 0x00 | 11 | lw | Local write |
| 0x00 | 10 | lr | Local read — **must be set for all MKeys** |
| 0x00 | 9:8 | access_mode | **0x0 PA** (VA = PA, no translation), 0x1 MTT, 0x2 KLMs |
| 0x04 | 31:8 | qpn | **must be 0xFFFFFF** |
| 0x04 | 7:0 | mkey[7:0] | Variant (key) byte of the MKey |
| 0x08 | — | reserved | |
| 0x0C | 31 | length64 | Region length is 2^64 bytes (len then reserved) |
| 0x0C | 23:0 | pd | Protection domain |
| 0x10–0x14 | 64 | start_addr | Start virtual address |
| 0x18–0x1C | 64 | len | Region length (reserved if length64) |
| 0x20–0x30 | — | reserved | |
| 0x34 | 31:0 | translations_octword_size | 16-byte units for MTT/KLM list; reserved for PA; multiple of 4 |
| 0x38 | 4:0 | log_entity_size | MTT page size log2 (≥ 12); reserved otherwise |
| 0x3C | — | reserved | |

MKey value (PRM 6.1, Figure 6): bits 31:8 = index, bits 7:0 = key.

PA-mode MKey for a polling driver: access_mode = 0, lr = lw = 1, qpn =
0xFFFFFF, pd = ALLOC_PD result, length64 = 1 (or start_addr/len for a range),
translations_octword_size = 0. PRM: PA mode "Not valid for block mode MKey,
replicated MTT MKey".

### 6.19 CREATE_MKEY 0x200 / DESTROY_MKEY 0x202 (PRM 12.5.1, 12.5.3, Tables 226–229, 234–237)

CREATE_MKEY input:

| Offset | Bits | Name | Meaning |
|---|---|---|---|
| 0x08 | — | reserved | |
| 0x0C | 31 | pg_access | Per-page rights in MTTs; only with access_mode MTT |
| 0x10–0x4C | 512 | mkey context | §6.18 |
| 0x50–0x5C | — | reserved | |
| 0x60 | 31:0 | translations_octword_actual_size | Octwords of translation entries delivered; may be 0 |
| 0x64–0x10C | — | reserved | |
| 0x110… | | klm / pas / mtt[...] | Translation entries |

Len in 0x110 (+ entries). CREATE_MKEY output: 0x08 **23:0 mkey_index**, 0x0C
reserved; len 16. Whether the usable key is `(mkey_index << 8) | mkey[7:0]`
(by Figure 6) — **HW-CHECK**.

DESTROY_MKEY: in 0x08 **23:0 mkey_index**, 0x0C reserved; out reserved. Len 16/16.

### 6.20 ACCESS_REGISTER 0x805 (PRM 12.4.1, Tables 221–224)

| In offset | Bits | Name | Meaning |
|---|---|---|---|
| 0x04 | 15:0 | op_mod | **0 WRITE, 1 READ** |
| 0x08 | 15:0 | register_id | Register ID |
| 0x0C | 31:0 | argument | Per register_id (none defined for PMTU/PTYS/PAOS — use 0) |
| 0x10… | 31:0 | register data | Write: data. Read: "reserved" |

| Out offset | Bits | Name | Meaning |
|---|---|---|---|
| 0x08–0x0C | — | reserved | |
| 0x10… | 31:0 | register data | Read: data. Write: reserved |

Len in/out = 0x10 + register size. On READ the PRM calls input data
"reserved", yet the registers have Index fields (local_port) that must select
the port — **HW-CHECK** (supply the index fields in the input on READ).

### 6.21 Port registers (PRM 11.1, Tables 104–110)

Register IDs (Table 104): **PMTU 0x5003, PTYS 0x5004, PAOS 0x5006**, PFCC
0x5007, PPCNT 0x5008. local_port numbering (1-based?) — **PRM does not say — HW-CHECK**.

PMTU (Tables 105/106), 16 bytes:

| Offset | Bits | Name | Acc | Meaning |
|---|---|---|---|---|
| 0x00 | 23:16 | local_port | Index | |
| 0x04 | 31:16 | max_mtu | RO | Max MTU supported |
| 0x08 | 31:16 | admin_mtu | RW | Configured MTU (≤ max_mtu) |
| 0x0C | 31:16 | oper_mtu | RO | Actual MTU; larger packets dropped |

MTU unit (bytes? incl. L2 header/FCS?) — **HW-CHECK**.

PTYS (Tables 107/108), 64 bytes (0x00–0x3C):

| Offset | Bits | Name | Acc | Meaning |
|---|---|---|---|---|
| 0x00 | 31 | reserved | | |
| 0x00 | 23:16 | local_port | Index | |
| 0x00 | 2:0 | proto_mask | Index | Which protocol data is valid; bit 2 = Ethernet |
| 0x04–0x08 | — | reserved | | |
| 0x0C | 31:0 | eth_proto_capability | RO | Supported speeds (bitmask below) |
| 0x10–0x14 | — | reserved | | |
| 0x18 | 31:0 | eth_proto_admin | RW | Enabled speeds |
| 0x1C–0x20 | — | reserved | | |
| 0x24 | 31:0 | eth_proto_oper | RO | Current speed |
| 0x28–0x3C | — | reserved | | |

Speed bits (Table 108): 31 50GBase-KR2, 30 50GBase-CR2, 29 25GBase-SR,
28 25GBase-KR, 27 25GBase-CR, 22 100GBase-KR4, 21 100GBase-SR4, 20 100GBase-CR4,
18 50GBase-SR2, 16 40GBase-LR4/ER4, 15 40GBase-SR4, 14 10GBase-ER/LR,
13 10GBase-SR, 12 10GBase-CR, 7 40GBase-KR4, 6 40GBase-CR4, 4 10GBase-KR,
3 10GBase-KX4, 2 10GBase-CX4, 1 1000Base-KX, 0 SGMII. Bits 5, 8–11, 17, 19,
23–26 are not listed. Admin changes take effect only at link training; force it
by disabling/enabling the port (PAOS).

PAOS (Tables 109/110), 16 bytes:

| Offset | Bits | Name | Acc | Meaning |
|---|---|---|---|---|
| 0x00 | 31:24 | reserved | | |
| 0x00 | 23:16 | local_port | Index | |
| 0x00 | 11:8 | admin_status | RW | 1 up, 2 down by configuration, 3 up once, 4 disabled by system (HW only) |
| 0x00 | 3:0 | oper_status | RO | 1 up, 2 down, 4 down by port failure |
| 0x04 | 31 | ase | WO | Admin state update enable — must be 1 on a write for admin_status to apply |
| 0x08–0x0C | — | reserved | | |

No event-enable bits exist in PAOS in this revision.

### 6.22 NOP 0x80D (PRM 12.18.1, Tables 493–496)

Generates a command-completion event; nothing else. Useful to test the command
path. In 0x08–0x0C reserved; out 0x08–0x0C reserved. Len 16/16.

### 6.23 Related: Page Request event (PRM 7.13.10.2, Tables 91/92)

Event type 0x0B. EQE data 0x04 31:0 num_pages: signed, "missing / unneeded
pages", a recommendation only. Answer with exactly one MANAGE_PAGES, which
re-arms the event.

---

## 7. Command lengths at a glance

| Command | In len | Out len |
|---|---|---|
| ENABLE_HCA / DISABLE_HCA | 0x10 | 0x0C |
| QUERY_ISSI | 0x10 | 0x70 |
| SET_ISSI | 0x10 | 0x10 |
| QUERY_PAGES | 0x10 | 0x10 |
| MANAGE_PAGES (give) | 0x10 + 8n | 0x10 |
| MANAGE_PAGES (return n) | 0x10 | 0x10 + 8n |
| QUERY_HCA_CAP | 0x10 | 0x1010 |
| SET_HCA_CAP | 0x1010 | 0x10 |
| INIT_HCA / TEARDOWN_HCA | 0x10 | 0x10 |
| SET_DRIVER_VERSION | 0x50 | 0x10 |
| ALLOC/DEALLOC PD, UAR, TD | 0x10 | 0x10 |
| QUERY_SPECIAL_CONTEXTS | 0x10 | 0x10 |
| CREATE_MKEY (PA) | 0x110 | 0x10 |
| DESTROY_MKEY | 0x10 | 0x10 |
| ACCESS_REG PMTU/PAOS | 0x20 | 0x20 |
| ACCESS_REG PTYS | 0x50 | 0x50 |
| NOP | 0x10 | 0x10 |

(MANAGE_PAGES return: what goes in the input for op_mod 2 beyond
input_num_entries is not stated.) Lengths below 8 are rejected (delivery
status 0x7/0x8); whether a longer-than-needed length is accepted — **HW-CHECK**.

---

## 8. PRM inconsistencies found

1. Mailbox ctrl_signature range: 7.14.1.1.1 says bytes 1C0h–1FFh, Table 98 says
   dwords 1D0h–1FCh; both are inside the data area while the ctrl fields are
   0x230–0x23F (each is 0x40 below 0x200–0x23F / 0x210–0x23C).
2. Entry signature: Table 96 "does not include the mailbox"; 9.2.1 "the mailbox
   is also protected by the command signature". No algorithm given for the entry.
3. Mailbox pointer alignment: entry pointers keep bits 31:9 (512 B), next_pointer
   bits 31:10 (1 KB); the block is 0x240 bytes.
4. QUERY_PAGES sign: 12.3.1 prose says positive = spare pages; Table 133 says
   positive = driver should provide.
5. Output syndrome "Valid only if status = 0x0" (Table 102) — inverted.
6. "BAD_OPCODE" (7.1, 12.3.10) does not exist in Table 128 (BAD_OP = 0x02).
7. SET_HCA_CAP: op_mod bit 0 "Reserved" vs. the same table's "op_mod[0:0]
   Maximum/Current"; "must be run prior to initial QUERY_PAGES" vs. 8.2 order.
8. HCA cap 0x3C: layout shows stat_rate_support in 31:16, descriptions say 31:16
   reserved.
9. RW mark on log_max_sq (0x6C), unlike its neighbours.
10. 7.2.2.1 names log_max_bf_regs_per_page / log_max_bf_pages, and 10.1 names
    HCA_CAP.imaicl; none is in Table 144.
11. Stale cross-references: "Section 11.2, HCA Driver Start-up, page 366" and
    "Chapter 11 Initialization and Teardown, page 365" — actual is 8.2/8.3 (p.151–152).
12. 12.5 prose speaks of ALLOC_MKEY / DEALLOC_MKEY; the opcodes are CREATE_MKEY /
    DESTROY_MKEY.
13. ACCESS_REGISTER input data "reserved" on READ although registers carry Index
    fields.
14. Table 127 links QUERY_RQ (0x90B) to 12.11.3, the same section as DESTROY_RQ.
15. Table 108 runs "Bit 18 – 50GBase-SR2" and "Bit 16 – 40GBase LR4/ER4" into one
    line (typesetting only).
16. ENABLE_HCA / DISABLE_HCA output layouts end at 0x08 (12 bytes) while all
    similar commands are 16 bytes.

---

## 9. HW-CHECK (things the PRM leaves open)

1. Expected `cmd_interface_rev` value(s) the driver must "match".
2. log_cmdq_stride unit (bytes assumed) and that entries are 64 bytes apart.
3. Byte order of init-segment MMIO accesses (the PRM's big-endian convention is
   assumed to apply to MMIO dwords too, i.e. byte-swap on x86).
4. UAR page *n* address in BAR0 (`n << (12 + log_uar_page_sz)`?), and whether UAR
   page 0 overlaps the initialization segment.
5. Health counter bit range at 0x1010 and the internal-error buffer's extent.
6. Software reset / recovery of a function left initialized by a previous owner
   (FlexBoot): no procedure in the PRM; whether DISABLE_HCA/ENABLE_HCA or PCIe
   FLR is needed before rewriting cmdq_phy_addr.
7. Command-completion timeout (none given) and how long `initializing` may stay 1.
8. Entry signature algorithm and coverage; real ctrl_signature range; power-on
   value of cmdif_checksum.
9. Mailbox alignment actually enforced (512 B vs 1 KB vs 4 KB).
10. supported_issi bit placement (byte 0x6F bit 0 = ISSI 0 assumed).
11. QUERY_PAGES / num_pages sign (Table 133 reading assumed); MANAGE_PAGES op_mod 2
    input semantics; max PAS entries per MANAGE_PAGES.
12. Whether page requests arrive after INIT_HCA without an EQ (a polling driver
    may need to poll QUERY_PAGES op_mod 3 regular_pages, or must create an EQ
    for PAGE_REQUEST as 8.2 says).
13. SET_HCA_CAP: what must be set; writing back queried values; log_uar_page_sz.
14. Whether the MKey value is `mkey_index << 8 | mkey[7:0]`; whether the
    reserved LKey can be used instead of a PA MKey on this firmware.
15. ACCESS_REG READ: local_port supplied in input data; local_port numbering;
    PMTU unit.
16. Command lengths: whether exact lengths are required (ENABLE/DISABLE_HCA 12 vs 16 bytes).
17. stat_rate_support at cap 0x3C 31:16.
