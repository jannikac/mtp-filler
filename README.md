# MTP Filler

Fills the Storage of a MTP device. This can be used to prevent automatic OTA updates. This program uses `libmtp-rs` (bindings to `libmtp`) on Linux and `winmtp` (bindings to Windows WPD API).

[![asciicast](https://asciinema.org/a/Lfd75pn3mietl5JKOXZO5f12S.svg)](https://asciinema.org/a/Lfd75pn3mietl5JKOXZO5f12S)

## Usage

1. Download the program for your machine from the [releases page](https://github.com/jannikac/mtp-filler/releases).
2. Connect your MTP device via USB
3. Run the executable. It will prompt on how much space to leave on the device, which device to select etc.

Linux Notes: You may have to kill processes that are using the MTP device. For example for KDE the `kio` process takes over MTP devices after they are connected which causes this program to fail.

Windows Notes: Dont press CTRL+C or kill the process while it is transferring. MTP is slow and sometimes 1 GiB of transfer can take up to 2-3 Minutes.

## AI Usage

- No AI was used in the core implementaton and cli.
- AI was used for the following components / parts
  - Patching winmtp library to enable a progress bar on Windows
  - Vendored libmtp so a fully static library can be built. If you dont trust it, build it from source and use the dynamically linked version. The vendored version should be the same as the upstream version, it just has a build.rs script that builds the static version of the `libmtp` and `libusb` libraries.

## Building

Builds are provided via Github releases. You can also build the Software yourself. For instructions see below.

### Prerequisites

- A working rust toolchain

### Linux

There are two supported Linux build modes.

#### Fully static build - default for releases

For a fully static Linux executable, use the vendored `libmtp-sys` in [`vendor/libmtp-sys`] together with the musl target:

```bash
cargo build --release --target x86_64-unknown-linux-musl
```

The musl binary will be written to `target/x86_64-unknown-linux-musl/release/mtp-filler`.

This path builds bundled static copies of `libusb` and `libmtp` during `cargo build`, and the resulting musl binary is fully static.

The vendored build disables MTPZ support, so legacy Zune-era MTPZ devices are not supported by this program. This also avoids the extra `libgcrypt` and `libgpg-error` dependency chain.

#### Fully dynamically linked build - only use if you cant use musl / fully statically linked binaries

Install the development version of [libmtp](http://libmtp.sourceforge.net/) via your package manager or other means. For example `libmtp` on Arch or `libmtp-dev` on Ubuntu. Then build with:

```bash
LIBMTP_SYS_USE_PKG_CONFIG=1 cargo build --release
```

This uses the system `libmtp` installation discovered through `pkg-config`, so the resulting executable depends on the host system's shared libraries at runtime.

### Windows

```bash
cargo build --release
```
