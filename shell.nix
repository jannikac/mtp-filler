let
  # Pinned nixpkgs, deterministic. Last updated: 2/12/21.
  # pkgs = import (fetchTarball("https://github.com/NixOS/nixpkgs/archive/a58a0b5098f0c2a389ee70eb69422a052982d990.tar.gz")) {};

  # Rolling updates, not deterministic.
  pkgs = import (fetchTarball ("channel:nixos-25.11")) { };
in
pkgs.callPackage (
  {
    mkShell,
    lib,
    cargo,
    rustc,
    rust-analyzer,
    rustfmt,
    slint-lsp,
    pkg-config,
    fontconfig,
    wayland,
    libxkbcommon,
    libGL,
    libmtp,
  }:
  mkShell {
    strictDeps = true;
    LD_LIBRARY_PATH = lib.makeLibraryPath [
      fontconfig
      wayland
      libxkbcommon
      libGL
      libmtp
    ];
    nativeBuildInputs = [
      cargo
      rustc
      rust-analyzer
      rustfmt
      slint-lsp
      pkg-config
    ];
    buildInputs = [
      fontconfig
      wayland
      libxkbcommon
      libGL
      libmtp
    ];
  }
) { }
