# MTP Filler

Fills the Storage of a MTP device. This can be used to prevent automatic OTA updates. This program uses `libmtp-rs` (bindings to `libmtp`) on Linux and `winmtp` (bindings to Windows WPD API). Windows and Linux behaviour differs a little. The Windows Version doesn't have a progress bar while uploading to the device (PRs welcome).

## Usage

Connect your MTP device via USB and run the executable. It will prompt on how much space to leave on the device, which device to select etc.

Linux Notes: You may have to kill processes that are using the MTP device. For example for KDE the `kio` process takes over MTP devices after they are connected which causes this program to fail.

Windows Notes: Dont press CTRL+C or kill the process while it is transferring. MTP is slow and sometimes 1 GiB of transfer can take up to 2-3 Minutes.

## Building

Builds are provided via Github releases. You can also build the Software yourself. For instructions see below.

### Prerequisites

A working rust toolchain.

### Linux

Install the dev version of [libmtp](http://libmtp.sourceforge.net/) via your package manger or other means. For example `libmtp` on Arch or `libmtp-dev` on Ubuntu. Then run

```bash
cargo build --release
```

### Windows

```bash
cargo build --release
```
