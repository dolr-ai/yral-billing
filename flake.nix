{
  description = "Rust Axum server development environment";
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };
  outputs = { self, nixpkgs, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
      in {
        devShells.default = pkgs.mkShell {
          buildInputs = [
            pkgs.rustup
            pkgs.rustfmt
            pkgs.clippy
            pkgs.pkg-config
            pkgs.openssl
            pkgs.sqlite
            pkgs.diesel-cli
            pkgs.flyctl
          ];
          shellHook = ''
            echo "Rust Axum development shell ready."
            export PKG_CONFIG_PATH="${pkgs.sqlite.dev}/lib/pkgconfig:$PKG_CONFIG_PATH"
            export LD_LIBRARY_PATH="${pkgs.sqlite}/lib:$LD_LIBRARY_PATH"
          '';
        };
      }
    );
}
