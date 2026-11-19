{
  description = "Rhesus: the Monkey language in Rust";
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      # Pinned directly to the commit hash to bypass GitHub API 502 errors on HEAD
      url = "github:oxalica/rust-overlay/85d4adbbd952fcaf89fed3cb4ba540a4770d6e64";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };
  outputs = { self, nixpkgs, rust-overlay, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };

        # Reads rust-toolchain.toml, so the shell and the build use one compiler.
        toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

        rustPlatform = pkgs.makeRustPlatform {
          cargo = toolchain;
          rustc = toolchain;
        };
      in
      {
        packages.default = rustPlatform.buildRustPackage {
          pname = "rhesus";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = [ "-p" "rhesus-cli" ];
          cargoTestFlags = [ "--workspace" ];
        };

        devShells.default = pkgs.mkShell {
          packages = [ toolchain pkgs.bacon ];
          RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
        };
      });
}
