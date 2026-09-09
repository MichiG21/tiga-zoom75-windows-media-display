# Zoom75 TIGA / PocketWuque BLE upload transcript

Source: `bluetoothd-hci-latest.pklg` from the supplied iPhone sysdiagnose.

This is an evidence transcript of the **two successful red/green GIF uploads** in that capture. The two upload data streams are byte-for-byte identical.

## 1. GATT map observed in service discovery

Custom service UUID:

`1f40eaf8-aab4-14a3-f1ba-f61f35cddbaa`

Observed service handle range: `0x0010`–`0x001A`.

| Characteristic | UUID | Value handle | Properties observed | Role in capture |
|---|---|---:|---|---|
| 0001 | `1f400001-aab4-14a3-f1ba-f61f35cddbaa` | `0x0012` | `0x0C` (write + write-without-response) | control commands |
| 0002 | `1f400002-aab4-14a3-f1ba-f61f35cddbaa` | `0x0014` | `0x10` (notify) | status/progress notifications |
| 0003 | `1f400003-aab4-14a3-f1ba-f61f35cddbaa` | `0x0017` | `0x0C` (write + write-without-response) | bulk upload |
| 0004 | `1f400004-aab4-14a3-f1ba-f61f35cddbaa` | `0x0019` | `0x10` (notify) | second notify characteristic; no upload payload observed on it in this capture |

The upload itself uses ATT **Write Command (`0x52`)**, i.e. write-without-response, for both the control and bulk characteristics.

## 2. Two control writes immediately before each bulk upload

The characteristic value starts with an `0x88` application envelope. For these control messages, bytes 4..6 encode the following payload length as a 24-bit big-endian integer.

### Control #1

Full characteristic value:

`88 00 00 00 00 00 10 00 09 03 04 00 08 00 00 FF FC 00 03 5D D0 05 00 00`

Envelope payload length: `0x10` = 16 bytes.

Inner payload:

`09 03 04 00 08 00 00 FF FC 00 03 5D D0 05 00 00`

The 24-bit value `03 5D D0` is **220624**, exactly the size of the DLX/file object including its 4-byte trailer.

### Control #2

Full characteristic value:

`88 00 00 00 00 00 0A 00 08 03 00 FF FF FF 00 FF FF FF`

Envelope payload length: `0x0A` = 10 bytes.

Inner payload:

`08 03 00 FF FF FF 00 FF FF FF`

### Timing

First captured upload:

- Control #1: ~160 ms before first bulk write
- Control #2: ~79 ms before first bulk write

Second captured upload:

- Control #1: ~498 ms before first bulk write
- Control #2: ~154 ms before first bulk write

Therefore the **ordering** is much more strongly established than an exact required delay.

## 3. Bulk transfer

Bulk characteristic: UUID ending `0003`, value handle `0x0017`.

Each successful upload contains **1239 ATT Write Commands** and takes about 20 seconds in this capture.

Characteristic-value length distribution for each upload:

- 1185 writes × 182 bytes
- 52 writes × 115 bytes
- 1 write × 140 bytes
- 1 final write × 98 bytes

For every one of the 1239 writes:

- final byte = XOR checksum
- XOR of the entire characteristic value (payload + checksum) = `0x00`

Removing one XOR byte from every write and concatenating the remaining bytes produces exactly **220649 bytes**.

SHA-256 of that reassembled stream:

`a4be0a5df3e785a7cc4a13969d1b28b72312eda684627e487859ef11bdfae7d0`

## 4. Reassembled bulk stream structure

The 220649-byte stream is:

```
8 bytes    outer 0x88 bulk envelope
17 bytes   upload preamble
220624 B   file/DLX object
```

### Outer bulk envelope

`88 00 00 00 03 5D E1 00`

Bytes 4..6 are `03 5D E1` = **220641**, exactly the bytes that follow this 8-byte envelope:

`17 + 220624 = 220641`.

### 17-byte upload preamble

`08 05 01 00 09 00 FF FF FF 00 FF FF FF 02 02 FF FF`

### DLX/file object

File size: **220624 bytes**.

Layout:

```
8 B       file prefix
452 B     media header
110080 B  frame 1
110080 B  frame 2
4 B       trailer
```

#### File prefix

`00 44 4C 58 FC FF 00 00`

Bytes `44 4C 58` are ASCII `DLX`.

#### Trailer

`FC FF 00 00`

## 5. Media header / frame evidence

The media header begins after the 8-byte file prefix and is exactly **452 bytes**.

Selected fields that self-consistently match the file:

- header offset `0x15C`: little-endian `452`
- header offset `0x160`: little-endian `460` = 8-byte file prefix + 452-byte header
- header offset `0x164`: little-endian `220620` = file bytes excluding the final 4-byte trailer
- header offset `0x170`: big-endian `320`
- header offset `0x174`: big-endian `172`
- header offset `0x1C0`: little-endian `110080` = one RGB565 frame

The header begins:

`60 01 03 00 02 00 00 01 ...`

The byte `02` correlates with this capture containing two frames, but its semantic meaning is not yet treated as proven.

### Frame 1

- 110080 bytes
- 55040 / 55040 pixels are `F8 00`
- RGB565 **big-endian** = solid red

### Frame 2

- 110080 bytes
- 55040 / 55040 pixels are `07 E0`
- RGB565 **big-endian** = solid green

## 6. 4096-byte boundary behavior

Bulk writes are normally 181 data bytes + 1 XOR byte.

PocketWuque shortens writes so a write never crosses a 4096-byte boundary **relative to the start of the 220624-byte DLX/file object**.

Examples:

- first shortened write begins at file-relative offset 3957 and carries 139 bytes, ending exactly at 4096
- next boundary write ends exactly at 8192
- next at 12288
- and so on

This repeats across the complete file. That behavior should be preserved in the first replay implementation rather than optimized away.

## 7. Notifications during upload

Status notifications arrive on value handle `0x0014` / UUID ending `0002`.

Immediately around setup, examples include:

`88 00 00 00 00 00 05 0E 07 02 01 09 03`

`88 00 00 00 00 00 03 0F 09 04 02`

`88 00 00 00 00 00 05 0F 07 02 01 08 03`

`88 00 00 00 00 00 03 0E 08 04 02`

During the transfer, a repeatedly observed status is:

`88 00 00 00 00 00 03 0B 08 06 05`

After the last bulk write, the successful completion notification is:

`88 00 00 00 00 00 03 0C 08 06 02`

In the first upload it arrived ~153 ms after the last bulk write; in the second ~142 ms after the last bulk write.

No extra control write was observed between the last bulk write and this completion notification.

## 8. What is proven vs. still unknown

### Strongly evidenced by this capture

- GATT UUID/handle mapping above
- control write ordering
- `0x88` 24-bit length envelope behavior in these messages
- exact red/green bulk byte stream
- per-write XOR checksum
- 181-byte normal bulk payload size
- 4096-byte DLX-file boundary-aware chunking
- 8-byte `DLX` file prefix
- 452-byte media header
- RGB565 big-endian pixel data
- 4-byte `FC FF 00 00` trailer
- successful completion notification `... 08 06 02`
- the two uploads in the capture are byte-for-byte identical

### Not yet proven

- semantic names of all control/status fields
- whether all setup notifications are mandatory gates or merely status responses
- exact animation timing field(s)
- whether writes can safely be pipelined/faster than PocketWuque
- whether the second notification characteristic is needed for media upload
- how the header changes for 15 frames / 5 FPS

The next safe engineering step is therefore a Windows BLE probe that only scans/connects/discovers this GATT service. After that succeeds, a guarded known-good replay can reproduce this exact transaction.
