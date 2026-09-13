/*
  Impure entry: only `system` comes from the evaluator.
  Everything else is pinned in flake.lock via pins.nix.
*/
{
  system ? builtins.currentSystem,
}:
let
  pins = import ./pins.nix;
  overlay = import pins.rust-overlay;
  pkgs = import pins.nixpkgs {
    inherit system;
    overlays = [ overlay ];
  };
  craneLib = import "${pins.crane}/lib" { inherit pkgs; };
in
import ./nix/package.nix {
  inherit pkgs craneLib;
}
