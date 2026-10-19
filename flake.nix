{
  description = "Terminal viewer for spec documents";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { nixpkgs, rust-overlay, ... }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];
      forAllSystems =
        f:
        nixpkgs.lib.genAttrs systems (
          system:
          let
            pkgs = import nixpkgs {
              inherit system;
              overlays = [ rust-overlay.overlays.default ];
            };
            toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
          in
          f { inherit pkgs toolchain; }
        );
    in
    {
      packages = forAllSystems (
        { pkgs, toolchain }:
        let
          rustPlatform = pkgs.makeRustPlatform {
            cargo = toolchain;
            rustc = toolchain;
          };
        in
        rec {
          spec-viewer = rustPlatform.buildRustPackage {
            pname = "spec-viewer";
            version = (fromTOML (builtins.readFile ./crates/spec-viewer/Cargo.toml)).package.version;
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;
            cargoBuildFlags = [ "--package=spec-viewer" ];
            cargoTestFlags = [ "--workspace" ];
            nativeCheckInputs = [ pkgs.git ];
            meta.mainProgram = "spec-viewer";
          };
          default = spec-viewer;
        }
      );

      devShells = forAllSystems (
        { pkgs, toolchain }:
        {
          default = pkgs.mkShell { packages = [ toolchain ]; };
        }
      );
    };
}
