{
  description = "Terminal viewer for spec documents";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      git-hooks,
      ...
    }:
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
          f { inherit system pkgs toolchain; }
        );
    in
    {
      homeManagerModules.default =
        {
          config,
          lib,
          pkgs,
          ...
        }:
        let
          cfg = config.programs.spec-viewer;
          tomlFormat = pkgs.formats.toml { };
        in
        {
          options.programs.spec-viewer = {
            enable = lib.mkEnableOption "spec-viewer";
            package = lib.mkOption {
              type = lib.types.package;
              default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
              description = "The spec-viewer package to install.";
            };
            settings = lib.mkOption {
              inherit (tomlFormat) type;
              default = { };
              description = "Written to {file}`$XDG_CONFIG_HOME/spec-viewer/config.toml`.";
            };
          };

          config = lib.mkIf cfg.enable {
            home.packages = [ cfg.package ];
            xdg.configFile."spec-viewer/config.toml" = lib.mkIf (cfg.settings != { }) {
              source = tomlFormat.generate "spec-viewer-config.toml" cfg.settings;
            };
          };
        };

      packages = forAllSystems (
        { pkgs, toolchain, ... }:
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
        {
          system,
          pkgs,
          toolchain,
        }:
        let
          hooks = git-hooks.lib.${system}.run {
            src = ./.;
            hooks = {
              convco.enable = true;
              rustfmt = {
                enable = true;
                packageOverrides = {
                  cargo = toolchain;
                  rustfmt = toolchain;
                };
              };
              clippy = {
                enable = true;
                packageOverrides = {
                  cargo = toolchain;
                  clippy = toolchain;
                };
                settings = {
                  denyWarnings = true;
                  extraArgs = "--workspace --all-targets";
                };
              };
            };
          };
        in
        {
          default = pkgs.mkShell {
            packages = [ toolchain ];
            inherit (hooks) shellHook;
          };
        }
      );
    };
}
