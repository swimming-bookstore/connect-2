{
  description = "Connect 2 — pinned toolchain";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    systems.url = "github:nix-systems/default";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane.url = "github:ipetkov/crane";
  };

  nixConfig = {
    extra-substituters = [
      "https://cache.nixos.org"
    ];
    extra-trusted-public-keys = [
      "cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY="
    ];
  };

  outputs =
    {
      self,
      nixpkgs,
      systems,
      rust-overlay,
      crane,
      ...
    }:
    let
      forAllSystems = nixpkgs.lib.genAttrs (import systems);
      pkgsFor =
        system:
        import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };
      packFor =
        system:
        import ./nix/package.nix {
          pkgs = pkgsFor system;
          craneLib = (crane.mkLib (pkgsFor system));
        };
    in
    {
      packages = forAllSystems (
        system:
        let
          pack = packFor system;
        in
        {
          connect2 = pack.client;
          connect2-agent = pack.agent;
          connect2-turn = pack.turn;
          default = pack.client;
        }
        // nixpkgs.lib.optionalAttrs (pack.tauri != null) {
          connect2-tauri = pack.tauri;
          connect2-desktop = pack.desktop;
        }
      );

      devShells = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          pack = packFor system;
        in
        {
          default = pkgs.mkShell {
            packages = [
              pack.rustToolchain
              pkgs.pkg-config
              pkgs.nodejs_22
              pkgs.git
            ]
            ++ pkgs.lib.optionals pkgs.stdenv.isLinux [
              pkgs.gtk3
              pkgs.glib
              pkgs.webkitgtk_4_1
              pkgs.libsoup_3
              pkgs.openssl
            ];
            CONNECT2_KEEP_RUSTC = "1";
            env.RUST_BACKTRACE = "1";
          };
        }
      );

      checks = forAllSystems (
        system:
        {
          connect2 = self.packages.${system}.connect2;
        }
        // nixpkgs.lib.optionalAttrs (self.packages.${system} ? connect2-tauri) {
          connect2-tauri = self.packages.${system}.connect2-tauri;
        }
      );
    };
}
