# USB Host MSC for JZF407 — Implementation Plan

Goal: plug a USB-A flash drive containing `firmware.bin` into the board, board
reads the file from FAT32, writes it into the inactive flash slot, reboots,
runs the new firmware.

This is a multi-month embedded R&D effort. Path is laid out in stages so each
stage is provable on its own and we can stop / pivot at any boundary.

## Hardware prerequisites

JZF407 must expose USB OTG_FS pins:
- PA11 = USB_DM
- PA12 = USB_DP
- PA9  = USB_VBUS (host detects 5V on the connector)
- PA10 = USB_ID (host/device select; tie low for host)
- 5V on VBUS pin of the USB-A connector (host supplies, plate's 5V rail)

The board has a USB-A connector that must be wired for **HOST** (not device).
If the connector is a USB-B/micro for device mode, this whole plan is moot —
USB Host needs a USB-A "downstream-facing" port that supplies 5V.

## Stage 0 — verify hardware [BLOCKING]
- Inspect JZF407 schematic / PCB: is the USB-A port wired as HOST?
- Does it have a 5V supply switch (e.g. MIC2026, TPS2051) for VBUS to the device?
- Without host wiring, NOTHING below works — switch to SD-card plan.

## Stage 1 — bring up USB OTG_FS in host mode
- Enable USB OTG_FS clock, configure PA11/PA12 as USB AF.
- Use `embassy-usb-synopsys-otg` low-level driver in host mode (currently
  experimental — only device mode is upstream).
- Detect device attach (VBUS + DP/DM pull-up sensed).
- Issue control transfer GET_DESCRIPTOR(DEVICE) and read VID/PID.
- Success criterion: defmt::info!("USB: device VID={:04x} PID={:04x}", ...);

This is the riskiest stage. If `embassy-usb-synopsys-otg` host support is
not viable, switch to writing a raw OTG_FS host driver against the PAC —
weeks of work.

## Stage 2 — set address + read configuration descriptor
- Standard USB enumeration: SET_ADDRESS, GET_DESCRIPTOR(CONFIG), parse interfaces.
- Filter for Mass Storage class (bInterfaceClass = 0x08, SubClass = 0x06 SCSI,
  Protocol = 0x50 Bulk-Only).
- Reject devices that aren't mass storage (early return).

## Stage 3 — implement Bulk-Only Transport (BOT)
- BOT = SCSI commands wrapped in USB Bulk transfers.
- Implement CBW (Command Block Wrapper) → SCSI command → CSW (Status Wrapper).
- SCSI commands needed: INQUIRY, READ_CAPACITY(10), READ(10).

## Stage 4 — FAT32 layer
- Implement `BlockDevice` trait from `embedded-sdmmc` or `embedded-fatfs`
  on top of the SCSI READ(10) calls.
- Mount filesystem, open `/firmware.bin`, stream it.

## Stage 5 — flash write + boot
- Either use `embassy-boot-stm32` two-slot OTA, or write directly into the
  second half of flash and jump.
- Validate CRC of read data before flashing.
- After flash, set DFU state and reset.

## Stage 6 — robustness
- VBUS short-circuit detection / power switch control.
- Hot-plug (device removed mid-read).
- Reject non-FAT32 or wrong-format drives gracefully (don't brick).
- File missing → defmt error, do nothing.

## Why this is hard (recap)

- USB Host is not first-class in embassy yet (device mode is).
- MSC class host is not implemented in Rust no_std — we write it.
- FAT32 is the easy part.
- Total: 2-4 months of focused work for someone with USB experience.
- For someone new to USB: 6+ months.
