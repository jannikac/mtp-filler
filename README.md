# MTP Filler

Fills the Storage of a MTP device. This can be used to prevent automatic OTA updates. This program uses `libmtp-rs` (bindings to `libmtp`) on Linux and `winmtp` (bindings to Windows WPD API).

[![asciicast](https://asciinema.org/a/Lfd75pn3mietl5JKOXZO5f12S.svg)](https://asciinema.org/a/Lfd75pn3mietl5JKOXZO5f12S)

## Usage

Download the program for your machine from the [releases page](https://github.com/jannikac/mtp-filler/releases), connect your MTP device via USB, and then follow the steps for your platform.

### Windows

Run the `.exe`. It should work out of the box.

The program will prompt for how much space to leave on the device, which device to select, etc.

### macOS

Make the binary executable and then run it:

```bash
chmod +x ./mtp-filler
./mtp-filler
```

If macOS blocks the app because it is from an unidentified developer, allow it in `System Settings` -> `Privacy & Security` and then run it again.

The program will prompt for how much space to leave on the device, which device to select, etc.

### Linux

Make the binary executable and then run it:

```bash
chmod +x ./mtp-filler
./mtp-filler
```

The program will prompt for how much space to leave on the device, which device to select, etc.

## Troubleshooting

### No attached MTP devices detected

Make sure the device is connected via USB, unlocked, and set to MTP / File Transfer mode. If it was already connected, unplug it and reconnect it before starting the program again.

### Permission denied or the binary does not start on Linux or macOS

Make sure the binary is executable:

```bash
chmod +x ./mtp-filler
./mtp-filler
```

### Device is busy or cannot be opened

This usually means the device was already mounted or another program automatically claimed it, so `mtp-filler` cannot access it at the same time.

Close the file manager and any other program that may be using the device, unmount or disconnect it if needed, and then try again. On KDE, `kio` often grabs MTP devices automatically after they are connected.

### macOS says the app is from an unidentified developer

Open `System Settings` -> `Privacy & Security`, allow the app to run, and then start it again.

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
