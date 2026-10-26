{
  description = "LG Hue Sync host checks and native build tools";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { nixpkgs, rust-overlay, ... }:
    let
      systems = [ "aarch64-darwin" "aarch64-linux" "x86_64-linux" ];
      forEachSystem = f: nixpkgs.lib.genAttrs systems (system:
        f (import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        }));
    in {
      devShells = forEachSystem (pkgs:
        let
          rust = pkgs.rust-bin.stable."1.99.1".minimal.override {
            extensions = [ "clippy" "rustfmt" ];
            targets = [ "armv7-unknown-linux-gnueabi" ];
          };
        in {
          default = pkgs.mkShell {
            name = "lg-hue-sync";
            packages = with pkgs; [
              rust cargo-nextest pkg-config uv python3 bun nodejs_24
              gnumake perl cmake git curl openssh gnutar gzip bzip2 unzip
              file binutils docker-client
            ];
            # Native scripts must not select a different rustup/Homebrew toolchain.
            WEBOS_RUSTC = "${rust}/bin/rustc";
            WEBOS_CARGO = "${rust}/bin/cargo";
          };
        });
    };
}
