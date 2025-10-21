# MTP Filler

Fills the Storage of a MTP device. This can be used to prevent automatic OTA updates.

## Usage

Connect your MTP device via USB and run the executable. It will prompt on how much space to leave on the device, which device to select etc.

## Building

### Linux

Install [libmtp](http://libmtp.sourceforge.net/) via your package manger or other means. Then run

```bash
cargo build --release
```

### Windows

Windows Binaries and building is currently WIP
