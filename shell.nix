let
  # Pinned nixpkgs, deterministic. Last updated: 2/12/21.
  # pkgs = import (fetchTarball("https://github.com/NixOS/nixpkgs/archive/a58a0b5098f0c2a389ee70eb69422a052982d990.tar.gz")) {};

  # Rolling updates, not deterministic.
  pkgs = import (fetchTarball ("channel:nixos-25.11")) { };
in
pkgs.callPackage (
  {
    mkShell,
    cargo,
    rustc,
    rust-analyzer,
    rustfmt,
    pkg-config,
    libmtp,
  }:
  mkShell {
    strictDeps = true;
    nativeBuildInputs = [
      cargo
      rustc
      rust-analyzer
      rustfmt
      pkg-config
    ];
    buildInputs = [
      libmtp
    ];
  }
) { }
